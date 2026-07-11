//! Backend-owned runtime runs and reconnectable event delivery.
//!
//! HTTP clients create a run and subscribe to its ordered event journal. The
//! task is owned here instead of by one response body, so browser reloads only
//! disconnect a subscriber. Explicit cancellation remains process-local, while
//! persisted status and events let clients reconstruct completed work.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use tokio::sync::{Notify, broadcast, mpsc, oneshot};
use uuid::Uuid;

use crate::conversation::ConversationId;
use crate::error;
use crate::store::{
    RuntimeRunAction, RuntimeRunEventRecord, RuntimeRunRecord, RuntimeRunStatus, Store,
};

const RUN_EVENT_CHANNEL_CAPACITY: usize = 512;
const RUN_JOURNAL_COMMAND_CAPACITY: usize = 512;
const RUN_LEASE_DURATION: Duration = Duration::from_secs(30);
const RUN_LEASE_RENEW_INTERVAL: Duration = Duration::from_secs(10);

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
    pub action: RuntimeRunAction,
    pub status: RuntimeRunStatus,
    pub error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl From<RuntimeRunRecord> for RunSnapshot {
    fn from(record: RuntimeRunRecord) -> Self {
        Self {
            id: record.id,
            conversation_id: record.conversation_id.as_str().to_string(),
            action: record.action,
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

pub struct PendingRunEvent {
    response: oneshot::Receiver<Result<RunEventEnvelope>>,
}

impl PendingRunEvent {
    pub async fn persisted(self) -> Result<RunEventEnvelope> {
        self.response
            .await
            .map_err(|_| anyhow!("runtime run journal stopped before persisting event"))?
    }
}

struct ActiveRun {
    sender: broadcast::Sender<RunEventEnvelope>,
    cancellation: RunCancellation,
    completion: Arc<Notify>,
}

#[derive(Clone, Default)]
pub struct RunCancellation {
    cancelled: Arc<AtomicBool>,
    notify: Arc<Notify>,
}

impl RunCancellation {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        self.notify.notify_waiters();
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    pub fn check(&self) -> Result<()> {
        if self.is_cancelled() {
            Err(RuntimeCancelled.into())
        } else {
            Ok(())
        }
    }

    pub async fn cancelled(&self) {
        loop {
            let notified = self.notify.notified();
            if self.is_cancelled() {
                return;
            }
            notified.await;
        }
    }
}

#[derive(Debug)]
pub struct RuntimeCancelled;

impl std::fmt::Display for RuntimeCancelled {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("runtime run was cancelled")
    }
}

impl std::error::Error for RuntimeCancelled {}

pub fn is_runtime_cancelled(error: &anyhow::Error) -> bool {
    error.downcast_ref::<RuntimeCancelled>().is_some()
}

enum RunJournalCommand {
    Create {
        conversation_id: ConversationId,
        action: RuntimeRunAction,
        response: oneshot::Sender<Result<RuntimeRunRecord>>,
    },
    Append {
        run_id: String,
        payload: String,
        event: RunEvent,
        broadcast: broadcast::Sender<RunEventEnvelope>,
        response: oneshot::Sender<Result<RunEventEnvelope>>,
    },
    Finish {
        run_id: String,
        status: RuntimeRunStatus,
        error: Option<String>,
        payload: String,
        event: RunEvent,
        broadcast: broadcast::Sender<RunEventEnvelope>,
        response: oneshot::Sender<Result<Option<RunEventEnvelope>>>,
    },
    Snapshot {
        run_id: String,
        response: oneshot::Sender<Result<RuntimeRunRecord>>,
    },
    ActiveForConversation {
        conversation_id: ConversationId,
        response: oneshot::Sender<Result<Option<RuntimeRunRecord>>>,
    },
    EventsAfter {
        run_id: String,
        after: u64,
        response: oneshot::Sender<Result<Vec<RuntimeRunEventRecord>>>,
    },
}

#[derive(Clone)]
struct RunJournal {
    commands: mpsc::Sender<RunJournalCommand>,
}

impl RunJournal {
    fn start(store_path: Option<PathBuf>) -> Result<Self> {
        let store = match store_path.as_ref() {
            Some(path) => Store::open_at(path)?,
            None => Store::open()?,
        };
        store.interrupt_expired_runtime_runs(unix_millis()?)?;
        let owner_id = Uuid::new_v4().to_string();

        let (commands, receiver) = mpsc::channel(RUN_JOURNAL_COMMAND_CAPACITY);
        std::thread::Builder::new()
            .name("windie-run-journal".to_string())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_time()
                    .build()
                    .expect("failed to build run journal runtime");
                runtime.block_on(run_journal_worker(store, receiver, owner_id));
            })
            .context("failed to start runtime run journal")?;

        Ok(Self { commands })
    }

    async fn request<T>(
        &self,
        command: impl FnOnce(oneshot::Sender<Result<T>>) -> RunJournalCommand,
    ) -> Result<T> {
        let (response, receiver) = oneshot::channel();
        self.commands
            .send(command(response))
            .await
            .map_err(|_| anyhow!("runtime run journal stopped"))?;
        receiver
            .await
            .map_err(|_| anyhow!("runtime run journal stopped before replying"))?
    }

    async fn create(
        &self,
        conversation_id: &ConversationId,
        action: RuntimeRunAction,
    ) -> Result<RuntimeRunRecord> {
        self.request(|response| RunJournalCommand::Create {
            conversation_id: conversation_id.clone(),
            action,
            response,
        })
        .await
    }

    fn enqueue(
        &self,
        run_id: &str,
        event: RunEvent,
        broadcast: broadcast::Sender<RunEventEnvelope>,
    ) -> Result<PendingRunEvent> {
        let payload = serde_json::to_string(&event).context("failed to serialize runtime event")?;
        let (response, receiver) = oneshot::channel();
        self.commands
            .try_send(RunJournalCommand::Append {
                run_id: run_id.to_string(),
                payload,
                event,
                broadcast,
                response,
            })
            .map_err(|error| anyhow!("runtime run journal queue rejected event: {error}"))?;
        Ok(PendingRunEvent { response: receiver })
    }

    async fn finish(
        &self,
        run_id: &str,
        status: RuntimeRunStatus,
        error: Option<&str>,
        event: RunEvent,
        broadcast: broadcast::Sender<RunEventEnvelope>,
    ) -> Result<Option<RunEventEnvelope>> {
        let payload = serde_json::to_string(&event).context("failed to serialize runtime event")?;
        self.request(|response| RunJournalCommand::Finish {
            run_id: run_id.to_string(),
            status,
            error: error.map(str::to_string),
            payload,
            event,
            broadcast,
            response,
        })
        .await
    }

    async fn snapshot(&self, run_id: &str) -> Result<RuntimeRunRecord> {
        self.request(|response| RunJournalCommand::Snapshot {
            run_id: run_id.to_string(),
            response,
        })
        .await
    }

    async fn active_for_conversation(
        &self,
        conversation_id: &ConversationId,
    ) -> Result<Option<RuntimeRunRecord>> {
        self.request(|response| RunJournalCommand::ActiveForConversation {
            conversation_id: conversation_id.clone(),
            response,
        })
        .await
    }

    async fn events_after(&self, run_id: &str, after: u64) -> Result<Vec<RuntimeRunEventRecord>> {
        self.request(|response| RunJournalCommand::EventsAfter {
            run_id: run_id.to_string(),
            after,
            response,
        })
        .await
    }
}

