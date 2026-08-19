//! Runtime session lifecycle and advancement workflows.

use super::*;

use crate::plugin::PluginCatalog;
use crate::session::SessionResolution;

#[derive(Debug, Clone, PartialEq, Eq)]
/// Action a session manager should take for a session-targeted wakeup.
pub enum SessionResumeAction {
    ApproveTool(ToolCallId),
    DenyTool(ToolCallId),
}

#[derive(Debug, Clone)]
/// Session and action resolved from a wakeup that targets an existing session.
pub struct SessionResume {
    pub session: Session,
    pub action: SessionResumeAction,
}

/// Provider/runtime inputs needed to execute a run.
///
/// Long-lived API execution and blocking CLI calls both pass through this
/// struct so gateway, Bifrost endpoint, model override, reasoning, and tool
/// executor access stay explicit.
pub struct RuntimeDependencies<'a> {
    pub(in crate::operation) gateway_url: GatewayUrl,
    pub(in crate::operation) base_url: BaseUrl,
    pub(in crate::operation) model_override: Option<ModelName>,
    pub(in crate::operation) reasoning: Option<ReasoningRequest>,
    pub(in crate::operation) tools: &'a ToolProviderRegistry,
    pub(in crate::operation) plugin_catalog: Option<&'a PluginCatalog>,
}

impl<'a> RuntimeDependencies<'a> {
    /// Groups provider/runtime dependencies for one session.
    pub fn new(
        gateway_url: GatewayUrl,
        base_url: BaseUrl,
        model_override: Option<ModelName>,
        reasoning: Option<ReasoningRequest>,
        tools: &'a ToolProviderRegistry,
    ) -> Self {
        Self {
            gateway_url,
            base_url,
            model_override,
            reasoning,
            tools,
            plugin_catalog: None,
        }
    }

    /// Builds runtime dependencies from one durable session record.
    ///
    /// Session execution must use the persisted model and reasoning settings
    /// rather than reconstructing them independently in each client adapter.
    pub fn for_session(
        session: &Session,
        gateway_url: GatewayUrl,
        base_url: BaseUrl,
        tools: &'a ToolProviderRegistry,
        plugin_catalog: Option<&'a PluginCatalog>,
    ) -> Self {
        let runtime = Self::new(
            gateway_url,
            base_url,
            Some(ModelName::new(session.model.clone())),
            session.reasoning.clone(),
            tools,
        );

        match plugin_catalog {
            Some(catalog) => runtime.with_plugin_catalog(catalog),
            None => runtime,
        }
    }

    /// Adds the read-only plugin catalog used to build model context and
    /// resolve plugin-owned built-in actions.
    pub fn with_plugin_catalog(mut self, catalog: &'a PluginCatalog) -> Self {
        self.plugin_catalog = Some(catalog);
        self
    }
}

/// Persists session events and head changes for one client adapter.
///
/// API and CLI adapters may publish or display the resulting event
/// differently, but the durable SQLite write and session-head update must stay
/// identical. This helper deliberately has no live-event transport policy.
#[derive(Debug, Clone)]
pub(crate) struct SessionEventRecorder {
    store_path: Option<PathBuf>,
    session_id: SessionId,
}

impl SessionEventRecorder {
    /// Creates a recorder for the default store or an explicit test store.
    pub(crate) fn new(store_path: Option<PathBuf>, session_id: SessionId) -> Self {
        Self {
            store_path,
            session_id,
        }
    }

    /// Appends one replayable event and returns its persisted record.
    pub(crate) fn record(&self, event: SessionEvent) -> Result<crate::session::SessionEventRecord> {
        let mut store = self.open_store()?;
        store.append_session_event(&self.session_id, event)
    }

    /// Moves the durable session head after a message was persisted.
    pub(crate) fn update_head(&self, message_id: &MessageId) -> Result<()> {
        let mut store = self.open_store()?;
        store.update_session_head(&self.session_id, Some(message_id))
    }

    fn open_store(&self) -> Result<Store> {
        match self.store_path.as_ref() {
            Some(path) => Store::open_at(path),
            None => Store::open(),
        }
    }
}

/// Resolves a session-targeted wakeup into the persisted session and action.
///
/// Sessions are created as selectable branches through the store and started
/// through the session manager. This helper is only for wakeups that target an
/// already durable session: approving or denying a tool call.
pub fn resume_session_from_wakeup(store: &Store, wakeup: Wakeup) -> Result<Option<SessionResume>> {
    let (session_id, action) = match wakeup {
        Wakeup::ApproveTool(decision) => (
            decision.session_id,
            SessionResumeAction::ApproveTool(decision.tool_call_id),
        ),
        Wakeup::DenyTool(decision) => (
            decision.session_id,
            SessionResumeAction::DenyTool(decision.tool_call_id),
        ),
    };
    let session = store.load_session(&session_id)?;

    if session.status != SessionStatus::WaitingForApproval {
        return Ok(None);
    }

    Ok(Some(SessionResume { session, action }))
}

/// Resolves an explicit control input to the durable session it targets.
pub fn resolve_session_control(store: &Store, control: SessionControl) -> Result<Session> {
    match control {
        SessionControl::Cancel(cancellation) => store.load_session(&cancellation.session_id),
    }
}

