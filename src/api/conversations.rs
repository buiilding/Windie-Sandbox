//! Conversation tree, settings, image, token-count, and direct query routes.

use super::{
    ApiError, ApiResult, ApiState, BaseUrl, CONTENT_TYPE, ConversationId, ConversationInfo,
    DeletedResponse, Deserialize, GatewayUrl, ImageAssetId, InspectionReport, Json, Message,
    MessageId, MessageInputPart, MessageMetadata, MessagePart, ModelName, Path, PathBuf,
    QueryRequest, ReasoningRequest, Response, Result, Role, Router, RuntimeOutput,
    RuntimeRunAction, Serialize, State, StatusCode, ToolCall, Value, error, get, open_store,
    operation, patch, post, run_store,
};
use anyhow::Context as _;
use base64::{Engine as _, engine::general_purpose::STANDARD};

pub(super) fn routes() -> Router<ApiState> {
    Router::new()
        .route(
            "/api/conversations",
            get(list_conversations).post(create_conversation),
        )
        .route(
            "/api/conversations/{conversation_id}",
            get(inspect_conversation).delete(remove_conversation),
        )
        .route(
            "/api/conversations/{conversation_id}/activate",
            post(activate_message),
        )
        .route(
            "/api/conversations/{conversation_id}/messages",
            post(insert_message),
        )
        .route(
            "/api/conversations/{conversation_id}/messages/{message_id}",
            patch(update_message).delete(remove_message),
        )
        .route(
            "/api/conversations/{conversation_id}/images/{asset_id}",
            get(get_conversation_image),
        )
        .route(
            "/api/conversations/{conversation_id}/system-prompt",
            patch(set_system_prompt).delete(remove_system_prompt),
        )
        .route(
            "/api/conversations/{conversation_id}/model",
            patch(set_conversation_model),
        )
        .route(
            "/api/conversations/{conversation_id}/reasoning",
            patch(set_conversation_reasoning),
        )
        .route(
            "/api/conversations/{conversation_id}/truncate",
            post(truncate_conversation),
        )
        .route(
            "/api/conversations/{conversation_id}/fork",
            post(fork_conversation),
        )
        .route(
            "/api/conversations/{conversation_id}/input-tokens",
            post(count_input_tokens),
        )
        .route("/api/conversations/{conversation_id}/query", post(query))
}

#[derive(Debug, Serialize)]
/// API response for the persisted conversation list.
struct ConversationListResponse {
    conversations: Vec<ConversationSummary>,
}

#[derive(Debug, Serialize)]
/// One persisted conversation row used by UI sidebars.
struct ConversationSummary {
    id: String,
    model: String,
    message_count: i64,
}

impl From<ConversationInfo> for ConversationSummary {
    fn from(info: ConversationInfo) -> Self {
        Self {
            id: info.id.as_str().to_string(),
            model: info.model,
            message_count: info.message_count,
        }
    }
}

/// Lists persisted conversations without loading their message trees.
async fn list_conversations(State(state): State<ApiState>) -> ApiResult<ConversationListResponse> {
    let conversations = run_store(&state, |store| operation::list_conversations(store))
        .await?
        .into_iter()
        .map(ConversationSummary::from)
        .collect();

    Ok(Json(ConversationListResponse { conversations }))
}

#[derive(Debug, Serialize)]
/// ID response for operations that create a conversation.
struct ConversationIdResponse {
    conversation_id: String,
}

/// Creates a new empty conversation.
async fn create_conversation(State(state): State<ApiState>) -> ApiResult<ConversationIdResponse> {
    let model = ModelName::new(state.model.clone());
    let conversation_id = run_store(&state, move |store| {
        operation::create_conversation(store, &model)
    })
    .await?;

    Ok(Json(ConversationIdResponse {
        conversation_id: conversation_id.as_str().to_string(),
    }))
}

/// Loads full read-only runtime state for one conversation.
async fn inspect_conversation(
    axum::extract::State(state): axum::extract::State<ApiState>,
    Path(conversation_id): Path<String>,
    query: axum::extract::Query<InspectQuery>,
) -> ApiResult<InspectionReport> {
    let conversation_id = ConversationId::new(conversation_id);
    let model_override = query.model.clone().map(ModelName::new);
    let report = run_store(&state, move |store| {
        operation::inspect_conversation(store, &conversation_id, model_override)
    })
    .await?;

    Ok(Json(report))
}

#[derive(Debug, Deserialize)]
/// Optional query parameters for inspection.
struct InspectQuery {
    model: Option<String>,
}

#[derive(Debug, Deserialize)]
/// Request body for selecting the active message.
struct MessageIdRequest {
    message_id: String,
}

#[derive(Debug, Serialize)]
/// Response for message selection.
struct ActiveMessageResponse {
    active_message_id: String,
}

