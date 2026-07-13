//! Runtime run lifecycle.
//!
//! A run is one backend-owned execution lifecycle. Browser clients can create a
//! run, subscribe to its event log, disconnect, reconnect, or explicitly stop
//! it. The frontend observes and commands runs; it does not keep them alive.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::conversation::{ConversationId, MessageId, ToolCallId};
use crate::gateway::GatewayUrl;
use crate::llm::{BaseUrl, ModelName, ReasoningRequest};
use crate::operation::{self, RunRuntime};
use crate::output::RuntimeOutput;
use crate::runtime::RuntimeEventSink;
use crate::store::Store;
use crate::tool_provider::ToolProviderRegistry;

const RUN_EVENT_CHANNEL_CAPACITY: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
/// Stable identifier for one backend-owned runtime run.
pub struct RunId(String);

impl RunId {
    /// Creates a fresh run ID.
    pub fn fresh() -> Self {
        Self(Uuid::new_v4().to_string())
    }

    /// Wraps raw ID text from API or storage.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Exposes the ID at persistence and protocol boundaries.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for RunId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Durable lifecycle state for one run.
pub enum RunStatus {
    Running,
    WaitingForApproval,
    Completed,
    Failed,
    Cancelled,
}

impl RunStatus {
    /// Converts storage text into the typed status.
    pub fn from_storage(value: &str) -> Option<Self> {
        match value {
            "running" => Some(Self::Running),
            "waiting_for_approval" => Some(Self::WaitingForApproval),
            "completed" => Some(Self::Completed),
            "failed" => Some(Self::Failed),
            "cancelled" => Some(Self::Cancelled),
            _ => None,
        }
    }

