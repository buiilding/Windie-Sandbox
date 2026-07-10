//! Backend-owned runtime runs and reconnectable event delivery.
//!
//! HTTP clients create a run and subscribe to its ordered event journal. The
//! task is owned here instead of by one response body, so browser reloads only
//! disconnect a subscriber. Explicit cancellation remains process-local, while
//! persisted status and events let clients reconstruct completed work.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use tokio::task::AbortHandle;

use crate::conversation::ConversationId;
use crate::error;
use crate::store::{RuntimeRunEventRecord, RuntimeRunRecord, Store};

const RUN_EVENT_CHANNEL_CAPACITY: usize = 512;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
/// One event emitted by a backend-owned runtime action.
pub enum RunEvent {
    AssistantDelta {
        text: String,
    },
    ReasoningDelta {
        text: String,
    },
    ToolCallDelta {
        index: u16,
        id: Option<String>,
        name: Option<String>,
        arguments_delta: Option<String>,
    },
    AssistantMessageSaved {
        message_id: String,
    },
    ToolResultSaved {
        message_id: String,
    },
    QueryDone {
        message_id: Option<String>,
    },
    QueryError {
        error: String,
        causes: Vec<String>,
    },
    RunCancelled,
}

impl RunEvent {
    /// Returns the SSE event name matching the serialized event type.
    pub fn event_name(&self) -> &'static str {
        match self {
            Self::AssistantDelta { .. } => "assistant_delta",
            Self::ReasoningDelta { .. } => "reasoning_delta",
            Self::ToolCallDelta { .. } => "tool_call_delta",
            Self::AssistantMessageSaved { .. } => "assistant_message_saved",
            Self::ToolResultSaved { .. } => "tool_result_saved",
            Self::QueryDone { .. } => "query_done",
            Self::QueryError { .. } => "query_error",
            Self::RunCancelled => "run_cancelled",
        }
    }

    /// Returns whether no later events should be expected for this run.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::QueryDone { .. } | Self::QueryError { .. } | Self::RunCancelled
        )
    }
}

#[derive(Debug, Clone, Serialize)]
/// Ordered event envelope returned to reconnecting clients.
pub struct RunEventEnvelope {
    pub run_id: String,
    pub sequence: u64,
    #[serde(flatten)]
    pub event: RunEvent,
}

#[derive(Debug, Clone, Serialize)]
/// Public state for one backend-owned run.
pub struct RunSnapshot {
    pub id: String,
    pub conversation_id: String,
    pub status: String,
    pub error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl From<RuntimeRunRecord> for RunSnapshot {
    fn from(record: RuntimeRunRecord) -> Self {
        Self {
            id: record.id,
            conversation_id: record.conversation_id.as_str().to_string(),
            status: record.status,
            error: record.error,
            created_at: record.created_at,
            updated_at: record.updated_at,
        }
    }
}

/// Initial replay plus live receiver for one event subscription.
pub struct RunSubscription {
    pub history: Vec<RunEventEnvelope>,
    pub receiver: broadcast::Receiver<RunEventEnvelope>,
}

struct ActiveRun {
    sender: broadcast::Sender<RunEventEnvelope>,
    abort_handle: Option<AbortHandle>,
}

#[derive(Clone)]
/// Coordinates active tasks and the persisted run journal.
pub struct RunManager {
    store_path: Option<PathBuf>,
    active: Arc<Mutex<HashMap<String, ActiveRun>>>,
}

impl RunManager {
    /// Creates a manager and marks runs abandoned by an older process.
    pub fn new(store_path: Option<PathBuf>) -> Result<Self> {
        let manager = Self {
            store_path,
            active: Arc::new(Mutex::new(HashMap::new())),
        };
        manager.open_store()?.interrupt_running_runtime_runs()?;
        Ok(manager)
    }

    /// Creates persisted run state before its task is spawned.
    pub fn begin(&self, conversation_id: &ConversationId) -> Result<RunSnapshot> {
        if let Some(active) = self.open_store()?.active_runtime_run(conversation_id)? {
            return Err(anyhow!(
                "conversation already has a running action: {}",
                active.id
            ));
        }

        let record = self.open_store()?.create_runtime_run(conversation_id)?;
        let (sender, _) = broadcast::channel(RUN_EVENT_CHANNEL_CAPACITY);
        self.active
            .lock()
            .map_err(|_| anyhow!("runtime run manager lock was poisoned"))?
            .insert(
                record.id.clone(),
                ActiveRun {
                    sender,
                    abort_handle: None,
                },
            );

        Ok(record.into())
    }

    /// Attaches the spawned task's explicit cancellation handle.
    pub fn register_task(&self, run_id: &str, abort_handle: AbortHandle) -> Result<()> {
        if let Some(active) = self
            .active
            .lock()
            .map_err(|_| anyhow!("runtime run manager lock was poisoned"))?
            .get_mut(run_id)
        {
            active.abort_handle = Some(abort_handle);
        }
        Ok(())
    }

    /// Persists and broadcasts one ordered event.
    pub fn publish(&self, run_id: &str, event: RunEvent) -> Result<RunEventEnvelope> {
        let payload = serde_json::to_string(&event).context("failed to serialize runtime event")?;
        let sequence = self
            .open_store()?
            .append_runtime_run_event(run_id, &payload)?;
        let envelope = RunEventEnvelope {
            run_id: run_id.to_string(),
            sequence,
            event,
        };

        if let Some(sender) = self
            .active
            .lock()
            .map_err(|_| anyhow!("runtime run manager lock was poisoned"))?
            .get(run_id)
            .map(|active| active.sender.clone())
        {
            let _ = sender.send(envelope.clone());
        }

        Ok(envelope)
    }

