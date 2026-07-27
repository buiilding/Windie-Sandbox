//! Live session supervision.
//!
//! This module owns backend task supervision for observable sessions. It starts
//! runtime work, records replayable session events, publishes live events to
//! subscribers, and handles stop/resume/approval commands. It does not own HTTP
//! routing, CLI parsing, terminal formatting, or SQLite schema details beyond
//! calling the store boundary.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;

use crate::conversation::{ConversationId, MessageId, ToolCallId};
use crate::gateway::GatewayUrl;
use crate::llm::{BaseUrl, ModelName, ReasoningRequest};
use crate::operation::{self, MessageInputPart, RuntimeDependencies};
use crate::output::RuntimeOutput;
use crate::runtime::RuntimeEventSink;
use crate::session::{
    Session, SessionEvent, SessionEventRecord, SessionId, SessionQueryResult, SessionStatus,
};
use crate::store::Store;
use crate::tool_provider::ToolProviderRegistry;
use crate::wakeup::{StopWakeup, ToolDecisionWakeup, Wakeup};

const SESSION_EVENT_CHANNEL_CAPACITY: usize = 256;

/// Live subscription to events from one session.
pub struct SessionSubscription {
    receiver: broadcast::Receiver<SessionEventRecord>,
}

impl SessionSubscription {
    /// Waits for the next live event from the subscribed session.
    pub async fn recv(&mut self) -> Result<SessionEventRecord> {
        loop {
            match self.receiver.recv().await {
                Ok(event) => return Ok(event),
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => {
                    return Err(anyhow::anyhow!("session event stream closed"));
                }
            }
        }
    }
}

/// Backend-owned runtime session supervisor.
#[derive(Clone)]
pub struct SessionManager {
    store_path: Option<PathBuf>,
    gateway_url: String,
    base_url: String,
    tools: Arc<ToolProviderRegistry>,
    /// Live running tasks, keyed by session. A task is removed when it finishes,
    /// including when the session pauses for approval.
    active: Arc<Mutex<HashMap<String, JoinHandle<()>>>>,
    /// Per-session gates that serialize input acceptance and run handoff.
    gates: Arc<Mutex<HashMap<String, Arc<Mutex<()>>>>>,
    /// Durable broadcast channel per session, keyed by session. This outlives
    /// any one task so a single subscription survives pause/resume across
    /// approval waits. Removed only on terminal completion.
    channels: Arc<Mutex<HashMap<String, broadcast::Sender<SessionEventRecord>>>>,
}

/// Complete input needed by one spawned session task.
struct SessionTaskInput {
    session_id: SessionId,
    conversation_id: ConversationId,
    head_message_id: Option<MessageId>,
    model_override: Option<ModelName>,
    reasoning: Option<ReasoningRequest>,
    command: SessionCommand,
    sender: broadcast::Sender<SessionEventRecord>,
}

impl SessionManager {
    /// Builds a session manager for the API server runtime.
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
            gates: Arc::new(Mutex::new(HashMap::new())),
            channels: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Deletes one terminal session after confirming no live task still owns it.
    pub async fn remove_session(&self, session_id: &SessionId) -> Result<()> {
        let gate = self.session_gate(session_id);
        let _gate = gate.lock().expect("session gate poisoned");
        let store = self.open_store()?;
        let session = store.load_session(session_id)?;
        drop(store);

        if self
            .active
            .lock()
            .expect("run manager lock poisoned")
            .contains_key(session_id.as_str())
        {
            return Err(crate::error::invalid_request(
                "cannot delete a live session; stop it first",
            ));
        }
        if matches!(
            session.status,
            SessionStatus::Running | SessionStatus::WaitingForApproval
        ) {
            return Err(crate::error::invalid_request(
                "cannot delete a running or approval-waiting session; stop it first",
            ));
        }

        let mut store = self.open_store()?;
        operation::remove_session(&mut store, session_id)?;
        self.channels
            .lock()
            .expect("run manager lock poisoned")
            .remove(session_id.as_str());
        Ok(())
    }