    /// Returns the stable storage representation.
    pub fn as_storage(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::WaitingForApproval => "waiting_for_approval",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

impl std::fmt::Display for RunStatus {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_storage())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Stored metadata for one runtime run.
pub struct Run {
    pub id: RunId,
    pub conversation_id: ConversationId,
    pub start_head_message_id: Option<MessageId>,
    pub current_head_message_id: Option<MessageId>,
    pub status: RunStatus,
    pub model: String,
    pub reasoning: Option<ReasoningRequest>,
    pub error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
/// Durable event emitted by one run.
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
    WaitingForApproval,
    Completed {
        message_id: Option<String>,
    },
    Failed {
        error: String,
        causes: Vec<String>,
    },
    Cancelled,
}

impl RunEvent {
    /// Returns the SSE event name matching the JSON `type`.
    pub fn event_name(&self) -> &'static str {
        match self {
            Self::AssistantDelta { .. } => "assistant_delta",
            Self::ReasoningDelta { .. } => "reasoning_delta",
            Self::ToolCallDelta { .. } => "tool_call_delta",
            Self::AssistantMessageSaved { .. } => "assistant_message_saved",
            Self::ToolResultSaved { .. } => "tool_result_saved",
            Self::WaitingForApproval => "waiting_for_approval",
            Self::Completed { .. } => "completed",
            Self::Failed { .. } => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// One persisted event record with a monotonic run-local cursor.
pub struct RunEventRecord {
    pub id: i64,
    pub run_id: RunId,
    pub event: RunEvent,
    pub created_at: i64,
}

/// Live subscription to events from one run.
pub struct RunSubscription {
    receiver: broadcast::Receiver<RunEventRecord>,
}

impl RunSubscription {
    /// Waits for the next live event from the subscribed run.
    pub async fn recv(&mut self) -> Result<RunEventRecord> {
        loop {
            match self.receiver.recv().await {
                Ok(event) => return Ok(event),
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => {
                    return Err(anyhow::anyhow!("run event stream closed"));
                }
            }
        }
    }
}

/// Backend-owned runtime run supervisor.
#[derive(Clone)]
pub struct RunManager {
    store_path: Option<PathBuf>,
    gateway_url: String,
    base_url: String,
    tools: Arc<ToolProviderRegistry>,
    active: Arc<Mutex<HashMap<String, RunningRun>>>,
}

struct RunningRun {
    sender: broadcast::Sender<RunEventRecord>,
    task: JoinHandle<()>,
}

impl RunManager {
    /// Builds a run manager for the API server runtime.
    pub fn new(
        store_path: Option<PathBuf>,
        gateway_url: String,
        base_url: String,
        tools: Arc<ToolProviderRegistry>,
    ) -> Self {
        Self {
            store_path,
            gateway_url,
            base_url,
            tools,
            active: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Starts a backend-owned run from an explicit message head.
    pub fn start(
        &self,
        conversation_id: ConversationId,
        head_message_id: Option<MessageId>,
        model: String,
        reasoning: Option<ReasoningRequest>,
    ) -> Result<Run> {
        let mut store = self.open_store()?;
        let run_id = RunId::fresh();
        let run = store.create_run(
            &run_id,
            &conversation_id,
            head_message_id.as_ref(),
            &model,
            reasoning.as_ref(),
        )?;
        drop(store);

        self.spawn(
            run_id,
            conversation_id,
            head_message_id,
            Some(ModelName::new(model)),
            reasoning,
            RunCommand::Continue,
        );

        Ok(run)
    }

    /// Stops one live run explicitly.
    pub fn stop(&self, run_id: &RunId) -> Result<()> {
        let running = self
            .active
            .lock()
            .expect("run manager lock poisoned")
            .remove(run_id.as_str());

        if let Some(running) = &running {
            running.task.abort();
        }

        let mut store = self.open_store()?;
        store.update_run_status(run_id, RunStatus::Cancelled, None)?;
        let record = store.append_run_event(run_id, RunEvent::Cancelled)?;
        if let Some(running) = running {
            let _ = running.sender.send(record);
        }

        Ok(())
    }

    /// Subscribes to future live events from a run.
    pub fn subscribe(&self, run_id: &RunId) -> Option<RunSubscription> {
        self.active
            .lock()
            .expect("run manager lock poisoned")
            .get(run_id.as_str())
            .map(|running| RunSubscription {
                receiver: running.sender.subscribe(),
            })
    }

    /// Resumes a waiting run after a policy change.
    pub fn resume(&self, run_id: &RunId) -> Result<()> {
        let store = self.open_store()?;
        let run = store.load_run(run_id)?;
        drop(store);

        if self
            .active
            .lock()
            .expect("run manager lock poisoned")
            .contains_key(run_id.as_str())
        {
            return Ok(());
        }

        if run.status != RunStatus::WaitingForApproval {
            return Ok(());
        }

        self.spawn(
            run.id,
            run.conversation_id,
            run.current_head_message_id,
            Some(ModelName::new(run.model)),
            run.reasoning,
            RunCommand::Continue,
        );

        Ok(())
    }

    /// Approves one pending tool call and continues the waiting run.
    pub fn approve_tool(&self, run_id: &RunId, tool_call_id: ToolCallId) -> Result<()> {
        self.resume_with_command(run_id, RunCommand::ApproveTool(tool_call_id))
    }

    /// Denies one pending tool call and continues the waiting run.
    pub fn deny_tool(&self, run_id: &RunId, tool_call_id: ToolCallId) -> Result<()> {
        self.resume_with_command(run_id, RunCommand::DenyTool(tool_call_id))
    }

    /// Resumes all waiting runs in one conversation. Used after switching tool
    /// policy to full access.
    pub fn resume_waiting_for_conversation(&self, conversation_id: &ConversationId) -> Result<()> {
        let store = self.open_store()?;
        let runs = store.list_conversation_runs(conversation_id)?;
        drop(store);

        for run in runs
            .into_iter()
            .filter(|run| run.status == RunStatus::WaitingForApproval)
        {
            self.resume(&run.id)?;
        }

        Ok(())
    }

    fn spawn(
        &self,
        run_id: RunId,
        conversation_id: ConversationId,
        head_message_id: Option<MessageId>,
        model_override: Option<ModelName>,
        reasoning: Option<ReasoningRequest>,
        command: RunCommand,
    ) {
        let (sender, _) = broadcast::channel(RUN_EVENT_CHANNEL_CAPACITY);
        let task_sender = sender.clone();
        let manager = self.clone();
        let run_id_for_task = run_id.clone();

        let task = tokio::spawn(async move {
            let result = manager
                .run_task(
                    run_id_for_task.clone(),
                    conversation_id,
                    head_message_id,
                    model_override,
                    reasoning,
                    command,
                    task_sender,
                )
                .await;
            if let Err(error) = result {
                manager
                    .record_failure(&run_id_for_task, &error)
                    .unwrap_or_else(|failure_error| {
                        eprintln!("failed to persist run failure: {failure_error}");
                    });
            }
            manager
                .active
                .lock()
                .expect("run manager lock poisoned")
                .remove(run_id_for_task.as_str());
        });

        let run_key = run_id.as_str().to_string();
        let mut active = self.active.lock().expect("run manager lock poisoned");
        active.insert(run_key.clone(), RunningRun { sender, task });
        if active
            .get(&run_key)
            .is_some_and(|running| running.task.is_finished())
        {
            active.remove(&run_key);
        }
    }

    async fn run_task(
        &self,
        run_id: RunId,
        conversation_id: ConversationId,
        head_message_id: Option<MessageId>,
        model_override: Option<ModelName>,
        reasoning: Option<ReasoningRequest>,
        command: RunCommand,
        sender: broadcast::Sender<RunEventRecord>,
    ) -> Result<()> {
        let mut store = self.open_store()?;
        store.update_run_status(&run_id, RunStatus::Running, None)?;
        let output = RunOutput {
            store_path: self.store_path.clone(),
            run_id: run_id.clone(),
            sender: sender.clone(),
        };
        let events = RunEvents {
            store_path: self.store_path.clone(),
            run_id: run_id.clone(),
            sender,
        };
        let runtime = RunRuntime::new(
            GatewayUrl::new(self.gateway_url.clone()),
            BaseUrl::new(self.base_url.clone()),
            model_override,
            reasoning,
            self.tools.as_ref(),
        );

        let outcome = match command {
            RunCommand::Continue => {
                operation::run_until_blocked(
                    &output,
                    &events,
                    &mut store,
                    &conversation_id,
                    head_message_id.as_ref(),
                    runtime,
                )
                .await?
            }
            RunCommand::ApproveTool(tool_call_id) => {
                operation::approve_run_tool(
                    &output,
                    &events,
                    &mut store,
                    &conversation_id,
                    head_message_id.as_ref(),
                    &tool_call_id,
                    runtime,
                )
                .await?
            }
            RunCommand::DenyTool(tool_call_id) => {
                operation::deny_run_tool(
                    &output,
                    &events,
                    &mut store,
                    &conversation_id,
                    head_message_id.as_ref(),
                    &tool_call_id,
                    runtime,
                )
                .await?
            }
        };

        match outcome {
            crate::runtime::RuntimeOutcome::Completed { head_message_id } => {
                store.update_run_head(&run_id, head_message_id.as_ref())?;
                store.update_run_status(&run_id, RunStatus::Completed, None)?;
                events.record(RunEvent::Completed {
                    message_id: head_message_id.map(|id| id.as_str().to_string()),
                })?;
            }
            crate::runtime::RuntimeOutcome::WaitingForApproval { head_message_id } => {
                store.update_run_head(&run_id, Some(&head_message_id))?;
                store.update_run_status(&run_id, RunStatus::WaitingForApproval, None)?;
                events.record(RunEvent::WaitingForApproval)?;
            }
        }

        Ok(())
    }

    fn record_failure(&self, run_id: &RunId, error: &anyhow::Error) -> Result<()> {
        let causes = error.chain().map(ToString::to_string).collect::<Vec<_>>();
        let message = error
            .chain()
            .last()
            .map(ToString::to_string)
            .unwrap_or_else(|| error.to_string());
        let mut store = self.open_store()?;
        store.update_run_status(run_id, RunStatus::Failed, Some(&message))?;
        let record = store.append_run_event(
            run_id,
            RunEvent::Failed {
                error: message,
                causes,
            },
        )?;
        if let Some(running) = self
            .active
            .lock()
            .expect("run manager lock poisoned")
            .get(run_id.as_str())
        {
            let _ = running.sender.send(record);
        }

        Ok(())
    }

    fn open_store(&self) -> Result<Store> {
        match self.store_path.as_ref() {
            Some(path) => Store::open_at(path),
            None => Store::open(),
        }
    }

    fn resume_with_command(&self, run_id: &RunId, command: RunCommand) -> Result<()> {
        let store = self.open_store()?;
        let run = store.load_run(run_id)?;
        drop(store);

        if self
            .active
            .lock()
            .expect("run manager lock poisoned")
            .contains_key(run_id.as_str())
        {
            return Ok(());
        }

        self.spawn(
            run.id,
            run.conversation_id,
            run.current_head_message_id,
            Some(ModelName::new(run.model)),
            run.reasoning,
            command,
        );

        Ok(())
    }
}

enum RunCommand {
    Continue,
    ApproveTool(ToolCallId),
    DenyTool(ToolCallId),
}

struct RunEvents {
    store_path: Option<PathBuf>,
    run_id: RunId,
    sender: broadcast::Sender<RunEventRecord>,
}

impl RunEvents {
    fn record(&self, event: RunEvent) -> Result<RunEventRecord> {
        let mut store = match self.store_path.as_ref() {
            Some(path) => Store::open_at(path),
            None => Store::open(),
        }?;
        let record = store.append_run_event(&self.run_id, event)?;
        let _ = self.sender.send(record.clone());

        Ok(record)
    }
}

impl RuntimeEventSink for RunEvents {
    fn assistant_message_saved(&self, message_id: &MessageId) {
        if let Err(error) = self.record(RunEvent::AssistantMessageSaved {
            message_id: message_id.as_str().to_string(),
        }) {
            eprintln!("failed to append runtime event: {error}");
        }
    }

    fn tool_result_saved(&self, message_id: &MessageId) {
        if let Err(error) = self.record(RunEvent::ToolResultSaved {
            message_id: message_id.as_str().to_string(),
        }) {
            eprintln!("failed to append runtime event: {error}");
        }
    }
}

struct RunOutput {
    store_path: Option<PathBuf>,
    run_id: RunId,
    sender: broadcast::Sender<RunEventRecord>,
}

impl RunOutput {
    fn record(&self, event: RunEvent) -> Result<()> {
        let mut store = match self.store_path.as_ref() {
            Some(path) => Store::open_at(path),
            None => Store::open(),
        }?;
        let record = store.append_run_event(&self.run_id, event)?;
        let _ = self.sender.send(record);

        Ok(())
    }
}

impl RuntimeOutput for RunOutput {
    fn start_assistant_message(&self) {}

    fn assistant_delta(&self, text: &str) -> Result<()> {
        self.record(RunEvent::AssistantDelta {
            text: text.to_string(),
        })
    }

    fn reasoning_delta(&self, text: &str) -> Result<()> {
        self.record(RunEvent::ReasoningDelta {
            text: text.to_string(),
        })
    }

    fn tool_call_delta(
        &self,
        index: u16,
        id: Option<&str>,
        name: Option<&str>,
        arguments_delta: Option<&str>,
    ) -> Result<()> {
        self.record(RunEvent::ToolCallDelta {
            index,
            id: id.map(str::to_string),
            name: name.map(str::to_string),
            arguments_delta: arguments_delta.map(str::to_string),
        })
    }

    fn end_assistant_message(&self) {}

    fn assistant_tool_calls(&self, _tool_calls: &[crate::conversation::ToolCall]) {}
}
