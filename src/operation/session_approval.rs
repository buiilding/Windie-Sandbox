//! Session tool-approval operation workflows.
//! Tree-wide tools: tool lookup is conversation-wide.

use super::*;

#[derive(Debug, Clone, Serialize)]
/// One session-owned pending approval surfaced to clients.
pub struct SessionToolApprovalRequest {
    pub session_id: SessionId,
    pub conversation_id: ConversationId,
    pub session_status: SessionStatus,
    pub head_message_id: Option<MessageId>,
    pub approval: ToolApprovalRequest,
}

pub fn list_session_tool_approvals_with_registry(
    store: &Store,
    conversation_id: &ConversationId,
    head_message_id: Option<&MessageId>,
    registry: &ToolProviderRegistry,
) -> Result<Vec<ToolApprovalRequest>> {
    pending_approvals_at_head(
        store,
        RuntimeInput {
            conversation_id,
            head_message_id,
            tools: registry,
            model_request: RuntimeModelRequest::new(None, None),
        },
    )
}

pub fn list_session_approvals_with_registry(
    store: &Store,
    session: &Session,
    registry: &ToolProviderRegistry,
) -> Result<Vec<SessionToolApprovalRequest>> {
    let approvals = list_session_tool_approvals_with_registry(
        store,
        &session.conversation_id,
        session.current_head_message_id.as_ref(),
        registry,
    )?;

    Ok(approvals
        .into_iter()
        .map(|approval| SessionToolApprovalRequest {
            session_id: session.id.clone(),
            conversation_id: session.conversation_id.clone(),
            session_status: session.status,
            head_message_id: session.current_head_message_id.clone(),
            approval,
        })
        .collect())
}

pub fn list_conversation_session_approvals_with_registry(
    store: &Store,
    conversation_id: &ConversationId,
    registry: &ToolProviderRegistry,
) -> Result<Vec<SessionToolApprovalRequest>> {
    let mut approvals = Vec::new();

    for session in store.list_conversation_sessions(conversation_id)? {
        if session.status != SessionStatus::WaitingForApproval {
            continue;
        }
        approvals.extend(list_session_approvals_with_registry(
            store, &session, registry,
        )?);
    }

    Ok(approvals)
}

pub async fn approve_session_tool<O, E>(
    output: &O,
    events: &E,
    store: &mut Store,
    conversation_id: &ConversationId,
    head_message_id: Option<&MessageId>,
    tool_call_id: &ToolCallId,
    runtime: RuntimeDependencies<'_>,
) -> Result<RuntimeOutcome>
where
    O: RuntimeOutput,
    E: RuntimeEventSink,
{
    let pending =
        load_pending_tool_call_at_head(store, conversation_id, head_message_id, tool_call_id)?;
    let execution = prepare_pending_tool_execution(store, conversation_id, &pending, runtime.tools)?;
    let result = match execution {
        PendingToolExecution::Finished(result) => result,
        PendingToolExecution::Execute(attached_tool) => {
            execute_pending_tool_call(&pending, &attached_tool, runtime.tools).await?
        }
    };
    let message_id = store_pending_tool_result_at_head(store, conversation_id, &pending, &result)?;
    events.tool_result_saved(&message_id);

    continue_session_after_tool_result(output, events, store, conversation_id, &message_id, runtime)
        .await
}

pub async fn deny_session_tool<O, E>(
    output: &O,
    events: &E,
    store: &mut Store,
    conversation_id: &ConversationId,
    head_message_id: Option<&MessageId>,
    tool_call_id: &ToolCallId,
    runtime: RuntimeDependencies<'_>,
) -> Result<RuntimeOutcome>
where
    O: RuntimeOutput,
    E: RuntimeEventSink,
{
    let pending =
        load_pending_tool_call_at_head(store, conversation_id, head_message_id, tool_call_id)?;
    let result = deny_pending_tool_call(&pending);
    let message_id = store_pending_tool_result_at_head(store, conversation_id, &pending, &result)?;
    events.tool_result_saved(&message_id);

    continue_session_after_tool_result(output, events, store, conversation_id, &message_id, runtime)
        .await
}

async fn continue_session_after_tool_result<O, E>(
    output: &O,
    events: &E,
    store: &mut Store,
    conversation_id: &ConversationId,
    head_message_id: &MessageId,
    runtime: RuntimeDependencies<'_>,
) -> Result<RuntimeOutcome>
where
    O: RuntimeOutput,
    E: RuntimeEventSink,
{
    if !pending_approvals_at_head(
        store,
        RuntimeInput {
            conversation_id,
            head_message_id: Some(head_message_id),
            tools: runtime.tools,
            model_request: RuntimeModelRequest::new(None, None),
        },
    )?
    .is_empty()
    {
        return Ok(RuntimeOutcome::WaitingForApproval {
            head_message_id: head_message_id.clone(),
        });
    }

    advance_session_until_blocked(
        output,
        events,
        store,
        conversation_id,
        Some(head_message_id),
        runtime,
    )
    .await
}
