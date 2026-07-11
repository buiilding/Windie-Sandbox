//! Gateway health, lifecycle, model discovery, and model metadata routes.

use super::{
    ApiResult, ApiState, BaseUrl, Deserialize, GatewayUrl, Json, ModelInfo, ModelName, Query,
    Router, Serialize, get, operation, post,
};

pub(super) fn routes() -> Router<ApiState> {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/status", get(status))
        .route("/api/models", get(list_models))
        .route("/api/model-parameters", get(model_parameters))
        .route("/api/gateway/start", post(start_gateway))
        .route("/api/gateway/stop", post(stop_gateway))
}

#[derive(Debug, Serialize)]
/// Health payload for UI startup checks.
struct HealthResponse {
    ok: bool,
}

/// Confirms that the API server process is reachable.
async fn health() -> ApiResult<HealthResponse> {
    Ok(Json(HealthResponse { ok: true }))
}

#[derive(Debug, Serialize)]
/// Local runtime readiness as seen from the API process.
struct StatusResponse {
    gateway_running: bool,
}

/// Returns current local gateway readiness.
async fn status(
    axum::extract::State(state): axum::extract::State<ApiState>,
) -> ApiResult<StatusResponse> {
    Ok(Json(StatusResponse {
        gateway_running: operation::gateway_status(GatewayUrl::new(state.gateway_url)).await,
    }))
}

#[derive(Debug, Serialize)]
/// Models returned by the configured local Bifrost gateway.
struct ModelListResponse {
    models: Vec<ModelResponse>,
}

#[derive(Debug, Serialize)]
/// One provider-qualified model id available through Bifrost.
struct ModelResponse {
    id: String,
    context_length: Option<u64>,
    max_input_tokens: Option<u64>,
    max_output_tokens: Option<u64>,
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
async fn list_models(
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
struct ModelParametersQuery {
    model: String,
}

/// Loads normalized Bifrost model-parameter metadata for one selected model.
async fn model_parameters(
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
enum GatewayStartResponse {
    AlreadyRunning,
    Started,
}

/// Starts the configured local Bifrost gateway when possible.
async fn start_gateway(
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
enum GatewayStopResponse {
    NotRunning,
    Stopped,
}

/// Stops the configured local Bifrost gateway when Windie can identify it.
async fn stop_gateway(
    axum::extract::State(state): axum::extract::State<ApiState>,
) -> ApiResult<GatewayStopResponse> {
    let status = operation::stop_gateway(GatewayUrl::new(state.gateway_url)).await?;
    let response = match status {
        crate::gateway::GatewayStop::NotRunning => GatewayStopResponse::NotRunning,
        crate::gateway::GatewayStop::Stopped => GatewayStopResponse::Stopped,
    };

    Ok(Json(response))
}