async fn run_journal_worker(
    mut store: Store,
    mut receiver: mpsc::Receiver<RunJournalCommand>,
    owner_id: String,
) {
    let renewal = tokio::time::sleep(RUN_LEASE_RENEW_INTERVAL);
    tokio::pin!(renewal);
    loop {
        let command = tokio::select! {
            command = receiver.recv() => match command {
                Some(command) => command,
                None => break,
            },
            () = &mut renewal => {
                let _ = store.renew_runtime_run_leases(&owner_id, lease_deadline_millis());
                renewal.as_mut().reset(tokio::time::Instant::now() + RUN_LEASE_RENEW_INTERVAL);
                continue;
            }
        };
        match command {
            RunJournalCommand::Create {
                conversation_id,
                action,
                response,
            } => {
                let result = store
                    .interrupt_expired_runtime_runs(unix_millis().unwrap_or(i64::MAX))
                    .and_then(|_| {
                        store.create_owned_runtime_run(
                            &conversation_id,
                            action,
                            &owner_id,
                            lease_deadline_millis(),
                        )
                    });
                let _ = response.send(result);
            }
            RunJournalCommand::Append {
                run_id,
                payload,
                event,
                broadcast,
                response,
            } => {
                let result = store
                    .append_runtime_run_event(&run_id, &payload)
                    .map(|sequence| {
                        let envelope = RunEventEnvelope {
                            run_id,
                            sequence,
                            event,
                        };
                        let _ = broadcast.send(envelope.clone());
                        envelope
                    });
                let _ = response.send(result);
            }
            RunJournalCommand::Finish {
                run_id,
                status,
                error,
                payload,
                event,
                broadcast,
                response,
            } => {
                let result = store
                    .finish_runtime_run(&run_id, status, error.as_deref(), &payload)
                    .map(|sequence| {
                        sequence.map(|sequence| {
                            let envelope = RunEventEnvelope {
                                run_id,
                                sequence,
                                event,
                            };
                            let _ = broadcast.send(envelope.clone());
                            envelope
                        })
                    });
                let _ = response.send(result);
            }
            RunJournalCommand::Snapshot { run_id, response } => {
                let _ = response.send(store.runtime_run(&run_id));
            }
            RunJournalCommand::ActiveForConversation {
                conversation_id,
                response,
            } => {
                let _ = response.send(store.active_runtime_run(&conversation_id));
            }
            RunJournalCommand::EventsAfter {
                run_id,
                after,
                response,
            } => {
                let _ = response.send(store.runtime_run_events_after(&run_id, after));
            }
        }
    }
    let _ = store.interrupt_runtime_runs_for_owner(&owner_id);
}

