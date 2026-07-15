//! Conversation-level API handlers.

use super::*;

#[derive(Debug, Serialize)]
/// API response for the persisted conversation list.
pub(super) struct ConversationListResponse {
    pub(super) conversations: Vec<ConversationSummary>,
}

#[derive(Debug, Serialize)]
/// One persisted conversation row used by UI sidebars.
pub(super) struct ConversationSummary {
    pub(super) id: String,
    pub(super) title: Option<String>,
    pub(super) model: String,
    pub(super) message_count: i64,
}

impl From<ConversationInfo> for ConversationSummary {
    fn from(info: ConversationInfo) -> Self {
        Self {
            id: info.id.as_str().to_string(),
            title: info.title,
            model: info.model,
            message_count: info.message_count,
        }
    }
}

/// Lists persisted conversations without loading their message trees.
pub(super) async fn list_conversations(
    State(state): State<ApiState>,
) -> ApiResult<ConversationListResponse> {
    let store = open_store(&state)?;
    let conversations = operation::list_conversations(&store)?
        .into_iter()
        .map(ConversationSummary::from)
        .collect();

    Ok(Json(ConversationListResponse { conversations }))
}

#[derive(Debug, Serialize)]
/// ID response for operations that create a conversation.
pub(super) struct ConversationIdResponse {
    pub(super) conversation_id: String,
}

/// Creates a new empty conversation.
pub(super) async fn create_conversation(
    State(state): State<ApiState>,
) -> ApiResult<ConversationIdResponse> {
    let store = open_store(&state)?;
    let conversation_id = operation::create_conversation(&store, &ModelName::new(state.model))?;

    Ok(Json(ConversationIdResponse {
        conversation_id: conversation_id.as_str().to_string(),
    }))
}

#[derive(Debug, Serialize)]
/// Generic deletion response.
pub(super) struct DeletedResponse {
    pub(super) deleted: bool,
}

/// Removes one conversation and all owned persisted data.
pub(super) async fn remove_conversation(
    State(state): State<ApiState>,
    Path(conversation_id): Path<String>,
) -> ApiResult<DeletedResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let mut store = open_store(&state)?;

    operation::remove_conversation(&mut store, &conversation_id)?;

    Ok(Json(DeletedResponse { deleted: true }))
}

#[derive(Debug, Deserialize)]
/// Request body for setting the conversation default model.
pub(super) struct ConversationModelRequest {
    pub(super) model: String,
}

#[derive(Debug, Serialize)]
/// Response for conversation model mutation.
pub(super) struct ConversationModelResponse {
    pub(super) model: String,
}

/// Sets the conversation model used by future queries.
pub(super) async fn set_conversation_model(
    State(state): State<ApiState>,
    Path(conversation_id): Path<String>,
    Json(request): Json<ConversationModelRequest>,
) -> ApiResult<ConversationModelResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let mut store = open_store(&state)?;
    let model = ModelName::new(request.model);

    operation::set_conversation_model(&mut store, &conversation_id, &model)?;

    Ok(Json(ConversationModelResponse {
        model: operation::conversation_model(&store, &conversation_id)?
            .as_str()
            .to_string(),
    }))
}

#[derive(Debug, Deserialize)]
/// Request body for setting the conversation default reasoning effort.
pub(super) struct ConversationReasoningRequest {
    pub(super) effort: Option<String>,
}

#[derive(Debug, Serialize)]
/// Response for conversation reasoning mutation.
pub(super) struct ConversationReasoningResponse {
    pub(super) reasoning: Option<ReasoningRequest>,
}

/// Sets the conversation reasoning effort used by future queries.
pub(super) async fn set_conversation_reasoning(
    State(state): State<ApiState>,
    Path(conversation_id): Path<String>,
    Json(request): Json<ConversationReasoningRequest>,
) -> ApiResult<ConversationReasoningResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let mut store = open_store(&state)?;
    let reasoning = operation::set_conversation_reasoning_effort(
        &mut store,
        &conversation_id,
        request.effort.as_deref(),
    )?;

    Ok(Json(ConversationReasoningResponse { reasoning }))
}

/// Deletes descendants after one checkpoint message.
pub(super) async fn truncate_conversation(
    State(state): State<ApiState>,
    Path(conversation_id): Path<String>,
    Json(request): Json<MessageIdRequest>,
) -> ApiResult<MessageIdResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let message_id = MessageId::new(request.message_id);
    let mut store = open_store(&state)?;

    operation::truncate_conversation(&mut store, &conversation_id, &message_id)?;

    Ok(Json(MessageIdResponse {
        message_id: message_id.as_str().to_string(),
    }))
}

/// Creates a new conversation copied through a checkpoint message.
pub(super) async fn fork_conversation(
    State(state): State<ApiState>,
    Path(conversation_id): Path<String>,
    Json(request): Json<MessageIdRequest>,
) -> ApiResult<ConversationIdResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let message_id = MessageId::new(request.message_id);
    let mut store = open_store(&state)?;
    let forked_conversation_id =
        operation::fork_conversation(&mut store, &conversation_id, &message_id)?;

    Ok(Json(ConversationIdResponse {
        conversation_id: forked_conversation_id.as_str().to_string(),
    }))
}
