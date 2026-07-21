//! Bifrost gateway, model, and input-token API handlers.

use super::*;

#[derive(Debug, Serialize)]
/// Models returned by the configured local Bifrost gateway.
pub(super) struct ModelListResponse {
    pub(super) models: Vec<ModelResponse>,
}

#[derive(Debug, Serialize)]
/// One provider-qualified model id available through Bifrost.
pub(super) struct ModelResponse {
    pub(super) id: String,
    pub(super) context_length: Option<u64>,
    pub(super) max_input_tokens: Option<u64>,
    pub(super) max_output_tokens: Option<u64>,
}

impl From<ModelInfo> for ModelResponse {
    fn from(model: ModelInfo) -> Self {
        Self {
            id: model.id,
            context_length: model.context_length,
            max_input_tokens: model.max_input_tokens,
            max_output_tokens: model.max_output_tokens,
        }
    }
}

/// Lists models reported by the running gateway for API clients.
pub(super) async fn list_models(
    axum::extract::State(state): axum::extract::State<ApiState>,
) -> ApiResult<ModelListResponse> {
    let models = operation::list_models(
        GatewayUrl::new(state.gateway_url),
        BaseUrl::new(state.base_url),
    )
    .await?;

    Ok(Json(ModelListResponse {
        models: models.into_iter().map(ModelResponse::from).collect(),
    }))
}

#[derive(Debug, Deserialize)]
/// Query parameters for model-parameter metadata lookup.
pub(super) struct ModelParametersQuery {
    pub(super) model: String,
}

/// Loads normalized Bifrost model-parameter metadata for one selected model.
pub(super) async fn model_parameters(
    axum::extract::State(state): axum::extract::State<ApiState>,
    Query(query): Query<ModelParametersQuery>,
) -> ApiResult<operation::ModelRuntimeParameters> {
    let model = ModelName::new(query.model);
    let parameters = operation::model_runtime_parameters(
        GatewayUrl::new(state.gateway_url),
        BaseUrl::new(state.base_url),
        &model,
    )
    .await?;

    Ok(Json(parameters))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
/// Result of a gateway start request.
pub(super) enum GatewayStartResponse {
    AlreadyRunning,
    Started,
}

/// Starts the configured local Bifrost gateway when possible.
pub(super) async fn start_gateway(
    axum::extract::State(state): axum::extract::State<ApiState>,
) -> ApiResult<GatewayStartResponse> {
    let status = operation::start_gateway(GatewayUrl::new(state.gateway_url)).await?;
    let response = match status {
        crate::gateway::GatewayStart::AlreadyRunning => GatewayStartResponse::AlreadyRunning,
        crate::gateway::GatewayStart::Started => GatewayStartResponse::Started,
    };

    Ok(Json(response))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
/// Result of a gateway stop request.
pub(super) enum GatewayStopResponse {
    NotRunning,
    Stopped,
}

/// Stops the configured local Bifrost gateway when Windie can identify it.
pub(super) async fn stop_gateway(
    axum::extract::State(state): axum::extract::State<ApiState>,
) -> ApiResult<GatewayStopResponse> {
    let status = operation::stop_gateway(GatewayUrl::new(state.gateway_url)).await?;
    let response = match status {
        crate::gateway::GatewayStop::NotRunning => GatewayStopResponse::NotRunning,
        crate::gateway::GatewayStop::Stopped => GatewayStopResponse::Stopped,
    };

    Ok(Json(response))
}

#[derive(Debug, Deserialize)]
/// Request body for counting the current model-facing input tokens.
pub(super) struct InputTokensRequest {
    pub(super) model: Option<String>,
    pub(super) head_message_id: Option<String>,
}

#[derive(Debug, Serialize)]
/// Response body for a read-only input-token count.
pub(super) struct InputTokensResponse {
    pub(super) input_tokens: Option<u64>,
    pub(super) total_tokens: Option<u64>,
    pub(super) model: Option<String>,
    pub(super) source: Option<String>,
    pub(super) raw: Option<Value>,
}

impl InputTokensResponse {
    /// Builds the API shape while preserving the count source computed before
    /// the async Bifrost request.
    fn from_count(count: Option<InputTokenCount>, source: Option<String>) -> Self {
        match count {
            Some(count) => Self {
                input_tokens: Some(count.input_tokens),
                total_tokens: count.total_tokens,
                model: count.model,
                source,
                raw: Some(count.raw),
            },
            None => Self {
                input_tokens: None,
                total_tokens: None,
                model: None,
                source,
                raw: None,
            },
        }
    }
}

/// Counts current model-facing input tokens without mutating conversation state.
pub(super) async fn count_input_tokens(
    State(state): State<ApiState>,
    Path(conversation_id): Path<String>,
    Json(request): Json<InputTokensRequest>,
) -> ApiResult<InputTokensResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let store = open_store(&state)?;
    let model = operation::resolve_conversation_model(
        &store,
        &conversation_id,
        request.model.map(ModelName::new),
    )?;
    let head_message_id = request.head_message_id.map(MessageId::new);
    let context = operation::conversation_input_token_context(
        &store,
        &conversation_id,
        head_message_id.as_ref(),
    )?;
    let had_context = context.is_some();
    let source = context
        .as_ref()
        .map(|context| context.source().as_str().to_string());
    drop(store);
    let count = operation::count_input_tokens_for_context(
        GatewayUrl::new(state.gateway_url),
        BaseUrl::new(state.base_url),
        &model,
        context,
    )
    .await?;

    let source = if count.is_none() && had_context {
        Some("unsupported".to_string())
    } else {
        source
    };

    Ok(Json(InputTokensResponse::from_count(count, source)))
}
