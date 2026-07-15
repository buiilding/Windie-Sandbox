//! Runtime session lifecycle and advancement workflows.

use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
/// Action a session manager should take for a session-targeted wakeup.
pub enum SessionResumeAction {
    ApproveTool(ToolCallId),
    DenyTool(ToolCallId),
    Stop,
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
        }
    }
}

/// Creates a durable session from a wakeup and captures the head/model used.
pub fn start_session_from_wakeup(store: &mut Store, wakeup: ContinueWakeup) -> Result<Session> {
    let head_message_id = wakeup.head_message_id;
    let model = match wakeup.model {
        Some(model) => model,
        None => conversation_model(store, &wakeup.conversation_id)?,
    };
    let session_id = SessionId::fresh();

    store.create_session(
        &session_id,
        &wakeup.conversation_id,
        head_message_id.as_ref(),
        model.as_str(),
        wakeup.reasoning.as_ref(),
    )
}

/// Resolves a session-targeted wakeup into the persisted session and action.
///
/// Conversation wakeups create new sessions through `start_session_from_wakeup`.
/// This helper is only for wakeups that target an already durable session.
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
        Wakeup::Stop(stop) => (stop.session_id, SessionResumeAction::Stop),
        Wakeup::Query(_) | Wakeup::Continue(_) => {
            anyhow::bail!("conversation wakeups create sessions instead of resuming them")
        }
    };
    let session = store.load_session(&session_id)?;

    if action != SessionResumeAction::Stop && session.status != SessionStatus::WaitingForApproval {
        return Ok(None);
    }

    Ok(Some(SessionResume { session, action }))
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