    /// Stops and joins all live tasks for a conversation before deleting it.
    ///
    /// Joining aborted tasks is important: the database delete must not race a
    /// task that is still able to persist a terminal status or event.
    pub async fn remove_conversation(&self, conversation_id: &ConversationId) -> Result<()> {
        let store = self.open_store()?;
        let sessions = store.list_conversation_sessions(conversation_id)?;
        drop(store);

        let session_ids = sessions
            .iter()
            .map(|session| session.id.as_str().to_string())
            .collect::<std::collections::HashSet<_>>();
        let tasks = {
            let mut active = self.active.lock().expect("run manager lock poisoned");
            session_ids
                .iter()
                .filter_map(|session_id| active.remove(session_id))
                .collect::<Vec<_>>()
        };

        for task in &tasks {
            task.abort();
        }
        self.channels
            .lock()
            .expect("run manager lock poisoned")
            .retain(|session_id, _| !session_ids.contains(session_id));
        for task in tasks {
            let _ = task.await;
        }

        let mut store = self.open_store()?;
        operation::remove_conversation(&mut store, conversation_id)
    }

    /// Creates a selectable branch without starting model execution.
    pub fn create_session_branch(
        &self,
        conversation_id: ConversationId,
        head_message_id: Option<MessageId>,
        model: String,
        reasoning: Option<ReasoningRequest>,
    ) -> Result<Session> {
        let mut store = self.open_store()?;
        store.create_session(
            &SessionId::fresh(),
            &conversation_id,
            head_message_id.as_ref(),
            &model,
            reasoning.as_ref(),
        )
    }

    /// Inserts one user query into a session branch and starts that session.
    pub fn query_session(
        &self,
        session_id: &SessionId,
        parts: &[MessageInputPart],
    ) -> Result<SessionQueryResult> {
        let gate = self.session_gate(session_id);
        let _gate = gate.lock().expect("session gate poisoned");
        let active = self
            .active
            .lock()
            .expect("run manager lock poisoned")
            .contains_key(session_id.as_str());

        let mut store = self.open_store()?;
        let session = store.load_session(session_id)?;
        if session.status == SessionStatus::WaitingForApproval {
            return Err(crate::error::invalid_request(
                "session is waiting for tool approval",
            ));
        }

        let prepared = operation::prepare_message_input(parts)?;
        if active {
            let input_id =
                store.enqueue_session_input(session_id, &prepared.content, &prepared.parts)?;
            let queue_depth = store.session_input_count(session_id)?;
            let record = store.append_session_event(
                session_id,
                SessionEvent::InputQueued {
                    input_id: input_id.as_str().to_string(),
                    queue_depth,
                },
            )?;
            self.channel_for_session(session_id).send(record).ok();
            let session = store.load_session(session_id)?;
            return Ok(SessionQueryResult {
                session,
                queued: true,
                input_id: Some(input_id),
                queue_depth,
            });
        }

        let user_message_id = operation::insert_prepared_user_message(
            &mut store,
            &session.conversation_id,
            session.current_head_message_id.as_ref(),
            &prepared,
        )?;
        store.update_session_head(session_id, Some(&user_message_id))?;
        store.update_session_status(session_id, SessionStatus::Running, None)?;

        let updated = store.load_session(session_id)?;
        let session_id_for_spawn = updated.id.clone();
        let conversation_id = updated.conversation_id.clone();
        let head_message_id = updated.current_head_message_id.clone();
        let model = updated.model.clone();
        let reasoning = updated.reasoning.clone();
        drop(store);

        self.spawn(
            session_id_for_spawn,
            conversation_id,
            head_message_id,
            Some(ModelName::new(model)),
            reasoning,
            SessionCommand::Continue,
        );

        Ok(SessionQueryResult {
            session: updated,
            queued: false,
            input_id: None,
            queue_depth: 0,
        })
    }

