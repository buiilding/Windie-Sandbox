//! Session approval API route handlers.

use super::*;

#[derive(Debug, Serialize)]
/// Response body for pending session-owned tool approvals.
pub(super) struct ApprovalListResponse {
    pub(super) approvals: Vec<ApprovalResponse>,
}

#[derive(Debug, Serialize)]
/// One pending session-owned approval returned to UI clients.
pub(super) struct ApprovalResponse {
    pub(super) scope: &'static str,
    pub(super) session_id: String,
    pub(super) conversation_id: String,
    pub(super) session_status: SessionStatus,
    pub(super) head_message_id: Option<String>,
    pub(super) assistant_message_id: String,
    pub(super) tool_call_id: String,
    pub(super) tool_name: String,
    pub(super) arguments: String,
    pub(super) reason: String,
}

impl From<operation::SessionToolApprovalRequest> for ApprovalResponse {
    fn from(request: operation::SessionToolApprovalRequest) -> Self {
        let approval = request.approval;
        Self {
            scope: "session",
            session_id: request.session_id.as_str().to_string(),
            conversation_id: request.conversation_id.as_str().to_string(),
            session_status: request.session_status,
            head_message_id: request.head_message_id.map(|id| id.as_str().to_string()),
            assistant_message_id: approval.assistant_message_id.as_str().to_string(),
            tool_call_id: approval.tool_call.id.as_str().to_string(),
            tool_name: approval.tool_call.name().to_string(),
            arguments: approval.tool_call.arguments().to_string(),
            reason: approval.reason,
        }
    }
}

/// Lists pending session-owned tool calls waiting for approval in a conversation.
pub(super) async fn list_conversation_session_approvals(
    State(state): State<ApiState>,
    Path(conversation_id): Path<String>,
) -> ApiResult<ApprovalListResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let store = open_store(&state)?;
    let approvals = operation::list_conversation_session_approvals_with_registry(
        &store,
        &conversation_id,
        &state.tool_registry,
    )?
    .into_iter()
    .map(ApprovalResponse::from)
    .collect();

    Ok(Json(ApprovalListResponse { approvals }))
}

/// Lists pending tool calls waiting for approval in one session.
pub(super) async fn list_session_approvals(
    State(state): State<ApiState>,
    Path(session_id): Path<String>,
) -> ApiResult<ApprovalListResponse> {
    let session_id = SessionId::new(session_id);
    let store = open_store(&state)?;
    let session = store.load_session(&session_id)?;
    let approvals =
        operation::list_session_approvals_with_registry(&store, &session, &state.tool_registry)?
            .into_iter()
            .map(ApprovalResponse::from)
            .collect();

    Ok(Json(ApprovalListResponse { approvals }))
}

/// Executes one approved pending tool call and resumes its session.
pub(super) async fn approve_session_tool(
    State(state): State<ApiState>,
    Path((session_id, tool_call_id)): Path<(String, String)>,
) -> ApiResult<SessionResponse> {
    let session_id = SessionId::new(session_id);
    state
        .session_manager
        .approve_tool(&session_id, ToolCallId::new(tool_call_id))?;
    let store = open_store(&state)?;
    let session = store.load_session(&session_id)?;

    Ok(Json(SessionResponse::from_session(session)))
}

/// Stores a rejected result for one pending tool call and resumes its session.
pub(super) async fn deny_session_tool(
    State(state): State<ApiState>,
    Path((session_id, tool_call_id)): Path<(String, String)>,
) -> ApiResult<SessionResponse> {
    let session_id = SessionId::new(session_id);
    state
        .session_manager
        .deny_tool(&session_id, ToolCallId::new(tool_call_id))?;
    let store = open_store(&state)?;
    let session = store.load_session(&session_id)?;

    Ok(Json(SessionResponse::from_session(session)))
}