/// Selects the active message for a conversation.
async fn activate_message(
    State(state): State<ApiState>,
    Path(conversation_id): Path<String>,
    Json(request): Json<MessageIdRequest>,
) -> ApiResult<ActiveMessageResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let message_id = MessageId::new(request.message_id);
    let response_id = message_id.clone();
    run_store(&state, move |store| {
        operation::activate_message(store, &conversation_id, &message_id)
    })
    .await?;

    Ok(Json(ActiveMessageResponse {
        active_message_id: response_id.as_str().to_string(),
    }))
}

#[derive(Debug, Deserialize)]
/// Request body for inserting one message.
struct InsertMessageRequest {
    role: Role,
    text: Option<String>,
    #[serde(default)]
    parts: Vec<InsertMessagePart>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
/// One ordered API message part to persist.
enum InsertMessagePart {
    Text { text: String },
    Image { path: PathBuf },
    ImageData { mime_type: String, data: String },
}

#[derive(Debug, Serialize)]
/// ID response for operations that create a message.
struct MessageIdResponse {
    message_id: String,
}

/// Inserts one message under the current active message.
async fn insert_message(
    State(state): State<ApiState>,
    Path(conversation_id): Path<String>,
    Json(request): Json<InsertMessageRequest>,
) -> ApiResult<MessageIdResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let parts = normalize_insert_parts(request.text, request.parts)?;
    let message_id = run_store(&state, move |store| {
        operation::insert_message(store, &conversation_id, request.role, &parts)
    })
    .await?;

    Ok(Json(MessageIdResponse {
        message_id: message_id.as_str().to_string(),
    }))
}

/// Converts request text and part fields into one ordered part list.
fn normalize_insert_parts(
    text: Option<String>,
    parts: Vec<InsertMessagePart>,
) -> Result<Vec<MessageInputPart>> {
    let parts = if parts.is_empty() {
        text.map(|text| vec![InsertMessagePart::Text { text }])
            .unwrap_or_default()
    } else {
        parts
    };

    if parts.is_empty() {
        return Err(error::invalid_request("message requires text or parts"));
    }

    let normalized = parts
        .into_iter()
        .map(|part| match part {
            InsertMessagePart::Text { text } => Ok(MessageInputPart::Text(text)),
            InsertMessagePart::Image { path } => Ok(MessageInputPart::ImagePath(path)),
            InsertMessagePart::ImageData { mime_type, data } => {
                let bytes = STANDARD.decode(data).map_err(|_| {
                    error::invalid_request("image_data must contain valid base64 data")
                })?;
                Ok(MessageInputPart::ImageBytes { mime_type, bytes })
            }
        })
        .collect::<Result<Vec<_>>>()?;

    if normalized
        .iter()
        .all(|part| matches!(part, MessageInputPart::Text(text) if text.is_empty()))
    {
        return Err(error::invalid_request(
            "message requires non-empty text or an image",
        ));
    }

    Ok(normalized)
}

#[derive(Debug, Deserialize)]
/// Request body for replacing one message.
struct UpdateMessageRequest {
    text: String,
}

/// Replaces one message's text content.
async fn update_message(
    State(state): State<ApiState>,
    Path((conversation_id, message_id)): Path<(String, String)>,
    Json(request): Json<UpdateMessageRequest>,
) -> ApiResult<MessageIdResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let message_id = MessageId::new(message_id);
    let response_id = message_id.clone();
    run_store(&state, move |store| {
        operation::update_message(store, &conversation_id, &message_id, &request.text)
    })
    .await?;

    Ok(Json(MessageIdResponse {
        message_id: response_id.as_str().to_string(),
    }))
}

/// Splices one message out of the conversation tree.
async fn remove_message(
    State(state): State<ApiState>,
    Path((conversation_id, message_id)): Path<(String, String)>,
) -> ApiResult<DeletedResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let message_id = MessageId::new(message_id);
    run_store(&state, move |store| {
        operation::remove_message(store, &conversation_id, &message_id)
    })
    .await?;

    Ok(Json(DeletedResponse { deleted: true }))
}

/// Returns durable image bytes for an image part in one conversation.
async fn get_conversation_image(
    State(state): State<ApiState>,
    Path((conversation_id, asset_id)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let conversation_id = ConversationId::new(conversation_id);
    let asset_id = ImageAssetId::new(asset_id);
    let image = run_store(&state, move |store| {
        store.load_conversation_image_asset(&conversation_id, &asset_id)
    })
    .await?;

    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, image.mime_type)
        .body(axum::body::Body::from(image.bytes))
        .context("failed to build image response")
        .map_err(ApiError::from)
}

/// Removes one conversation and all owned persisted data.
async fn remove_conversation(
    State(state): State<ApiState>,
    Path(conversation_id): Path<String>,
) -> ApiResult<DeletedResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    run_store(&state, move |store| {
        operation::remove_conversation(store, &conversation_id)
    })
    .await?;

    Ok(Json(DeletedResponse { deleted: true }))
}