    /// Starts one selected session from its current head without adding input.
    pub fn continue_session(&self, session_id: &SessionId) -> Result<Session> {
        let gate = self.session_gate(session_id);
        let _gate = gate.lock().expect("session gate poisoned");
        if self
            .active
            .lock()
            .expect("run manager lock poisoned")
            .contains_key(session_id.as_str())
        {
            return Err(crate::error::invalid_request("session is already running"));
        }

        let mut store = self.open_store()?;
        let session = store.load_session(session_id)?;
        if session.status == SessionStatus::WaitingForApproval {
            return Err(crate::error::invalid_request(
                "session is waiting for tool approval",
            ));
        }

        if store.session_input_count(session_id)? > 0 {
            let input = store.materialize_next_session_input(session_id)?;
            if let Some(input) = input {
                let updated = store.load_session(session_id)?;
                let record = store.append_session_event(
                    session_id,
                    SessionEvent::InputStarted {
                        input_id: input.id.as_str().to_string(),
                        message_id: updated
                            .current_head_message_id
                            .as_ref()
                            .map(ToString::to_string)
                            .unwrap_or_default(),
                    },
                )?;
                self.channel_for_session(session_id).send(record).ok();
                drop(store);
                self.spawn(
                    updated.id.clone(),
                    updated.conversation_id.clone(),
                    updated.current_head_message_id.clone(),
                    Some(ModelName::new(updated.model.clone())),
                    updated.reasoning.clone(),
                    SessionCommand::Continue,
                );
                return Ok(updated);
            }
        }
        store.update_session_status(session_id, SessionStatus::Running, None)?;
        let updated = store.load_session(session_id)?;
        drop(store);

        self.spawn(
            updated.id.clone(),
            updated.conversation_id.clone(),
            updated.current_head_message_id.clone(),
            Some(ModelName::new(updated.model.clone())),
            updated.reasoning.clone(),
            SessionCommand::Continue,
        );

        Ok(updated)
    }

    /// Stops one live session explicitly.
    pub fn stop(&self, session_id: &SessionId) -> Result<()> {
        let gate = self.session_gate(session_id);
        let _gate = gate.lock().expect("session gate poisoned");
        let store = self.open_store()?;
        let Some(resume) = operation::resume_session_from_wakeup(
            &store,
            Wakeup::Stop(StopWakeup {
                session_id: session_id.clone(),
            }),
        )?
        else {
            return Ok(());
        };
        drop(store);

        let session_key = resume.session.id.as_str().to_string();
        let running_task = self
            .active
            .lock()
            .expect("run manager lock poisoned")
            .remove(&session_key);

        if let Some(task) = &running_task {
            task.abort();
        }

        let mut store = self.open_store()?;
        store.update_session_status(&resume.session.id, SessionStatus::Cancelled, None)?;
        let record = store.append_session_event(&resume.session.id, SessionEvent::Cancelled)?;

        // Send the terminal event on the durable channel, then remove it so the
        // stream closes after delivering the cancellation.
        let sender = self
            .channels
            .lock()
            .expect("run manager lock poisoned")
            .remove(&session_key);
        if let Some(sender) = sender {
            let _ = sender.send(record);
        }

        Ok(())
    }

    /// Subscribes to future live events from a run.
    ///
    /// The receiver is bound to the session's durable channel, so it stays
    /// valid across approval pauses and resumes on the same session.
    pub fn subscribe(&self, session_id: &SessionId) -> Option<SessionSubscription> {
        self.channels
            .lock()
            .expect("run manager lock poisoned")
            .get(session_id.as_str())
            .map(|sender| SessionSubscription {
                receiver: sender.subscribe(),
            })
    }

    /// Resumes a waiting session after a policy change.
    pub fn resume(&self, session_id: &SessionId) -> Result<()> {
        let gate = self.session_gate(session_id);
        let _gate = gate.lock().expect("session gate poisoned");
        let store = self.open_store()?;
        let session = store.load_session(session_id)?;
        drop(store);

        if self
            .active
            .lock()
            .expect("run manager lock poisoned")
            .contains_key(session_id.as_str())
        {
            return Ok(());
        }

        if session.status != SessionStatus::WaitingForApproval {
            return Ok(());
        }

        self.spawn(
            session.id,
            session.conversation_id,
            session.current_head_message_id,
            Some(ModelName::new(session.model)),
            session.reasoning,
            SessionCommand::Continue,
        );

        Ok(())
    }