    /// Completes a run after persisting its terminal event.
    pub fn complete(&self, run_id: &str, message_id: Option<String>) -> Result<()> {
        self.publish(run_id, RunEvent::QueryDone { message_id })?;
        self.open_store()?
            .set_runtime_run_status(run_id, "completed", None)?;
        self.remove_active(run_id)
    }

    /// Fails a run after preserving the full client-facing error chain.
    pub fn fail(&self, run_id: &str, error: String, causes: Vec<String>) -> Result<()> {
        self.publish(
            run_id,
            RunEvent::QueryError {
                error: error.clone(),
                causes,
            },
        )?;
        self.open_store()?
            .set_runtime_run_status(run_id, "failed", Some(&error))?;
        self.remove_active(run_id)
    }

    /// Explicitly aborts active work and records cancellation.
    pub fn cancel(&self, run_id: &str) -> Result<RunSnapshot> {
        let snapshot = self.snapshot(run_id)?;
        if snapshot.status != "running" {
            return Err(error::invalid_request(format!(
                "runtime run is not running: {run_id} ({})",
                snapshot.status
            )));
        }
        let abort_handle = self
            .active
            .lock()
            .map_err(|_| anyhow!("runtime run manager lock was poisoned"))?
            .get(run_id)
            .and_then(|active| active.abort_handle.clone());
        if let Some(abort_handle) = abort_handle {
            abort_handle.abort();
        }
        self.publish(run_id, RunEvent::RunCancelled)?;
        self.open_store()?
            .set_runtime_run_status(run_id, "cancelled", None)?;
        self.remove_active(run_id)?;
        self.snapshot(run_id)
    }

    /// Loads current persisted run state.
    pub fn snapshot(&self, run_id: &str) -> Result<RunSnapshot> {
        Ok(self.open_store()?.runtime_run(run_id)?.into())
    }

    /// Loads the active run for a conversation, including after UI reload.
    pub fn active_for_conversation(
        &self,
        conversation_id: &ConversationId,
    ) -> Result<Option<RunSnapshot>> {
        Ok(self
            .open_store()?
            .active_runtime_run(conversation_id)?
            .map(Into::into))
    }

    /// Subscribes before loading replay history, preventing a creation gap.
    pub fn subscribe(&self, run_id: &str, after: u64) -> Result<RunSubscription> {
        let receiver = if let Some(sender) = self
            .active
            .lock()
            .map_err(|_| anyhow!("runtime run manager lock was poisoned"))?
            .get(run_id)
            .map(|active| active.sender.clone())
        {
            sender.subscribe()
        } else {
            let (sender, receiver) = broadcast::channel(1);
            drop(sender);
            receiver
        };
        let history = self.events_after(run_id, after)?;

        Ok(RunSubscription { history, receiver })
    }

    /// Reloads persisted events after a lagged broadcast receiver.
    pub fn events_after(&self, run_id: &str, after: u64) -> Result<Vec<RunEventEnvelope>> {
        self.open_store()?
            .runtime_run_events_after(run_id, after)?
            .into_iter()
            .map(|record| decode_event_record(run_id, record))
            .collect()
    }

    fn remove_active(&self, run_id: &str) -> Result<()> {
        self.active
            .lock()
            .map_err(|_| anyhow!("runtime run manager lock was poisoned"))?
            .remove(run_id);
        Ok(())
    }

    fn open_store(&self) -> Result<Store> {
        match self.store_path.as_ref() {
            Some(path) => Store::open_at(path),
            None => Store::open(),
        }
    }
}

fn decode_event_record(run_id: &str, record: RuntimeRunEventRecord) -> Result<RunEventEnvelope> {
    Ok(RunEventEnvelope {
        run_id: run_id.to_string(),
        sequence: record.sequence,
        event: serde_json::from_str(&record.payload).context("failed to decode runtime event")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_path(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("windie-run-{name}-{nonce}.db"))
    }

    #[test]
    fn persists_and_replays_ordered_events() {
        let path = test_path("replay");
        let store = Store::open_at(&path).unwrap();
        let conversation_id = store.create_conversation("openai/test").unwrap();
        drop(store);
        let manager = RunManager::new(Some(path.clone())).unwrap();
        let run = manager.begin(&conversation_id).unwrap();

        manager
            .publish(
                &run.id,
                RunEvent::AssistantDelta {
                    text: "hello".to_string(),
                },
            )
            .unwrap();
        manager
            .complete(&run.id, Some("message-1".to_string()))
            .unwrap();

        let replay = manager.events_after(&run.id, 0).unwrap();
        assert_eq!(replay.len(), 2);
        assert_eq!(replay[0].sequence, 1);
        assert!(matches!(replay[0].event, RunEvent::AssistantDelta { .. }));
        assert_eq!(replay[1].sequence, 2);
        assert!(matches!(replay[1].event, RunEvent::QueryDone { .. }));
        assert_eq!(manager.snapshot(&run.id).unwrap().status, "completed");

        let _ = fs::remove_file(path);
    }

    #[test]
    fn new_manager_marks_abandoned_runs_interrupted() {
        let path = test_path("interrupted");
        let store = Store::open_at(&path).unwrap();
        let conversation_id = store.create_conversation("openai/test").unwrap();
        drop(store);
        let first = RunManager::new(Some(path.clone())).unwrap();
        let run = first.begin(&conversation_id).unwrap();

        let restarted = RunManager::new(Some(path.clone())).unwrap();
        assert_eq!(restarted.snapshot(&run.id).unwrap().status, "interrupted");

        let _ = fs::remove_file(path);
    }
}