fn unix_millis() -> Result<i64> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before unix epoch")?;
    Ok(duration.as_millis().min(i64::MAX as u128) as i64)
}

fn lease_deadline_millis() -> i64 {
    unix_millis()
        .unwrap_or(i64::MAX - RUN_LEASE_DURATION.as_millis() as i64)
        .saturating_add(RUN_LEASE_DURATION.as_millis() as i64)
}

#[derive(Clone)]
/// Coordinates active tasks and the persisted run journal.
pub struct RunManager {
    journal: RunJournal,
    active: Arc<Mutex<HashMap<String, ActiveRun>>>,
}

impl RunManager {
    /// Creates a manager and marks runs abandoned by an older process.
    pub fn new(store_path: Option<PathBuf>) -> Result<Self> {
        Ok(Self {
            journal: RunJournal::start(store_path)?,
            active: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    /// Creates persisted run state before its task is spawned.
    pub async fn begin(&self, conversation_id: &ConversationId) -> Result<RunSnapshot> {
        self.begin_action(conversation_id, RuntimeRunAction::Query)
            .await
    }

    /// Creates persisted ownership for one concrete runtime action.
    pub async fn begin_action(
        &self,
        conversation_id: &ConversationId,
        action: RuntimeRunAction,
    ) -> Result<RunSnapshot> {
        let record = self.journal.create(conversation_id, action).await?;
        let (sender, _) = broadcast::channel(RUN_EVENT_CHANNEL_CAPACITY);
        self.active
            .lock()
            .map_err(|_| anyhow!("runtime run manager lock was poisoned"))?
            .insert(
                record.id.clone(),
                ActiveRun {
                    sender,
                    cancellation: RunCancellation::default(),
                    completion: Arc::new(Notify::new()),
                },
            );

        Ok(record.into())
    }

    /// Finalizes one direct operation without changing its client response shape.
    pub async fn finish_result<T>(&self, run_id: &str, result: Result<T>) -> Result<T> {
        match result {
            Ok(value) => {
                self.complete(run_id, None).await?;
                Ok(value)
            }
            Err(operation_error) => {
                let message = operation_error.to_string();
                let causes = operation_error.chain().map(ToString::to_string).collect();
                self.fail(run_id, message, causes).await?;
                Err(operation_error)
            }
        }
    }

    pub fn cancellation(&self, run_id: &str) -> Result<RunCancellation> {
        self.active
            .lock()
            .map_err(|_| anyhow!("runtime run manager lock was poisoned"))?
            .get_mut(run_id)
            .map(|active| active.cancellation.clone())
            .ok_or_else(|| error::not_found(format!("active runtime run does not exist: {run_id}")))
    }

    pub fn enqueue(&self, run_id: &str, event: RunEvent) -> Result<PendingRunEvent> {
        let sender = self
            .active
            .lock()
            .map_err(|_| anyhow!("runtime run manager lock was poisoned"))?
            .get(run_id)
            .map(|active| active.sender.clone())
            .ok_or_else(|| {
                error::invalid_request(format!("runtime run is not running: {run_id}"))
            })?;
        self.journal.enqueue(run_id, event, sender)
    }

    /// Persists and broadcasts one ordered event.
    #[cfg(test)]
    pub async fn publish(&self, run_id: &str, event: RunEvent) -> Result<RunEventEnvelope> {
        self.enqueue(run_id, event)?.persisted().await
    }

    /// Completes a run after persisting its terminal event.
    pub async fn complete(&self, run_id: &str, message_id: Option<String>) -> Result<()> {
        self.finish(
            run_id,
            RunEvent::QueryDone { message_id },
            RuntimeRunStatus::Completed,
            None,
        )
        .await?;
        Ok(())
    }

    /// Fails a run after preserving the full client-facing error chain.
    pub async fn fail(&self, run_id: &str, error: String, causes: Vec<String>) -> Result<()> {
        self.finish(
            run_id,
            RunEvent::QueryError {
                error: error.clone(),
                causes,
            },
            RuntimeRunStatus::Failed,
            Some(&error),
        )
        .await?;
        Ok(())
    }

    /// Requests cooperative cancellation and waits for the task to stop.
    pub async fn cancel(&self, run_id: &str) -> Result<RunSnapshot> {
        let (cancellation, completion) = self
            .active
            .lock()
            .map_err(|_| anyhow!("runtime run manager lock was poisoned"))?
            .get(run_id)
            .map(|active| (active.cancellation.clone(), Arc::clone(&active.completion)))
            .ok_or_else(|| {
                error::invalid_request(format!("runtime run is not running: {run_id}"))
            })?;
        cancellation.cancel();
        loop {
            let snapshot = self.snapshot(run_id).await?;
            if snapshot.status != RuntimeRunStatus::Running {
                return Ok(snapshot);
            }
            tokio::select! {
                () = completion.notified() => {}
                () = tokio::time::sleep(Duration::from_millis(50)) => {}
            }
        }
    }

    pub async fn finish_cancelled(&self, run_id: &str) -> Result<()> {
        self.finish(
            run_id,
            RunEvent::RunCancelled,
            RuntimeRunStatus::Cancelled,
            None,
        )
        .await?;
        Ok(())
    }

    /// Loads current persisted run state.
    pub async fn snapshot(&self, run_id: &str) -> Result<RunSnapshot> {
        Ok(self.journal.snapshot(run_id).await?.into())
    }

    /// Loads the active run for a conversation, including after UI reload.
    pub async fn active_for_conversation(
        &self,
        conversation_id: &ConversationId,
    ) -> Result<Option<RunSnapshot>> {
        Ok(self
            .journal
            .active_for_conversation(conversation_id)
            .await?
            .map(Into::into))
    }

    /// Subscribes before loading replay history, preventing a creation gap.
    pub async fn subscribe(&self, run_id: &str, after: u64) -> Result<RunSubscription> {
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
        let history = self.events_after(run_id, after).await?;

        Ok(RunSubscription { history, receiver })
    }

    /// Reloads persisted events after a lagged broadcast receiver.
    pub async fn events_after(&self, run_id: &str, after: u64) -> Result<Vec<RunEventEnvelope>> {
        self.journal
            .events_after(run_id, after)
            .await?
            .into_iter()
            .map(|record| decode_event_record(run_id, record))
            .collect()
    }

    fn remove_active(&self, run_id: &str) -> Result<()> {
        let removed = self
            .active
            .lock()
            .map_err(|_| anyhow!("runtime run manager lock was poisoned"))?
            .remove(run_id);
        if let Some(active) = removed {
            active.completion.notify_waiters();
        }
        Ok(())
    }

    async fn finish(
        &self,
        run_id: &str,
        event: RunEvent,
        status: RuntimeRunStatus,
        error_message: Option<&str>,
    ) -> Result<bool> {
        let sender = self
            .active
            .lock()
            .map_err(|_| anyhow!("runtime run manager lock was poisoned"))?
            .get(run_id)
            .map(|active| active.sender.clone())
            .ok_or_else(|| {
                error::not_found(format!("active runtime run does not exist: {run_id}"))
            })?;
        let Some(_envelope) = self
            .journal
            .finish(run_id, status, error_message, event, sender)
            .await?
        else {
            return Ok(false);
        };
        self.remove_active(run_id)?;
        Ok(true)
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

    #[tokio::test]
    async fn persists_and_replays_ordered_events() {
        let path = test_path("replay");
        let store = Store::open_at(&path).unwrap();
        let conversation_id = store.create_conversation("openai/test").unwrap();
        drop(store);
        let manager = RunManager::new(Some(path.clone())).unwrap();
        let run = manager.begin(&conversation_id).await.unwrap();

        manager
            .publish(
                &run.id,
                RunEvent::AssistantDelta {
                    text: "hello".to_string(),
                },
            )
            .await
            .unwrap();
        manager
            .complete(&run.id, Some("message-1".to_string()))
            .await
            .unwrap();

        let replay = manager.events_after(&run.id, 0).await.unwrap();
        assert_eq!(replay.len(), 2);
        assert_eq!(replay[0].sequence, 1);
        assert!(matches!(replay[0].event, RunEvent::AssistantDelta { .. }));
        assert_eq!(replay[1].sequence, 2);
        assert!(matches!(replay[1].event, RunEvent::QueryDone { .. }));
        assert_eq!(
            manager.snapshot(&run.id).await.unwrap().status,
            RuntimeRunStatus::Completed
        );

        let _ = fs::remove_file(path);
    }

    #[tokio::test]
    async fn new_manager_does_not_interrupt_another_live_owner() {
        let path = test_path("live-owner");
        let store = Store::open_at(&path).unwrap();
        let conversation_id = store.create_conversation("openai/test").unwrap();
        drop(store);
        let first = RunManager::new(Some(path.clone())).unwrap();
        let run = first.begin(&conversation_id).await.unwrap();

        let restarted = RunManager::new(Some(path.clone())).unwrap();
        assert_eq!(
            restarted.snapshot(&run.id).await.unwrap().status,
            RuntimeRunStatus::Running
        );

        let _ = fs::remove_file(path);
    }

    #[tokio::test]
    async fn new_manager_replaces_an_expired_owner() {
        let path = test_path("expired-owner");
        let store = Store::open_at(&path).unwrap();
        let conversation_id = store.create_conversation("openai/test").unwrap();
        let expired = store
            .create_owned_runtime_run(
                &conversation_id,
                RuntimeRunAction::Query,
                "expired-owner",
                0,
            )
            .unwrap();
        drop(store);

        let manager = RunManager::new(Some(path.clone())).unwrap();
        let replacement = manager.begin(&conversation_id).await.unwrap();

        assert_eq!(
            manager.snapshot(&expired.id).await.unwrap().status,
            RuntimeRunStatus::Interrupted
        );
        assert_eq!(replacement.status, RuntimeRunStatus::Running);
        let _ = fs::remove_file(path);
    }

    #[tokio::test]
    async fn concurrent_starts_create_only_one_running_run() {
        let path = test_path("concurrent-start");
        let store = Store::open_at(&path).unwrap();
        let conversation_id = store.create_conversation("openai/test").unwrap();
        drop(store);
        let manager = RunManager::new(Some(path.clone())).unwrap();
        let (first, second) = tokio::join!(
            manager.begin(&conversation_id),
            manager.begin(&conversation_id)
        );
        let results = [first, second];

        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(results.iter().filter(|result| result.is_err()).count(), 1);
        let _ = fs::remove_file(path);
    }

    #[tokio::test]
    async fn competing_terminal_transitions_persist_one_terminal_event() {
        let path = test_path("terminal-race");
        let store = Store::open_at(&path).unwrap();
        let conversation_id = store.create_conversation("openai/test").unwrap();
        drop(store);
        let manager = RunManager::new(Some(path.clone())).unwrap();
        let run = manager.begin(&conversation_id).await.unwrap();
        let _ = tokio::join!(
            manager.complete(&run.id, None),
            manager.finish_cancelled(&run.id)
        );

        let events = manager.events_after(&run.id, 0).await.unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|event| event.event.is_terminal())
                .count(),
            1
        );
        assert_ne!(
            manager.snapshot(&run.id).await.unwrap().status,
            RuntimeRunStatus::Running
        );
        let _ = fs::remove_file(path);
    }

    #[tokio::test]
    async fn fail_and_cancel_compete_for_one_terminal_transition() {
        let path = test_path("fail-cancel-race");
        let store = Store::open_at(&path).unwrap();
        let conversation_id = store.create_conversation("openai/test").unwrap();
        drop(store);
        let manager = RunManager::new(Some(path.clone())).unwrap();
        let run = manager.begin(&conversation_id).await.unwrap();
        let _ = tokio::join!(
            manager.fail(&run.id, "failed".to_string(), Vec::new()),
            manager.finish_cancelled(&run.id)
        );

        let events = manager.events_after(&run.id, 0).await.unwrap();
        assert_eq!(events.len(), 1);
        assert!(events[0].event.is_terminal());
        assert_ne!(
            manager.snapshot(&run.id).await.unwrap().status,
            RuntimeRunStatus::Running
        );
        let _ = fs::remove_file(path);
    }

    #[tokio::test]
    async fn rejects_late_events_after_terminal_event() {
        let path = test_path("late-event");
        let store = Store::open_at(&path).unwrap();
        let conversation_id = store.create_conversation("openai/test").unwrap();
        drop(store);
        let manager = RunManager::new(Some(path.clone())).unwrap();
        let run = manager.begin(&conversation_id).await.unwrap();
        manager.complete(&run.id, None).await.unwrap();

        let error = manager
            .publish(
                &run.id,
                RunEvent::AssistantDelta {
                    text: "late".to_string(),
                },
            )
            .await
            .unwrap_err();

        assert!(error.to_string().contains("is not running"));
        let events = manager.events_after(&run.id, 0).await.unwrap();
        assert_eq!(events.len(), 1);
        assert!(events[0].event.is_terminal());
        let _ = fs::remove_file(path);
    }

    #[tokio::test]
    async fn cancellation_waits_for_task_acknowledgement() {
        let path = test_path("cooperative-cancel");
        let store = Store::open_at(&path).unwrap();
        let conversation_id = store.create_conversation("openai/test").unwrap();
        drop(store);
        let manager = RunManager::new(Some(path.clone())).unwrap();
        let run = manager.begin(&conversation_id).await.unwrap();
        let cancellation = manager.cancellation(&run.id).unwrap();
        let cancel_manager = manager.clone();
        let cancel_id = run.id.clone();
        let cancel = tokio::spawn(async move { cancel_manager.cancel(&cancel_id).await });

        tokio::time::timeout(Duration::from_secs(1), async {
            while !cancellation.is_cancelled() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(!cancel.is_finished());

        manager.finish_cancelled(&run.id).await.unwrap();
        let snapshot = cancel.await.unwrap().unwrap();
        assert_eq!(snapshot.status, RuntimeRunStatus::Cancelled);
        let _ = fs::remove_file(path);
    }

    #[tokio::test]
    async fn bounded_journal_reports_backpressure() {
        let path = test_path("backpressure");
        let store = Store::open_at(&path).unwrap();
        let conversation_id = store.create_conversation("openai/test").unwrap();
        drop(store);
        let manager = RunManager::new(Some(path.clone())).unwrap();
        let run = manager.begin(&conversation_id).await.unwrap();
        let mut pending = Vec::new();
        let mut rejected = None;

        for _ in 0..10_000 {
            match manager.enqueue(
                &run.id,
                RunEvent::AssistantDelta {
                    text: "x".to_string(),
                },
            ) {
                Ok(write) => pending.push(write),
                Err(error) => {
                    rejected = Some(error);
                    break;
                }
            }
        }

        assert!(
            rejected
                .unwrap()
                .to_string()
                .contains("queue rejected event")
        );
        drop(pending);
        drop(manager);
        let _ = fs::remove_file(path);
    }
}
