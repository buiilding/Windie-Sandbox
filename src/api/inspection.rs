//! Conversation inspection API handlers.

use super::*;

/// Loads full read-only runtime state for one conversation.
pub(super) async fn inspect_conversation(
    axum::extract::State(state): axum::extract::State<ApiState>,
    Path(conversation_id): Path<String>,
    query: axum::extract::Query<InspectQuery>,
) -> ApiResult<InspectionReport> {
    let conversation_id = ConversationId::new(conversation_id);
    let store = open_store(&state)?;
    let head_message_id = query.head_message_id.clone().map(MessageId::new);
    let model_override = query.model.clone().map(ModelName::new);
    let report = operation::inspect_conversation(
        &store,
        &conversation_id,
        head_message_id.as_ref(),
        model_override,
    )?;

    Ok(Json(report))
}

#[derive(Debug, Deserialize)]
/// Optional query parameters for inspection.
pub(super) struct InspectQuery {
    pub(super) head_message_id: Option<String>,
    pub(super) model: Option<String>,
}
