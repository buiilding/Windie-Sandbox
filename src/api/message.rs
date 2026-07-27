//! Message and system-prompt API handlers.

use super::*;

#[derive(Debug, Deserialize)]
/// Request body for operations that target a message.
pub(super) struct MessageIdRequest {
    pub(super) message_id: String,
}

#[derive(Debug, Deserialize)]
/// Request body for inserting one message.
pub(super) struct InsertMessageRequest {
    pub(super) head_message_id: Option<String>,
    pub(super) role: Role,
    pub(super) text: Option<String>,
    #[serde(default)]
    pub(super) parts: Vec<InsertMessagePart>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
/// One ordered API message part to persist.
pub(super) enum InsertMessagePart {
    Text { text: String },
    Image { path: PathBuf },
    ImageData { mime_type: String, data: String },
}

#[derive(Debug, Serialize)]
/// ID response for operations that create a message.
pub(super) struct MessageIdResponse {
    pub(super) message_id: String,
}

/// Inserts one message under the requested head.
pub(super) async fn insert_message(
    State(state): State<ApiState>,
    Path(conversation_id): Path<String>,
    Json(request): Json<InsertMessageRequest>,
) -> ApiResult<MessageIdResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let head_message_id = requested_head_message_id(request.head_message_id);
    let parts = normalize_insert_parts(request.text, request.parts)?;
    let mut store = open_store(&state)?;
    let message_id = operation::insert_message(
        &mut store,
        &conversation_id,
        head_message_id.as_ref(),
        request.role,
        &parts,
    )?;

    Ok(Json(MessageIdResponse {
        message_id: message_id.as_str().to_string(),
    }))
}

/// Converts request text and part fields into one ordered part list.
pub(super) fn normalize_insert_parts(
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
        return Err(windie_error::invalid_request(
            "message requires text or parts",
        ));
    }

    let normalized = parts
        .into_iter()
        .map(|part| match part {
            InsertMessagePart::Text { text } => Ok(MessageInputPart::Text(text)),
            InsertMessagePart::Image { path } => Ok(MessageInputPart::ImagePath(path)),
            InsertMessagePart::ImageData { mime_type, data } => {
                let bytes = STANDARD.decode(data).map_err(|_| {
                    windie_error::invalid_request("image_data must contain valid base64 data")
                })?;
                Ok(MessageInputPart::ImageBytes { mime_type, bytes })
            }
        })
        .collect::<Result<Vec<_>>>()?;

    if normalized
        .iter()
        .all(|part| matches!(part, MessageInputPart::Text(text) if text.is_empty()))
    {
        return Err(windie_error::invalid_request(
            "message requires non-empty text or an image",
        ));
    }

    Ok(normalized)
}

#[derive(Debug, Deserialize)]
/// Request body for replacing one message.
pub(super) struct UpdateMessageRequest {
    pub(super) text: String,
}

/// Replaces one message's text content.
pub(super) async fn update_message(
    State(state): State<ApiState>,
    Path((conversation_id, message_id)): Path<(String, String)>,
    Json(request): Json<UpdateMessageRequest>,
) -> ApiResult<MessageIdResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let message_id = MessageId::new(message_id);
    let mut store = open_store(&state)?;

    operation::update_message(&mut store, &conversation_id, &message_id, &request.text)?;

    Ok(Json(MessageIdResponse {
        message_id: message_id.as_str().to_string(),
    }))
}

/// Splices one message out of the conversation tree.
pub(super) async fn remove_message(
    State(state): State<ApiState>,
    Path((conversation_id, message_id)): Path<(String, String)>,
) -> ApiResult<DeletedResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let message_id = MessageId::new(message_id);
    let mut store = open_store(&state)?;

    operation::remove_message(&mut store, &conversation_id, &message_id)?;

    Ok(Json(DeletedResponse { deleted: true }))
}

/// Returns durable image bytes for an image part in one conversation.
pub(super) async fn get_conversation_image(
    State(state): State<ApiState>,
    Path((conversation_id, asset_id)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let conversation_id = ConversationId::new(conversation_id);
    let asset_id = ImageAssetId::new(asset_id);
    let store = open_store(&state)?;
    let image = store.load_conversation_image_asset(&conversation_id, &asset_id)?;

    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, image.mime_type)
        .body(axum::body::Body::from(image.bytes))
        .context("failed to build image response")
        .map_err(ApiError::from)
}

#[derive(Debug, Deserialize)]
/// Request body for setting the conversation-wide system prompt (tree-wide).
pub(super) struct SystemPromptRequest {
    pub(super) text: String,
}

#[derive(Debug, Serialize)]
/// Response for system prompt mutation (tree-wide).
pub(super) struct SystemPromptResponse {
    pub(super) system_prompt: Option<String>,
}

/// Converts an optional API head string into a typed message ID.
pub(super) fn requested_head_message_id(head_message_id: Option<String>) -> Option<MessageId> {
    head_message_id.map(MessageId::new)
}

/// Sets or clears the conversation-wide system prompt.
pub(super) async fn set_system_prompt(
    State(state): State<ApiState>,
    Path(conversation_id): Path<String>,
    Json(request): Json<SystemPromptRequest>,
) -> ApiResult<SystemPromptResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let mut store = open_store(&state)?;

    operation::set_system_prompt(&mut store, &conversation_id, &request.text)?;

    Ok(Json(SystemPromptResponse {
        system_prompt: store.system_prompt(&conversation_id)?,
    }))
}

/// Removes the conversation-wide system prompt.
pub(super) async fn remove_system_prompt(
    State(state): State<ApiState>,
    Path(conversation_id): Path<String>,
) -> ApiResult<SystemPromptResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let mut store = open_store(&state)?;

    operation::remove_system_prompt(&mut store, &conversation_id)?;

    Ok(Json(SystemPromptResponse {
        system_prompt: None,
    }))
}