#[derive(Debug, Deserialize)]
/// Request body for setting the conversation-level system prompt.
struct SystemPromptRequest {
    text: String,
}

#[derive(Debug, Serialize)]
/// Response for system prompt mutation.
struct SystemPromptResponse {
    system_prompt: Option<String>,
}

/// Sets or clears the conversation-level system prompt.
async fn set_system_prompt(
    State(state): State<ApiState>,
    Path(conversation_id): Path<String>,
    Json(request): Json<SystemPromptRequest>,
) -> ApiResult<SystemPromptResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let system_prompt = run_store(&state, move |store| {
        operation::set_system_prompt(store, &conversation_id, &request.text)?;
        store.system_prompt(&conversation_id)
    })
    .await?;

    Ok(Json(SystemPromptResponse { system_prompt }))
}

/// Removes the conversation-level system prompt.
async fn remove_system_prompt(
    State(state): State<ApiState>,
    Path(conversation_id): Path<String>,
) -> ApiResult<SystemPromptResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    run_store(&state, move |store| {
        operation::remove_system_prompt(store, &conversation_id)
    })
    .await?;

    Ok(Json(SystemPromptResponse {
        system_prompt: None,
    }))
}

#[derive(Debug, Deserialize)]
/// Request body for setting the conversation default model.
struct ConversationModelRequest {
    model: String,
}

#[derive(Debug, Serialize)]
/// Response for conversation model mutation.
struct ConversationModelResponse {
    model: String,
}

/// Sets the conversation model used by future queries.
async fn set_conversation_model(
    State(state): State<ApiState>,
    Path(conversation_id): Path<String>,
    Json(request): Json<ConversationModelRequest>,
) -> ApiResult<ConversationModelResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let model = ModelName::new(request.model);
    let model = run_store(&state, move |store| {
        operation::set_conversation_model(store, &conversation_id, &model)?;
        operation::conversation_model(store, &conversation_id)
    })
    .await?;

    Ok(Json(ConversationModelResponse {
        model: model.as_str().to_string(),
    }))
}

#[derive(Debug, Deserialize)]
/// Request body for setting the conversation default reasoning effort.
struct ConversationReasoningRequest {
    effort: Option<String>,
}

#[derive(Debug, Serialize)]
/// Response for conversation reasoning mutation.
struct ConversationReasoningResponse {
    reasoning: Option<ReasoningRequest>,
}

/// Sets the conversation reasoning effort used by future queries.
async fn set_conversation_reasoning(
    State(state): State<ApiState>,
    Path(conversation_id): Path<String>,
    Json(request): Json<ConversationReasoningRequest>,
) -> ApiResult<ConversationReasoningResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let reasoning = run_store(&state, move |store| {
        operation::set_conversation_reasoning_effort(
            store,
            &conversation_id,
            request.effort.as_deref(),
        )
    })
    .await?;

    Ok(Json(ConversationReasoningResponse { reasoning }))
}

/// Deletes descendants after one checkpoint message.
async fn truncate_conversation(
    State(state): State<ApiState>,
    Path(conversation_id): Path<String>,
    Json(request): Json<MessageIdRequest>,
) -> ApiResult<ActiveMessageResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let message_id = MessageId::new(request.message_id);
    let active_message_id = run_store(&state, move |store| {
        operation::truncate_conversation(store, &conversation_id, &message_id)?;
        store.active_message_id(&conversation_id)
    })
    .await?;

    Ok(Json(ActiveMessageResponse {
        active_message_id: active_message_id
            .map(|id| id.as_str().to_string())
            .unwrap_or_default(),
    }))
}

/// Creates a new conversation copied through a checkpoint message.
async fn fork_conversation(
    State(state): State<ApiState>,
    Path(conversation_id): Path<String>,
    Json(request): Json<MessageIdRequest>,
) -> ApiResult<ConversationIdResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let message_id = MessageId::new(request.message_id);
    let forked_conversation_id = run_store(&state, move |store| {
        operation::fork_conversation(store, &conversation_id, &message_id)
    })
    .await?;

    Ok(Json(ConversationIdResponse {
        conversation_id: forked_conversation_id.as_str().to_string(),
    }))
}

#[derive(Debug, Deserialize)]
/// Request body for counting the current model-facing input tokens.
struct InputTokensRequest {
    model: Option<String>,
}

#[derive(Debug, Serialize)]
/// Response body for a read-only input-token count.
struct InputTokensResponse {
    input_tokens: Option<u64>,
    total_tokens: Option<u64>,
    model: Option<String>,
    source: Option<String>,
    raw: Option<Value>,
}