    /// Approves one pending tool call and continues the waiting session.
    pub fn approve_tool(&self, session_id: &SessionId, tool_call_id: ToolCallId) -> Result<()> {
        self.resume_with_wakeup(Wakeup::ApproveTool(ToolDecisionWakeup {
            session_id: session_id.clone(),
            tool_call_id,
        }))
    }

    /// Denies one pending tool call and continues the waiting session.
    pub fn deny_tool(&self, session_id: &SessionId, tool_call_id: ToolCallId) -> Result<()> {
        self.resume_with_wakeup(Wakeup::DenyTool(ToolDecisionWakeup {
            session_id: session_id.clone(),
            tool_call_id,
        }))
    }

    /// Resumes all waiting sessions in one conversation. Used after switching tool
    /// policy to full access.
    pub fn resume_waiting_for_conversation(&self, conversation_id: &ConversationId) -> Result<()> {
        let store = self.open_store()?;
        let sessions = store.list_conversation_sessions(conversation_id)?;
        drop(store);

        for session in sessions
            .into_iter()
            .filter(|session| session.status == SessionStatus::WaitingForApproval)
        {
            self.resume(&session.id)?;
        }

        Ok(())
    }

    fn spawn(
        &self,
        session_id: SessionId,
        conversation_id: ConversationId,
        head_message_id: Option<MessageId>,
        model_override: Option<ModelName>,
        reasoning: Option<ReasoningRequest>,
        command: SessionCommand,
    ) {
        let session_key = session_id.as_str().to_string();

        // Reuse the session's durable channel, creating it only on first spawn.
        // This is what lets one subscription survive approval pauses and resumes.
        let sender = {
            let mut channels = self.channels.lock().expect("run manager lock poisoned");
            channels
                .entry(session_key.clone())
                .or_insert_with(|| broadcast::channel(SESSION_EVENT_CHANNEL_CAPACITY).0)
                .clone()
        };

        let manager = self.clone();
        let run_id_for_task = session_id.clone();
        let task = tokio::spawn(async move {
            let result = manager
                .run_task(SessionTaskInput {
                    session_id: run_id_for_task.clone(),
                    conversation_id,
                    head_message_id,
                    model_override,
                    reasoning,
                    command,
                    sender: sender.clone(),
                })
                .await;
            if let Err(error) = result {
                manager
                    .record_failure(&run_id_for_task, &error)
                    .unwrap_or_else(|failure_error| {
                        eprintln!("failed to persist run failure: {failure_error}");
                    });
            }

            manager.finish_task(&run_id_for_task);
        });

        let mut active = self.active.lock().expect("run manager lock poisoned");
        active.insert(session_key.clone(), task);
        if active
            .get(&session_key)
            .is_some_and(|running| running.is_finished())
        {
            active.remove(&session_key);
        }
    }