/// Persists the terminal status/head and final event for a session outcome.
pub fn finish_session(
    store: &mut Store,
    session_id: &SessionId,
    outcome: RuntimeOutcome,
) -> Result<crate::session::SessionEventRecord> {
    match outcome {
        RuntimeOutcome::Completed { head_message_id } => {
            store.update_session_head(session_id, head_message_id.as_ref())?;
            store.update_session_status(session_id, SessionStatus::Completed, None)?;
            store.append_session_event(
                session_id,
                SessionEvent::Completed {
                    message_id: head_message_id.map(|id| id.as_str().to_string()),
                },
            )
        }
        RuntimeOutcome::WaitingForApproval { head_message_id } => {
            store.update_session_head(session_id, Some(&head_message_id))?;
            store.update_session_status(session_id, SessionStatus::WaitingForApproval, None)?;
            store.append_session_event(session_id, SessionEvent::WaitingForApproval)
        }
    }
}

/// Cancels one session and records the same durable event used by the API
/// session manager.
pub fn cancel_session(
    store: &mut Store,
    session_id: &SessionId,
) -> Result<(Session, crate::session::SessionEventRecord)> {
    let _session = resolve_session_control(
        store,
        SessionControl::Cancel(SessionCancellation {
            session_id: session_id.clone(),
        }),
    )?;
    store.update_session_status(session_id, SessionStatus::Cancelled, None)?;
    let record = store.append_session_event(session_id, SessionEvent::Cancelled)?;
    Ok((store.load_session(session_id)?, record))
}

/// Removes one terminal session and its exclusive conversation-tree suffix.
pub fn remove_session(store: &mut Store, session_id: &SessionId) -> Result<()> {
    store.remove_session(session_id)
}

/// Resolves a conversation head to its durable session branch.
pub fn resolve_session_at_head(
    store: &Store,
    conversation_id: &ConversationId,
    head_message_id: Option<&MessageId>,
) -> Result<SessionResolution> {
    store.resolve_session_at_head(conversation_id, head_message_id)
}

/// Resolves or creates one durable session branch at a conversation head.
pub fn resolve_or_create_session(
    store: &mut Store,
    conversation_id: &ConversationId,
    head_message_id: Option<&MessageId>,
    model: &ModelName,
    reasoning: Option<&ReasoningRequest>,
) -> Result<Session> {
    store.resolve_or_create_session(conversation_id, head_message_id, model.as_str(), reasoning)
}

/// Persists a failed session status and replayable failure event.
pub fn record_session_failure(
    store: &mut Store,
    session_id: &SessionId,
    error: &anyhow::Error,
) -> Result<crate::session::SessionEventRecord> {
    let causes = error.chain().map(ToString::to_string).collect::<Vec<_>>();
    let message = error
        .chain()
        .last()
        .map(ToString::to_string)
        .unwrap_or_else(|| error.to_string());

    store.update_session_status(session_id, SessionStatus::Failed, Some(&message))?;
    store.append_session_event(
        session_id,
        SessionEvent::Failed {
            error: message,
            causes,
        },
    )
}

/// Advances one backend-owned execution until it completes or waits for approval.
pub async fn advance_session_until_blocked<O, E>(
    output: &O,
    events: &E,
    store: &mut Store,
    conversation_id: &ConversationId,
    head_message_id: Option<&MessageId>,
    runtime: RuntimeDependencies<'_>,
) -> Result<RuntimeOutcome>
where
    O: RuntimeOutput,
    E: RuntimeEventSink,
{
    require_gateway_running(runtime.gateway_url).await?;
    let model = resolve_conversation_model(store, conversation_id, runtime.model_override)?;
    let reasoning = resolve_reasoning_request(store, conversation_id, runtime.reasoning)?;
    let reasoning = reasoning_request_for_model(&model, reasoning);
    let prompt_cache =
        prompt_cache_request(runtime.base_url.clone(), &model, conversation_id).await;
    let llm = BifrostClient::new(runtime.base_url, model);

    runtime_advance_until_blocked(
        output,
        &llm,
        store,
        RuntimeInput {
            conversation_id,
            head_message_id,
            tools: runtime.tools,
            plugin_catalog: runtime.plugin_catalog,
            model_request: RuntimeModelRequest::new(reasoning.as_ref(), prompt_cache.as_ref()),
        },
        events,
    )
    .await
}

/// Resolves the reasoning request for a runtime operation.
///
/// A caller-supplied request is a one-query override. When it is absent,
/// Windie uses the conversation-level persisted effort so CLI, API, and
/// inspector clients all flow through the same primitive.
pub(in crate::operation) fn resolve_reasoning_request(
    store: &Store,
    conversation_id: &ConversationId,
    reasoning_override: Option<ReasoningRequest>,
) -> Result<Option<ReasoningRequest>> {
    match reasoning_override {
        Some(reasoning) => Ok(Some(reasoning)),
        None => conversation_reasoning(store, conversation_id),
    }
}

/// Converts a client-selected reasoning setting into the request Windie should
/// send for one concrete model.
///
/// The UI only chooses a reasoning effort from Bifrost metadata. OpenAI
/// Responses models need an additional `summary` request before they stream
/// visible reasoning-summary deltas, so Windie adds that provider request
/// detail here instead of teaching every client about OpenAI-specific fields.
pub(in crate::operation) fn reasoning_request_for_model(
    model: &ModelName,
    reasoning: Option<ReasoningRequest>,
) -> Option<ReasoningRequest> {
    let mut reasoning = reasoning.filter(|reasoning| !reasoning.is_empty())?;

    if model.as_str().starts_with("openai/")
        && reasoning.effort.is_some()
        && reasoning.summary.is_none()
    {
        reasoning.summary = Some("auto".to_string());
    }

    Some(reasoning)
}