impl InputTokensResponse {
    /// Builds the API shape while preserving the count source computed before
    /// the async Bifrost request.
    fn from_count_result(
        result: operation::InputTokenCountResult,
        model: &ModelName,
        source: Option<String>,
    ) -> Self {
        match result {
            operation::InputTokenCountResult::Count(count) => Self {
                input_tokens: Some(count.input_tokens),
                total_tokens: count.total_tokens,
                model: count.model,
                source,
                raw: Some(count.raw),
            },
            operation::InputTokenCountResult::Unsupported => Self::unavailable(model),
            operation::InputTokenCountResult::EmptyContext => Self {
                input_tokens: None,
                total_tokens: None,
                model: None,
                source: None,
                raw: None,
            },
        }
    }

    /// Builds a successful response for providers that do not support
    /// pre-query input-token counting.
    fn unavailable(model: &ModelName) -> Self {
        Self {
            input_tokens: None,
            total_tokens: None,
            model: Some(model.as_str().to_string()),
            source: Some("unavailable".to_string()),
            raw: None,
        }
    }
}

/// Counts current model-facing input tokens without mutating conversation state.
async fn count_input_tokens(
    State(state): State<ApiState>,
    Path(conversation_id): Path<String>,
    Json(request): Json<InputTokensRequest>,
) -> ApiResult<InputTokensResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let model_override = request.model.map(ModelName::new);
    let (model, context, source) = run_store(&state, move |store| {
        let model = operation::resolve_conversation_model(store, &conversation_id, model_override)?;
        let context = operation::conversation_input_token_context(store, &conversation_id)?;
        let source = context
            .as_ref()
            .map(|context| context.source().as_str().to_string());
        Ok((model, context, source))
    })
    .await?;
    let result = operation::count_input_tokens_for_context(
        GatewayUrl::new(state.gateway_url),
        BaseUrl::new(state.base_url),
        &model,
        context,
    )
    .await?;

    Ok(Json(InputTokensResponse::from_count_result(
        result, &model, source,
    )))
}

/// Runs one model query against the current active path.
async fn query(
    axum::extract::State(state): axum::extract::State<ApiState>,
    Path(conversation_id): Path<String>,
    Json(request): Json<QueryRequest>,
) -> ApiResult<MessageResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let manager = state.run_manager.clone();
    let task_conversation_id = conversation_id.clone();
    let message = manager
        .execute_action(
            &conversation_id,
            RuntimeRunAction::Query,
            move |run_id, cancellation| async move {
                let mut store = open_store(&state)?;
                let runtime = operation::RuntimeTurnConfig::new(
                    &run_id,
                    cancellation,
                    GatewayUrl::new(state.gateway_url.clone()),
                    BaseUrl::new(state.base_url.clone()),
                    request.model_override(),
                    request.reasoning(),
                    state.tool_registry.as_ref(),
                );
                operation::query_conversation_with_registry(
                    &ApiOutput,
                    &mut store,
                    &task_conversation_id,
                    runtime,
                )
                .await
            },
        )
        .await?;

    Ok(Json(MessageResponse::from_message(message)))
}

/// deltas have nowhere to go and are intentionally dropped here.
struct ApiOutput;

impl RuntimeOutput for ApiOutput {
    fn start_assistant_message(&self) {}

    fn assistant_delta(&self, _text: &str) -> Result<()> {
        Ok(())
    }

    fn end_assistant_message(&self) {}

    fn assistant_tool_calls(&self, _tool_calls: &[ToolCall]) {}
}

#[derive(Debug, Serialize)]
/// Serializable message response for query results.
struct MessageResponse {
    id: Option<String>,
    parent_message_id: Option<String>,
    role: Role,
    content: String,
    parts: Vec<MessagePartResponse>,
    metadata: Option<MessageMetadata>,
}

impl MessageResponse {
    /// Converts an internal message into a JSON response without raw image bytes.
    fn from_message(message: Message) -> Self {
        Self {
            id: message.id.map(|id| id.as_str().to_string()),
            parent_message_id: message.parent_message_id.map(|id| id.as_str().to_string()),
            role: message.role,
            content: message.content,
            parts: message
                .parts
                .into_iter()
                .map(MessagePartResponse::from_part)
                .collect(),
            metadata: message.metadata,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
/// Serializable message part that hides image bytes.
enum MessagePartResponse {
    Text {
        text: String,
    },
    Image {
        asset_id: String,
        mime_type: String,
        byte_count: usize,
    },
}

impl MessagePartResponse {
    /// Converts one stored part into an API-safe payload.
    fn from_part(part: MessagePart) -> Self {
        match part {
            MessagePart::Text(text) => Self::Text { text },
            MessagePart::Image(image) => Self::Image {
                asset_id: image.asset_id.as_str().to_string(),
                mime_type: image.mime_type,
                byte_count: image.bytes.len(),
            },
        }
    }
}