    async fn run_task(&self, input: SessionTaskInput) -> Result<()> {
        let SessionTaskInput {
            session_id,
            conversation_id,
            head_message_id,
            model_override,
            reasoning,
            command,
            sender,
        } = input;
        let mut store = self.open_store()?;
        store.update_session_status(&session_id, SessionStatus::Running, None)?;
        let output = SessionOutput {
            store_path: self.store_path.clone(),
            session_id: session_id.clone(),
            sender: sender.clone(),
        };
        let events = SessionEvents {
            store_path: self.store_path.clone(),
            session_id: session_id.clone(),
            sender,
        };
        let runtime = RuntimeDependencies::new(
            GatewayUrl::new(self.gateway_url.clone()),
            BaseUrl::new(self.base_url.clone()),
            model_override,
            reasoning,
            self.tools.as_ref(),
        );

        let outcome = match command {
            SessionCommand::Continue => {
                operation::advance_session_until_blocked(
                    &output,
                    &events,
                    &mut store,
                    &conversation_id,
                    head_message_id.as_ref(),
                    runtime,
                )
                .await?
            }
            SessionCommand::ApproveTool(tool_call_id) => {
                operation::approve_session_tool(
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
            SessionCommand::DenyTool(tool_call_id) => {
                operation::deny_session_tool(
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

        let record = operation::finish_session(&mut store, &session_id, outcome)?;
        let _ = events.sender.send(record);

        Ok(())
    }

    fn record_failure(&self, session_id: &SessionId, error: &anyhow::Error) -> Result<()> {
        let mut store = self.open_store()?;
        let record = operation::record_session_failure(&mut store, session_id, error)?;
        if let Some(sender) = self
            .channels
            .lock()
            .expect("run manager lock poisoned")
            .get(session_id.as_str())
        {
            let _ = sender.send(record);
        }

        Ok(())
    }

    /// Completes one task and immediately hands off to the next queued input.
    fn finish_task(&self, session_id: &SessionId) {
        let gate = self.session_gate(session_id);
        let _gate = gate.lock().expect("session gate poisoned");
        self.active
            .lock()
            .expect("run manager lock poisoned")
            .remove(session_id.as_str());

        let Ok(mut store) = self.open_store() else {
            return;
        };
        let Ok(session) = store.load_session(session_id) else {
            return;
        };

        if session.status == SessionStatus::Completed {
            match store.materialize_next_session_input(session_id) {
                Ok(Some(input)) => {
                    let Ok(updated) = store.load_session(session_id) else {
                        return;
                    };
                    if let Ok(record) = store.append_session_event(
                        session_id,
                        SessionEvent::InputStarted {
                            input_id: input.id.as_str().to_string(),
                            message_id: updated
                                .current_head_message_id
                                .as_ref()
                                .map(ToString::to_string)
                                .unwrap_or_default(),
                        },
                    ) {
                        self.channel_for_session(session_id).send(record).ok();
                    }
                    drop(store);
                    self.spawn(
                        updated.id,
                        updated.conversation_id,
                        updated.current_head_message_id,
                        Some(ModelName::new(updated.model)),
                        updated.reasoning,
                        SessionCommand::Continue,
                    );
                    return;
                }
                Ok(None) => {}
                Err(error) => {
                    eprintln!("failed to start queued session input: {error}");
                    return;
                }
            }
        }

        if matches!(
            session.status,
            SessionStatus::Completed | SessionStatus::Failed | SessionStatus::Cancelled
        ) {
            self.channels
                .lock()
                .expect("run manager lock poisoned")
                .remove(session_id.as_str());
        }
    }

    /// Returns the process-local gate for one session.
    fn session_gate(&self, session_id: &SessionId) -> Arc<Mutex<()>> {
        let mut gates = self.gates.lock().expect("session gates lock poisoned");
        gates
            .entry(session_id.as_str().to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    /// Returns the durable live-event channel, creating it when needed.
    fn channel_for_session(&self, session_id: &SessionId) -> broadcast::Sender<SessionEventRecord> {
        let mut channels = self.channels.lock().expect("run manager lock poisoned");
        channels
            .entry(session_id.as_str().to_string())
            .or_insert_with(|| broadcast::channel(SESSION_EVENT_CHANNEL_CAPACITY).0)
            .clone()
    }

    fn open_store(&self) -> Result<Store> {
        match self.store_path.as_ref() {
            Some(path) => Store::open_at(path),
            None => Store::open(),
        }
    }

    /// Marks tasks interrupted by an API-process restart as failed.
    ///
    /// The queued inputs remain durable and can be resumed explicitly with the
    /// normal continue operation. Replaying an interrupted model request
    /// automatically could duplicate provider-side work, so recovery is
    /// intentionally conservative.
    pub fn recover_interrupted_sessions(&self) -> Result<()> {
        let mut store = self.open_store()?;
        let sessions = store.list_sessions()?;
        for session in sessions
            .into_iter()
            .filter(|session| session.status == SessionStatus::Running)
        {
            store.update_session_status(
                &session.id,
                SessionStatus::Failed,
                Some("session task was interrupted by Windie restart"),
            )?;
            store.append_session_event(
                &session.id,
                SessionEvent::Failed {
                    error: "session task was interrupted by Windie restart".to_string(),
                    causes: vec![
                        "the previous Windie process stopped before completion".to_string(),
                    ],
                },
            )?;
        }
        Ok(())
    }

    fn resume_with_wakeup(&self, wakeup: Wakeup) -> Result<()> {
        let session_id = match &wakeup {
            Wakeup::ApproveTool(decision) => &decision.session_id,
            Wakeup::DenyTool(decision) => &decision.session_id,
            Wakeup::Stop(stop) => &stop.session_id,
        };
        let gate = self.session_gate(session_id);
        let _gate = gate.lock().expect("session gate poisoned");
        let store = self.open_store()?;
        let Some(resume) = operation::resume_session_from_wakeup(&store, wakeup)? else {
            return Ok(());
        };
        drop(store);

        if self
            .active
            .lock()
            .expect("run manager lock poisoned")
            .contains_key(resume.session.id.as_str())
        {
            return Ok(());
        }

        let command = match resume.action {
            operation::SessionResumeAction::ApproveTool(tool_call_id) => {
                SessionCommand::ApproveTool(tool_call_id)
            }
            operation::SessionResumeAction::DenyTool(tool_call_id) => {
                SessionCommand::DenyTool(tool_call_id)
            }
            operation::SessionResumeAction::Stop => return Ok(()),
        };

        self.spawn(
            resume.session.id,
            resume.session.conversation_id,
            resume.session.current_head_message_id,
            Some(ModelName::new(resume.session.model)),
            resume.session.reasoning,
            command,
        );

        Ok(())
    }
}

enum SessionCommand {
    Continue,
    ApproveTool(ToolCallId),
    DenyTool(ToolCallId),
}

struct SessionEvents {
    store_path: Option<PathBuf>,
    session_id: SessionId,
    sender: broadcast::Sender<SessionEventRecord>,
}

impl SessionEvents {
    fn open_store(&self) -> Result<Store> {
        match self.store_path.as_ref() {
            Some(path) => Store::open_at(path),
            None => Store::open(),
        }
    }

    fn record(&self, event: SessionEvent) -> Result<SessionEventRecord> {
        let mut store = self.open_store()?;
        let record = store.append_session_event(&self.session_id, event)?;
        let _ = self.sender.send(record.clone());

        Ok(record)
    }

    fn update_head(&self, message_id: &MessageId) {
        let result: Result<()> = (|| {
            let mut store = self.open_store()?;
            store.update_session_head(&self.session_id, Some(message_id))?;
            Ok(())
        })();
        if let Err(error) = result {
            eprintln!("failed to update session head: {error}");
        }
    }
}

impl RuntimeEventSink for SessionEvents {
    fn assistant_message_saved(&self, message_id: &MessageId) {
        if let Err(error) = self.record(SessionEvent::AssistantMessageSaved {
            message_id: message_id.as_str().to_string(),
        }) {
            eprintln!("failed to append runtime event: {error}");
        }
        self.update_head(message_id);
    }

    fn tool_result_saved(&self, message_id: &MessageId) {
        if let Err(error) = self.record(SessionEvent::ToolResultSaved {
            message_id: message_id.as_str().to_string(),
        }) {
            eprintln!("failed to append runtime event: {error}");
        }
        self.update_head(message_id);
    }
}

struct SessionOutput {
    store_path: Option<PathBuf>,
    session_id: SessionId,
    sender: broadcast::Sender<SessionEventRecord>,
}

impl SessionOutput {
    fn record(&self, event: SessionEvent) -> Result<()> {
        let mut store = match self.store_path.as_ref() {
            Some(path) => Store::open_at(path),
            None => Store::open(),
        }?;
        let record = store.append_session_event(&self.session_id, event)?;
        let _ = self.sender.send(record);

        Ok(())
    }
}

impl RuntimeOutput for SessionOutput {
    fn start_assistant_message(&self) {}

    fn assistant_delta(&self, text: &str) -> Result<()> {
        self.record(SessionEvent::AssistantDelta {
            text: text.to_string(),
        })
    }

    fn reasoning_delta(&self, text: &str) -> Result<()> {
        self.record(SessionEvent::ReasoningDelta {
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
        self.record(SessionEvent::ToolCallDelta {
            index,
            id: id.map(str::to_string),
            name: name.map(str::to_string),
            arguments_delta: arguments_delta.map(str::to_string),
        })
    }

    fn end_assistant_message(&self) {}

    fn assistant_tool_calls(&self, _tool_calls: &[crate::conversation::ToolCall]) {}
}
