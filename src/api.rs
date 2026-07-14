//! Local developer API server.
//!
//! This module exposes Windie's existing runtime and store primitives over a
//! localhost-only JSON API. It is a test harness boundary for clients such as
//! `windie-inspector`; persistence, context construction, gateway checks, and
//! model requests still flow through the same modules used by the CLI.

use std::collections::VecDeque;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use axum::extract::{DefaultBodyLimit, Path, Query, Request, State};
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderName, HeaderValue, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use futures_util::stream;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::net::TcpListener;
use tower_http::cors::CorsLayer;

use crate::conversation::{
    ConversationId, ImageAssetId, MessageId, Role, ToolCallId, ToolSchema, ToolSchemaName,
};
use crate::error::{self, WindieErrorKind};
use crate::gateway::GatewayUrl;
use crate::llm::{BaseUrl, InputTokenCount, ModelInfo, ModelName, ReasoningRequest};
use crate::operation::{self, InspectionReport, MessageInputPart};
use crate::output::TerminalOutput;
use crate::session::{Session, SessionEventRecord, SessionId, SessionStatus};
use crate::session_manager::{SessionManager, SessionSubscription};
use crate::setup;
use crate::store::{ConversationInfo, Store};
use crate::tool::{ProviderToolName, ToolApprovalMode, ToolDefinition, ToolProviderId};
use crate::tool_provider::{ToolProviderRegistry, ToolProviderStatus};
use crate::wakeup::ContinueWakeup;

const API_TOKEN_HEADER: &str = "x-windie-api-token";
/// Maximum JSON request body accepted by the localhost API.
///
/// The default Axum body limit is too small for clipboard or local image data
/// sent as base64 message parts. This keeps image input practical while staying
/// bounded for a local developer harness.
const API_JSON_BODY_LIMIT_BYTES: usize = 32 * 1024 * 1024;

/// Sessions the local developer API server until the process is stopped.
pub async fn serve(
    address: SocketAddr,
    gateway_url: &str,
    base_url: &str,
    model: &str,
) -> Result<()> {
    let output = TerminalOutput;
    let gateway_start = operation::start_gateway(GatewayUrl::new(gateway_url)).await?;
    match gateway_start {
        crate::gateway::GatewayStart::AlreadyRunning => output.gateway_already_running(),
        crate::gateway::GatewayStart::Started => output.gateway_started(),
    };

    let api_token = match std::env::var("WINDIE_API_TOKEN") {
        Ok(token) => token,
        Err(_) => setup::ensure_api_token()?,
    };
    let tool_registry = Arc::new(ToolProviderRegistry::with_persistent_mcp_sessions());
    let session_manager = Arc::new(SessionManager::new(
        None,
        gateway_url.to_string(),
        base_url.to_string(),
        tool_registry.clone(),
    ));
    let state = ApiState {
        gateway_url: gateway_url.to_string(),
        base_url: base_url.to_string(),
        model: model.to_string(),
        api_token,
        store_path: None,
        tool_registry,
        session_manager,
    };
    let listener = TcpListener::bind(address)
        .await
        .with_context(|| format!("failed to bind API server at {address}"))?;

    output.api_started(&address, &state.api_token);
    let server_result = axum::serve(listener, router(state))
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("api server failed");

    if gateway_start == crate::gateway::GatewayStart::Started {
        match operation::stop_gateway(GatewayUrl::new(gateway_url)).await {
            Ok(crate::gateway::GatewayStop::NotRunning) => output.gateway_not_running(),
            Ok(crate::gateway::GatewayStop::Stopped) => output.gateway_stopped(),
            Err(error) => eprintln!("failed to stop Bifrost gateway: {error}"),
        }
    }

    server_result
}

/// Waits for the process-level shutdown signal used by the API server.
///
/// The gateway cleanup happens after Axum drains the listener, so Ctrl-C or a
/// normal terminate signal stops both the API and the Bifrost process the API
/// started.
async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(error) = tokio::signal::ctrl_c().await {
            eprintln!("failed to install Ctrl-C handler: {error}");
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(error) => {
                eprintln!("failed to install terminate signal handler: {error}");
                std::future::pending::<()>().await;
            }
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}

/// Builds the route table for the local API surface.
///
/// Handlers translate HTTP requests into shared operations and map returned
/// values into JSON responses. The router only owns HTTP mapping.
fn router(state: ApiState) -> Router {
    let cors = CorsLayer::new()
        .allow_origin([
            HeaderValue::from_static("http://localhost:3000"),
            HeaderValue::from_static("http://127.0.0.1:3000"),
        ])
        .allow_methods([Method::GET, Method::POST, Method::PATCH, Method::DELETE])
        .allow_headers([CONTENT_TYPE, HeaderName::from_static(API_TOKEN_HEADER)]);

    Router::new()
        .route("/api/health", get(health))
        .route("/api/status", get(status))
        .route("/api/models", get(list_models))
        .route("/api/model-parameters", get(model_parameters))
        .route("/api/tools", get(list_tools))
        .route("/api/tools/{provider_id}", get(list_provider_tools))
        .route("/api/gateway/start", post(start_gateway))
        .route("/api/gateway/stop", post(stop_gateway))
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
            "/api/conversations/{conversation_id}/tool-approval-mode",
            patch(set_tool_approval_mode),
        )
        .route(
            "/api/conversations/{conversation_id}/tool-schemas",
            post(insert_tool_schema),
        )
        .route(
            "/api/conversations/{conversation_id}/tool-schemas/{name}",
            patch(update_tool_schema).delete(remove_tool_schema),
        )
        .route(
            "/api/conversations/{conversation_id}/tools",
            post(attach_tool),
        )
        .route(
            "/api/conversations/{conversation_id}/tools/batch",
            post(attach_tools),
        )
        .route(
            "/api/conversations/{conversation_id}/tools/{schema_name}",
            axum::routing::delete(detach_tool),
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
            "/api/conversations/{conversation_id}/run-approvals",
            get(list_conversation_session_approvals),
        )
        .route(
            "/api/conversations/{conversation_id}/sessions",
            post(create_session),
        )
        .route(
            "/api/conversations/{conversation_id}/wakeups/continue",
            post(create_session),
        )
        .route(
            "/api/conversations/{conversation_id}/wakeups/query",
            post(create_session),
        )
        .route("/api/sessions", get(list_sessions))
        .route("/api/sessions/{session_id}", get(get_run))
        .route(
            "/api/sessions/{session_id}/approvals",
            get(list_session_approvals),
        )
        .route("/api/sessions/{session_id}/events", get(session_events))
        .route("/api/sessions/{session_id}/stop", post(stop_run))
        .route(
            "/api/sessions/{session_id}/approvals/{tool_call_id}/approve",
            post(approve_session_tool),
        )
        .route(
            "/api/sessions/{session_id}/approvals/{tool_call_id}/deny",
            post(deny_session_tool),
        )
        .route(
            "/api/conversations/{conversation_id}/runs",
            post(create_session),
        )
        .route("/api/runs", get(list_sessions))
        .route("/api/runs/{session_id}", get(get_run))
        .route(
            "/api/runs/{session_id}/approvals",
            get(list_session_approvals),
        )
        .route("/api/runs/{session_id}/events", get(session_events))
        .route("/api/runs/{session_id}/stop", post(stop_run))
        .route(
            "/api/runs/{session_id}/approvals/{tool_call_id}/approve",
            post(approve_session_tool),
        )
        .route(
            "/api/runs/{session_id}/approvals/{tool_call_id}/deny",
            post(deny_session_tool),
        )
        .route(
            "/api/conversations/{conversation_id}/input-tokens",
            post(count_input_tokens),
        )
        .layer(middleware::from_fn_with_state(
            state.clone(),
            require_api_token,
        ))
        .layer(DefaultBodyLimit::max(API_JSON_BODY_LIMIT_BYTES))
        .layer(cors)
        .with_state(state)
}

#[derive(Clone)]
/// Runtime settings captured by the API server at startup.
struct ApiState {
    gateway_url: String,
    base_url: String,
    model: String,
    api_token: String,
    store_path: Option<PathBuf>,
    tool_registry: Arc<ToolProviderRegistry>,
    session_manager: Arc<SessionManager>,
}

/// Opens the production store, or a test-scoped store when route tests inject
/// one through `ApiState`.
fn open_store(state: &ApiState) -> Result<Store> {
    match state.store_path.as_ref() {
        Some(path) => Store::open_at(path),
        None => Store::open(),
    }
}

#[derive(Debug, Serialize)]
/// Stable error response returned by failed API operations.
struct ErrorResponse {
    error: String,
    causes: Vec<String>,
}

/// Error wrapper that maps Windie failures into JSON HTTP responses.
struct ApiError(anyhow::Error);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        log_api_error(&self.0);

        let causes = error_causes(&self.0);
        let message = raw_error_message(&self.0);
        let status = match error::kind_from_error(&self.0) {
            Some(WindieErrorKind::NotFound) => StatusCode::NOT_FOUND,
            Some(WindieErrorKind::InvalidRequest) => StatusCode::BAD_REQUEST,
            None => StatusCode::INTERNAL_SERVER_ERROR,
        };

        (
            status,
            Json(ErrorResponse {
                error: message,
                causes,
            }),
        )
            .into_response()
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(error: anyhow::Error) -> Self {
        Self(error)
    }
}

type ApiResult<T> = std::result::Result<Json<T>, ApiError>;

/// Prints one API error chain to stderr for local developer visibility.
fn log_api_error(error: &anyhow::Error) {
    eprintln!("api error:");
    for cause in error.chain() {
        eprintln!("  {cause}");
    }
}

/// Returns the root cause text that clients should display first.
fn raw_error_message(error: &anyhow::Error) -> String {
    error
        .chain()
        .last()
        .map(ToString::to_string)
        .unwrap_or_else(|| error.to_string())
}

/// Returns the full context chain from outer boundary to root cause.
fn error_causes(error: &anyhow::Error) -> Vec<String> {
    error.chain().map(ToString::to_string).collect()
}

/// Requires the current local API token before executing non-health requests.
///
/// The browser UI sends this token in `X-Windie-Api-Token`. Preflight requests
/// and health checks stay open so clients can detect that the server exists
/// before they have a token configured.
async fn require_api_token(
    State(state): State<ApiState>,
    request: Request,
    next: Next,
) -> Response {
    if request.method() == Method::OPTIONS || request.uri().path() == "/api/health" {
        return next.run(request).await;
    }

    let provided = request
        .headers()
        .get(API_TOKEN_HEADER)
        .and_then(|value| value.to_str().ok());
    if provided != Some(state.api_token.as_str()) {
        eprintln!("api error:");
        eprintln!("  missing or invalid Windie API token");

        return (
            StatusCode::UNAUTHORIZED,
            Json(ErrorResponse {
                error: "missing or invalid Windie API token".to_string(),
                causes: vec!["missing or invalid Windie API token".to_string()],
            }),
        )
            .into_response();
    }

    next.run(request).await
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
/// API response for provider tools available to attach.
struct ToolCatalogResponse {
    tools: Vec<ToolDefinition>,
    providers: Vec<ToolProviderStatusResponse>,
}

#[derive(Debug, Serialize)]
/// Availability status for one approved tool provider.
struct ToolProviderStatusResponse {
    provider_id: String,
    display_name: String,
    available: bool,
    tool_count: usize,
    error: Option<String>,
}

impl ToolProviderStatusResponse {
    fn from_status(status: ToolProviderStatus) -> Self {
        Self {
            provider_id: status.provider_id.as_str().to_string(),
            display_name: status.display_name,
            available: status.available,
            tool_count: status.tool_count,
            error: status.error,
        }
    }
}

/// Lists provider tools clients may attach to conversations.
async fn list_tools(State(state): State<ApiState>) -> ApiResult<ToolCatalogResponse> {
    Ok(Json(ToolCatalogResponse {
        tools: operation::available_tools_with_registry(&state.tool_registry)?,
        providers: state
            .tool_registry
            .list_provider_statuses()
            .into_iter()
            .map(ToolProviderStatusResponse::from_status)
            .collect(),
    }))
}

/// Lists available tools for one provider.
async fn list_provider_tools(
    State(state): State<ApiState>,
    Path(provider_id): Path<String>,
) -> ApiResult<ToolCatalogResponse> {
    let provider_id = ToolProviderId::new(provider_id);

    Ok(Json(ToolCatalogResponse {
        tools: operation::available_provider_tools_with_registry(
            &state.tool_registry,
            &provider_id,
        )?,
        providers: state
            .tool_registry
            .list_provider_statuses()
            .into_iter()
            .filter(|status| status.provider_id == provider_id)
            .map(ToolProviderStatusResponse::from_status)
            .collect(),
    }))
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

#[derive(Debug, Serialize)]
/// API response for the persisted conversation list.
struct ConversationListResponse {
    conversations: Vec<ConversationSummary>,
}

#[derive(Debug, Serialize)]
/// One persisted conversation row used by UI sidebars.
struct ConversationSummary {
    id: String,
    title: Option<String>,
    model: String,
    message_count: i64,
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
async fn list_conversations(State(state): State<ApiState>) -> ApiResult<ConversationListResponse> {
    let store = open_store(&state)?;
    let conversations = operation::list_conversations(&store)?
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
    let store = open_store(&state)?;
    let conversation_id = operation::create_conversation(&store, &ModelName::new(state.model))?;

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
    let store = open_store(&state)?;
    let model_override = query.model.clone().map(ModelName::new);
    let report = operation::inspect_conversation(&store, &conversation_id, model_override)?;

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
    let mut store = open_store(&state)?;

    operation::activate_message(&mut store, &conversation_id, &message_id)?;

    Ok(Json(ActiveMessageResponse {
        active_message_id: message_id.as_str().to_string(),
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
    let mut store = open_store(&state)?;
    let message_id = operation::insert_message(&mut store, &conversation_id, request.role, &parts)?;

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
    let mut store = open_store(&state)?;

    operation::update_message(&mut store, &conversation_id, &message_id, &request.text)?;

    Ok(Json(MessageIdResponse {
        message_id: message_id.as_str().to_string(),
    }))
}

/// Splices one message out of the conversation tree.
async fn remove_message(
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
async fn get_conversation_image(
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

#[derive(Debug, Serialize)]
/// Generic deletion response.
struct DeletedResponse {
    deleted: bool,
}

/// Removes one conversation and all owned persisted data.
async fn remove_conversation(
    State(state): State<ApiState>,
    Path(conversation_id): Path<String>,
) -> ApiResult<DeletedResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let mut store = open_store(&state)?;

    operation::remove_conversation(&mut store, &conversation_id)?;

    Ok(Json(DeletedResponse { deleted: true }))
}

#[derive(Debug, Deserialize)]
/// Request body for setting a system prompt on the active or requested path.
struct SystemPromptRequest {
    text: String,
    head_message_id: Option<String>,
}

#[derive(Debug, Serialize)]
/// Response for system prompt mutation.
struct SystemPromptResponse {
    system_prompt: Option<String>,
}

#[derive(Debug, Deserialize)]
/// Optional query parameters for path-scoped context mutations.
struct ContextMutationQuery {
    head_message_id: Option<String>,
}

/// Converts an optional API head string into a typed message ID.
fn requested_head_message_id(head_message_id: Option<String>) -> Option<MessageId> {
    head_message_id.map(MessageId::new)
}

/// Loads the effective prompt for a requested head, or the active path when no
/// explicit head was requested.
fn system_prompt_response(
    store: &Store,
    conversation_id: &ConversationId,
    head_message_id: Option<&MessageId>,
) -> Result<Option<String>> {
    match head_message_id {
        Some(head_message_id) => {
            store.effective_system_prompt_for_head(conversation_id, Some(head_message_id))
        }
        None => store.system_prompt(conversation_id),
    }
}

/// Sets or clears the system prompt on the active or requested path.
async fn set_system_prompt(
    State(state): State<ApiState>,
    Path(conversation_id): Path<String>,
    Json(request): Json<SystemPromptRequest>,
) -> ApiResult<SystemPromptResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let mut store = open_store(&state)?;

    let head_message_id = requested_head_message_id(request.head_message_id);
    match head_message_id.as_ref() {
        Some(head_message_id) => operation::set_system_prompt_at_head(
            &mut store,
            &conversation_id,
            Some(head_message_id),
            &request.text,
        )?,
        None => operation::set_system_prompt(&mut store, &conversation_id, &request.text)?,
    }

    Ok(Json(SystemPromptResponse {
        system_prompt: system_prompt_response(&store, &conversation_id, head_message_id.as_ref())?,
    }))
}

/// Removes the system prompt on the active or requested path.
async fn remove_system_prompt(
    State(state): State<ApiState>,
    Path(conversation_id): Path<String>,
    Query(query): Query<ContextMutationQuery>,
) -> ApiResult<SystemPromptResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let mut store = open_store(&state)?;

    let head_message_id = requested_head_message_id(query.head_message_id);
    match head_message_id.as_ref() {
        Some(head_message_id) => operation::remove_system_prompt_at_head(
            &mut store,
            &conversation_id,
            Some(head_message_id),
        )?,
        None => operation::remove_system_prompt(&mut store, &conversation_id)?,
    }

    Ok(Json(SystemPromptResponse {
        system_prompt: system_prompt_response(&store, &conversation_id, head_message_id.as_ref())?,
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
    let mut store = open_store(&state)?;
    let reasoning = operation::set_conversation_reasoning_effort(
        &mut store,
        &conversation_id,
        request.effort.as_deref(),
    )?;

    Ok(Json(ConversationReasoningResponse { reasoning }))
}

#[derive(Debug, Deserialize)]
/// Request body for setting the conversation-level tool approval mode.
struct ToolApprovalModeRequest {
    mode: ToolApprovalMode,
}

#[derive(Debug, Serialize)]
/// Response for tool approval mode mutation.
struct ToolApprovalModeResponse {
    tool_approval_mode: ToolApprovalMode,
}

/// Sets the conversation default for attached tool approvals.
async fn set_tool_approval_mode(
    State(state): State<ApiState>,
    Path(conversation_id): Path<String>,
    Json(request): Json<ToolApprovalModeRequest>,
) -> ApiResult<ToolApprovalModeResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let mut store = open_store(&state)?;

    operation::set_tool_approval_mode(&mut store, &conversation_id, request.mode)?;
    drop(store);
    state
        .session_manager
        .resume_waiting_for_conversation(&conversation_id)?;
    let store = open_store(&state)?;

    Ok(Json(ToolApprovalModeResponse {
        tool_approval_mode: store.tool_approval_mode(&conversation_id)?,
    }))
}

#[derive(Debug, Deserialize)]
/// Request body for creating or updating a tool schema.
struct ToolSchemaRequest {
    name: String,
    description: String,
    parameters: Value,
    head_message_id: Option<String>,
}

impl ToolSchemaRequest {
    /// Converts API JSON into the typed tool schema contract.
    fn into_parts(self) -> (ToolSchema, Option<MessageId>) {
        (
            ToolSchema {
                name: ToolSchemaName::new(self.name),
                description: self.description,
                parameters: self.parameters,
            },
            requested_head_message_id(self.head_message_id),
        )
    }
}

#[derive(Debug, Serialize)]
/// Response for tool schema mutations.
struct ToolSchemaResponse {
    name: String,
}

#[derive(Debug, Serialize)]
/// Response for batch tool schema mutations.
struct ToolSchemasResponse {
    names: Vec<String>,
}

#[derive(Debug, Deserialize)]
/// Request body for attaching an available provider tool to a conversation.
struct AttachToolRequest {
    provider_id: String,
    tool_name: String,
    head_message_id: Option<String>,
}

#[derive(Debug, Deserialize)]
/// Request body for attaching multiple available provider tools.
struct AttachToolsRequest {
    tools: Vec<AttachToolRequest>,
    head_message_id: Option<String>,
}

/// Attaches one available provider tool to a conversation.
async fn attach_tool(
    State(state): State<ApiState>,
    Path(conversation_id): Path<String>,
    Json(request): Json<AttachToolRequest>,
) -> ApiResult<ToolSchemaResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let provider_id = ToolProviderId::new(request.provider_id);
    let tool_name = ProviderToolName::new(request.tool_name);
    let head_message_id = requested_head_message_id(request.head_message_id);
    let mut store = open_store(&state)?;
    let schema_name = match head_message_id.as_ref() {
        Some(head_message_id) => operation::attach_tool_with_registry_at_head(
            &mut store,
            &conversation_id,
            Some(head_message_id),
            &provider_id,
            &tool_name,
            &state.tool_registry,
        )?,
        None => operation::attach_tool_with_registry(
            &mut store,
            &conversation_id,
            &provider_id,
            &tool_name,
            &state.tool_registry,
        )?,
    };

    Ok(Json(ToolSchemaResponse {
        name: schema_name.as_str().to_string(),
    }))
}

/// Attaches multiple available provider tools to a conversation.
async fn attach_tools(
    State(state): State<ApiState>,
    Path(conversation_id): Path<String>,
    Json(request): Json<AttachToolsRequest>,
) -> ApiResult<ToolSchemasResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let head_message_id = requested_head_message_id(request.head_message_id);
    let requests = request
        .tools
        .into_iter()
        .map(|tool| {
            operation::ToolAttachmentInput::new(
                ToolProviderId::new(tool.provider_id),
                ProviderToolName::new(tool.tool_name),
            )
        })
        .collect::<Vec<_>>();
    let mut store = open_store(&state)?;
    let schema_names = match head_message_id.as_ref() {
        Some(head_message_id) => operation::attach_tools_with_registry_at_head(
            &mut store,
            &conversation_id,
            Some(head_message_id),
            &requests,
            &state.tool_registry,
        )?,
        None => operation::attach_tools_with_registry(
            &mut store,
            &conversation_id,
            &requests,
            &state.tool_registry,
        )?,
    };

    Ok(Json(ToolSchemasResponse {
        names: schema_names
            .into_iter()
            .map(|name| name.as_str().to_string())
            .collect(),
    }))
}

/// Inserts one tool schema on the active or requested path.
async fn insert_tool_schema(
    State(state): State<ApiState>,
    Path(conversation_id): Path<String>,
    Json(request): Json<ToolSchemaRequest>,
) -> ApiResult<ToolSchemaResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let (tool_schema, head_message_id) = request.into_parts();
    let mut store = open_store(&state)?;

    match head_message_id.as_ref() {
        Some(head_message_id) => operation::insert_tool_schema_at_head(
            &mut store,
            &conversation_id,
            Some(head_message_id),
            &tool_schema,
        )?,
        None => operation::insert_tool_schema(&mut store, &conversation_id, &tool_schema)?,
    }

    Ok(Json(ToolSchemaResponse {
        name: tool_schema.name.as_str().to_string(),
    }))
}

/// Updates one tool schema on the active or requested path.
async fn update_tool_schema(
    State(state): State<ApiState>,
    Path((conversation_id, name)): Path<(String, String)>,
    Json(request): Json<ToolSchemaRequest>,
) -> ApiResult<ToolSchemaResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let current_name = ToolSchemaName::new(name);
    let (tool_schema, head_message_id) = request.into_parts();
    let mut store = open_store(&state)?;

    match head_message_id.as_ref() {
        Some(head_message_id) => operation::update_tool_schema_at_head(
            &mut store,
            &conversation_id,
            Some(head_message_id),
            &current_name,
            &tool_schema,
        )?,
        None => operation::update_tool_schema(
            &mut store,
            &conversation_id,
            &current_name,
            &tool_schema,
        )?,
    }

    Ok(Json(ToolSchemaResponse {
        name: tool_schema.name.as_str().to_string(),
    }))
}

/// Removes one tool schema from the active or requested path.
async fn remove_tool_schema(
    State(state): State<ApiState>,
    Path((conversation_id, name)): Path<(String, String)>,
    Query(query): Query<ContextMutationQuery>,
) -> ApiResult<DeletedResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let name = ToolSchemaName::new(name);
    let head_message_id = requested_head_message_id(query.head_message_id);
    let mut store = open_store(&state)?;

    match head_message_id.as_ref() {
        Some(head_message_id) => operation::remove_tool_schema_at_head(
            &mut store,
            &conversation_id,
            Some(head_message_id),
            &name,
        )?,
        None => operation::remove_tool_schema(&mut store, &conversation_id, &name)?,
    }

    Ok(Json(DeletedResponse { deleted: true }))
}

/// Detaches one provider-backed tool schema from a conversation.
async fn detach_tool(
    State(state): State<ApiState>,
    Path((conversation_id, schema_name)): Path<(String, String)>,
    Query(query): Query<ContextMutationQuery>,
) -> ApiResult<DeletedResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let schema_name = ToolSchemaName::new(schema_name);
    let head_message_id = requested_head_message_id(query.head_message_id);
    let mut store = open_store(&state)?;

    match head_message_id.as_ref() {
        Some(head_message_id) => operation::detach_tool_at_head(
            &mut store,
            &conversation_id,
            Some(head_message_id),
            &schema_name,
        )?,
        None => operation::detach_tool(&mut store, &conversation_id, &schema_name)?,
    }

    Ok(Json(DeletedResponse { deleted: true }))
}

/// Deletes descendants after one checkpoint message.
async fn truncate_conversation(
    State(state): State<ApiState>,
    Path(conversation_id): Path<String>,
    Json(request): Json<MessageIdRequest>,
) -> ApiResult<ActiveMessageResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let message_id = MessageId::new(request.message_id);
    let mut store = open_store(&state)?;

    operation::truncate_conversation(&mut store, &conversation_id, &message_id)?;

    Ok(Json(ActiveMessageResponse {
        active_message_id: store
            .active_message_id(&conversation_id)?
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
    let mut store = open_store(&state)?;
    let forked_conversation_id =
        operation::fork_conversation(&mut store, &conversation_id, &message_id)?;

    Ok(Json(ConversationIdResponse {
        conversation_id: forked_conversation_id.as_str().to_string(),
    }))
}

#[derive(Debug, Serialize)]
/// Response body for pending session-owned tool approvals.
struct ApprovalListResponse {
    approvals: Vec<ApprovalResponse>,
}

#[derive(Debug, Serialize)]
/// One pending session-owned approval returned to UI clients.
struct ApprovalResponse {
    scope: &'static str,
    session_id: String,
    conversation_id: String,
    session_status: SessionStatus,
    head_message_id: Option<String>,
    assistant_message_id: String,
    tool_call_id: String,
    tool_name: String,
    arguments: String,
    reason: String,
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
async fn list_conversation_session_approvals(
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
async fn list_session_approvals(
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
async fn approve_session_tool(
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
async fn deny_session_tool(
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

#[derive(Debug, Deserialize)]
/// Request body for starting a backend-owned session.
struct CreateSessionRequest {
    head_message_id: Option<String>,
    model: Option<String>,
    reasoning: Option<ReasoningRequest>,
}

impl CreateSessionRequest {
    fn reasoning(&self) -> Option<ReasoningRequest> {
        self.reasoning
            .clone()
            .filter(|reasoning| !reasoning.is_empty())
    }
}

#[derive(Debug, Serialize)]
/// Serializable run response.
struct SessionResponse {
    id: String,
    conversation_id: String,
    start_head_message_id: Option<String>,
    current_head_message_id: Option<String>,
    status: SessionStatus,
    model: String,
    reasoning: Option<ReasoningRequest>,
    error: Option<String>,
    created_at: i64,
    updated_at: i64,
}

impl SessionResponse {
    fn from_session(session: Session) -> Self {
        Self {
            id: session.id.as_str().to_string(),
            conversation_id: session.conversation_id.as_str().to_string(),
            start_head_message_id: session
                .start_head_message_id
                .map(|id| id.as_str().to_string()),
            current_head_message_id: session
                .current_head_message_id
                .map(|id| id.as_str().to_string()),
            status: session.status,
            model: session.model,
            reasoning: session.reasoning,
            error: session.error,
            created_at: session.created_at,
            updated_at: session.updated_at,
        }
    }
}

#[derive(Debug, Serialize)]
/// List of runtime sessions visible to clients.
struct SessionListResponse {
    sessions: Vec<SessionResponse>,
}

/// Starts a backend-owned session from an explicit conversation head.
async fn create_session(
    State(state): State<ApiState>,
    Path(conversation_id): Path<String>,
    Json(request): Json<CreateSessionRequest>,
) -> ApiResult<SessionResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let head_message_id = request.head_message_id.clone().map(MessageId::new);
    let reasoning = request.reasoning();
    let session = state
        .session_manager
        .start_continue_wakeup(ContinueWakeup {
            conversation_id,
            head_message_id,
            model: request.model.map(ModelName::new),
            reasoning,
        })?;

    Ok(Json(SessionResponse::from_session(session)))
}

/// Lists persisted sessions.
async fn list_sessions(State(state): State<ApiState>) -> ApiResult<SessionListResponse> {
    let store = open_store(&state)?;
    let sessions = store
        .list_sessions()?
        .into_iter()
        .map(SessionResponse::from_session)
        .collect();

    Ok(Json(SessionListResponse { sessions }))
}

/// Loads one persisted session.
async fn get_run(
    State(state): State<ApiState>,
    Path(session_id): Path<String>,
) -> ApiResult<SessionResponse> {
    let store = open_store(&state)?;
    let session = store.load_session(&SessionId::new(session_id))?;

    Ok(Json(SessionResponse::from_session(session)))
}

/// Stops one live session explicitly.
async fn stop_run(
    State(state): State<ApiState>,
    Path(session_id): Path<String>,
) -> ApiResult<SessionResponse> {
    let session_id = SessionId::new(session_id);
    state.session_manager.stop(&session_id)?;
    let store = open_store(&state)?;
    let session = store.load_session(&session_id)?;

    Ok(Json(SessionResponse::from_session(session)))
}

#[derive(Debug, Deserialize)]
/// Cursor query for replaying session events.
struct SessionEventsQuery {
    after: Option<i64>,
}

struct SessionSseState {
    replay: VecDeque<SessionEventRecord>,
    subscription: Option<SessionSubscription>,
}

/// Streams persisted and live events for one session.
async fn session_events(
    State(state): State<ApiState>,
    Path(session_id): Path<String>,
    Query(query): Query<SessionEventsQuery>,
) -> Result<Sse<impl futures_util::Stream<Item = std::result::Result<Event, Infallible>>>, ApiError>
{
    let session_id = SessionId::new(session_id);
    let store = open_store(&state)?;
    let replay = store.load_session_events_after(&session_id, query.after)?;
    let subscription = state.session_manager.subscribe(&session_id);
    let stream = stream::unfold(
        SessionSseState {
            replay: replay.into(),
            subscription,
        },
        |mut state| async move {
            let record = if let Some(record) = state.replay.pop_front() {
                record
            } else if let Some(subscription) = state.subscription.as_mut() {
                match subscription.recv().await {
                    Ok(record) => record,
                    Err(_) => return None,
                }
            } else {
                return None;
            };
            let event_name = record.event.event_name();
            let data = session_event_data(&record);
            let sse = Event::default()
                .id(record.id.to_string())
                .event(event_name)
                .data(data);

            Some((Ok::<Event, Infallible>(sse), state))
        },
    );

    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

fn session_event_data(record: &SessionEventRecord) -> String {
    let mut value = serde_json::to_value(&record.event).unwrap_or_else(|error| {
        serde_json::json!({
            "type": "failed",
            "error": format!("failed to serialize runtime event: {error}"),
            "causes": [format!("failed to serialize runtime event: {error}")],
        })
    });
    if let Some(object) = value.as_object_mut() {
        object.insert("event_id".to_string(), serde_json::json!(record.id));
        object.insert(
            "session_id".to_string(),
            serde_json::json!(record.session_id.as_str()),
        );
        object.insert(
            "created_at".to_string(),
            serde_json::json!(record.created_at),
        );
    }

    value.to_string()
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
                source: None,
                raw: None,
            },
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
    let store = open_store(&state)?;
    let model = operation::resolve_conversation_model(
        &store,
        &conversation_id,
        request.model.map(ModelName::new),
    )?;
    let context = operation::conversation_input_token_context(&store, &conversation_id)?;
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

    Ok(Json(InputTokensResponse::from_count(count, source)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{Body, to_bytes};
    use axum::http::Request as HttpRequest;
    use serde_json::json;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tower::ServiceExt;

    use crate::conversation::{MessageMetadata, MessagePart, ToolCall};
    use crate::mcp::McpCommand;
    use crate::session::{SessionId, SessionStatus};
    use crate::tool::{ToolAnnotations, ToolPermission, ToolProviderKind, ToolProviderRef};

    static TEMP_DB_COUNTER: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn assistant_delta_event_uses_matching_sse_name_and_json_type() {
        let event = crate::session::SessionEvent::AssistantDelta {
            text: "hello".to_string(),
        };
        let body = serde_json::to_value(&event).unwrap();

        assert_eq!(event.event_name(), "assistant_delta");
        assert_eq!(body["type"], "assistant_delta");
        assert_eq!(body["text"], "hello");
    }

    #[test]
    fn reasoning_delta_event_uses_matching_sse_name_and_json_type() {
        let event = crate::session::SessionEvent::ReasoningDelta {
            text: "thinking".to_string(),
        };
        let body = serde_json::to_value(&event).unwrap();

        assert_eq!(event.event_name(), "reasoning_delta");
        assert_eq!(body["type"], "reasoning_delta");
        assert_eq!(body["text"], "thinking");
    }

    #[test]
    fn tool_call_delta_event_uses_matching_sse_name_and_json_type() {
        let event = crate::session::SessionEvent::ToolCallDelta {
            index: 0,
            id: Some("call_123".to_string()),
            name: Some("run_shell".to_string()),
            arguments_delta: Some(r#"{"command""#.to_string()),
        };
        let body = serde_json::to_value(&event).unwrap();

        assert_eq!(event.event_name(), "tool_call_delta");
        assert_eq!(body["type"], "tool_call_delta");
        assert_eq!(body["index"], 0);
        assert_eq!(body["id"], "call_123");
        assert_eq!(body["name"], "run_shell");
        assert_eq!(body["arguments_delta"], r#"{"command""#);
    }

    #[tokio::test]
    async fn health_does_not_require_token() {
        let app = test_app(temp_database_path());
        let response = app
            .oneshot(
                HttpRequest::builder()
                    .method(Method::GET)
                    .uri("/api/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response_json(response).await["ok"], true);
    }

    #[tokio::test]
    async fn protected_routes_reject_missing_or_invalid_token() {
        let app = test_app(temp_database_path());
        let missing = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .method(Method::GET)
                    .uri("/api/conversations")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let invalid = app
            .oneshot(
                HttpRequest::builder()
                    .method(Method::GET)
                    .uri("/api/conversations")
                    .header(API_TOKEN_HEADER, "wrong-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(missing.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(invalid.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn typed_windie_errors_map_to_http_status_codes() {
        let db_path = temp_database_path();
        let app = test_app(db_path.clone());
        let missing = app
            .clone()
            .oneshot(authed_request(
                Method::GET,
                "/api/conversations/missing",
                None,
            ))
            .await
            .unwrap();

        let created = response_json(
            app.clone()
                .oneshot(authed_request(Method::POST, "/api/conversations", None))
                .await
                .unwrap(),
        )
        .await;
        let conversation_id = created["conversation_id"].as_str().unwrap();
        let invalid = app
            .oneshot(authed_request(
                Method::POST,
                &format!("/api/conversations/{conversation_id}/messages"),
                Some(json!({"role":"user","text":""})),
            ))
            .await
            .unwrap();

        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
        assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
        let _ = fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn conversation_model_route_persists_model() {
        let db_path = temp_database_path();
        let app = test_app(db_path.clone());
        let created = response_json(
            app.clone()
                .oneshot(authed_request(Method::POST, "/api/conversations", None))
                .await
                .unwrap(),
        )
        .await;
        let conversation_id = created["conversation_id"].as_str().unwrap();

        let updated = response_json(
            app.clone()
                .oneshot(authed_request(
                    Method::PATCH,
                    &format!("/api/conversations/{conversation_id}/model"),
                    Some(json!({"model":"anthropic/test"})),
                ))
                .await
                .unwrap(),
        )
        .await;
        let inspected = response_json(
            app.clone()
                .oneshot(authed_request(
                    Method::GET,
                    &format!("/api/conversations/{conversation_id}"),
                    None,
                ))
                .await
                .unwrap(),
        )
        .await;
        let listed = response_json(
            app.oneshot(authed_request(Method::GET, "/api/conversations", None))
                .await
                .unwrap(),
        )
        .await;

        assert_eq!(updated["model"], "anthropic/test");
        assert_eq!(inspected["model"], "anthropic/test");
        assert_eq!(listed["conversations"][0]["model"], "anthropic/test");

        let _ = fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn conversation_reasoning_route_persists_reasoning_effort() {
        let db_path = temp_database_path();
        let app = test_app(db_path.clone());
        let created = response_json(
            app.clone()
                .oneshot(authed_request(Method::POST, "/api/conversations", None))
                .await
                .unwrap(),
        )
        .await;
        let conversation_id = created["conversation_id"].as_str().unwrap();

        let updated = response_json(
            app.clone()
                .oneshot(authed_request(
                    Method::PATCH,
                    &format!("/api/conversations/{conversation_id}/reasoning"),
                    Some(json!({"effort":"high"})),
                ))
                .await
                .unwrap(),
        )
        .await;
        let inspected = response_json(
            app.clone()
                .oneshot(authed_request(
                    Method::GET,
                    &format!("/api/conversations/{conversation_id}"),
                    None,
                ))
                .await
                .unwrap(),
        )
        .await;
        let cleared = response_json(
            app.oneshot(authed_request(
                Method::PATCH,
                &format!("/api/conversations/{conversation_id}/reasoning"),
                Some(json!({"effort":null})),
            ))
            .await
            .unwrap(),
        )
        .await;

        assert_eq!(updated["reasoning"]["effort"], "high");
        assert_eq!(inspected["reasoning"]["effort"], "high");
        assert_eq!(cleared["reasoning"], Value::Null);

        let _ = fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn conversation_image_route_returns_scoped_image_bytes() {
        let db_path = temp_database_path();
        let conversation_id = {
            let mut store = Store::open_at(&db_path).unwrap();
            let conversation_id = store.create_conversation("openai/test").unwrap();
            store
                .insert_message_with_parts(
                    &conversation_id,
                    None,
                    Role::User,
                    "image",
                    &[
                        crate::conversation::UnsavedMessagePart::Text("image".to_string()),
                        crate::conversation::UnsavedMessagePart::Image(
                            crate::conversation::UnsavedImagePart {
                                mime_type: "image/png".to_string(),
                                bytes: vec![1, 2, 3],
                            },
                        ),
                    ],
                    None,
                )
                .unwrap();
            conversation_id
        };
        let asset_id = {
            let store = Store::open_at(&db_path).unwrap();
            let messages = store.load_messages(&conversation_id).unwrap();
            match &messages[0].parts[1] {
                MessagePart::Image(image) => image.asset_id.as_str().to_string(),
                _ => panic!("expected image part"),
            }
        };
        let app = test_app(db_path.clone());

        let response = app
            .oneshot(authed_request(
                Method::GET,
                &format!("/api/conversations/{conversation_id}/images/{asset_id}"),
                None,
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[CONTENT_TYPE], "image/png");
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(&body[..], &[1, 2, 3]);

        let _ = fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn insert_tool_role_message_returns_raw_error() {
        let db_path = temp_database_path();
        let app = test_app(db_path.clone());
        let created = response_json(
            app.clone()
                .oneshot(authed_request(Method::POST, "/api/conversations", None))
                .await
                .unwrap(),
        )
        .await;
        let conversation_id = created["conversation_id"].as_str().unwrap();

        let response = app
            .oneshot(authed_request(
                Method::POST,
                &format!("/api/conversations/{conversation_id}/messages"),
                Some(json!({"role":"tool","text":"tool output"})),
            ))
            .await
            .unwrap();
        let status = response.status();
        let body = response_json_body(response).await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(
            body["error"],
            "role: tool messages must be created through approve or deny"
        );
        let _ = fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn insert_message_accepts_image_data_part() {
        let db_path = temp_database_path();
        let app = test_app(db_path.clone());
        let created = response_json(
            app.clone()
                .oneshot(authed_request(Method::POST, "/api/conversations", None))
                .await
                .unwrap(),
        )
        .await;
        let conversation_id = created["conversation_id"].as_str().unwrap();

        response_json(
            app.clone()
                .oneshot(authed_request(
                    Method::POST,
                    &format!("/api/conversations/{conversation_id}/messages"),
                    Some(json!({
                        "role": "user",
                        "parts": [
                            {"type": "text", "text": "clipboard image"},
                            {
                                "type": "image_data",
                                "mime_type": "image/png",
                                "data": "iVBORw0KGgo="
                            }
                        ]
                    })),
                ))
                .await
                .unwrap(),
        )
        .await;

        let report = response_json(
            app.oneshot(authed_request(
                Method::GET,
                &format!("/api/conversations/{conversation_id}"),
                None,
            ))
            .await
            .unwrap(),
        )
        .await;
        let parts = report["messages"][0]["parts"].as_array().unwrap();
        assert_eq!(parts[1]["type"], "image");
        assert_eq!(parts[1]["mime_type"], "image/png");
        assert_eq!(parts[1]["byte_count"], 8);

        let _ = fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn routes_share_operations_for_conversation_inspection_flow() {
        let db_path = temp_database_path();
        let app = test_app(db_path.clone());
        let created = response_json(
            app.clone()
                .oneshot(authed_request(Method::POST, "/api/conversations", None))
                .await
                .unwrap(),
        )
        .await;
        let conversation_id = created["conversation_id"].as_str().unwrap();

        let listed = response_json(
            app.clone()
                .oneshot(authed_request(Method::GET, "/api/conversations", None))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(listed["conversations"].as_array().unwrap().len(), 1);

        response_json(
            app.clone()
                .oneshot(authed_request(
                    Method::POST,
                    &format!("/api/conversations/{conversation_id}/messages"),
                    Some(json!({"role":"user","text":"hello from api"})),
                ))
                .await
                .unwrap(),
        )
        .await;
        response_json(
            app.clone()
                .oneshot(authed_request(
                    Method::PATCH,
                    &format!("/api/conversations/{conversation_id}/system-prompt"),
                    Some(json!({"text":"Use short answers."})),
                ))
                .await
                .unwrap(),
        )
        .await;
        let tool_mode = response_json(
            app.clone()
                .oneshot(authed_request(
                    Method::PATCH,
                    &format!("/api/conversations/{conversation_id}/tool-approval-mode"),
                    Some(json!({"mode":"auto_approve_attached"})),
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(tool_mode["tool_approval_mode"], "auto_approve_attached");
        response_json(
            app.clone()
                .oneshot(authed_request(
                    Method::POST,
                    &format!("/api/conversations/{conversation_id}/tool-schemas"),
                    Some(json!({
                        "name":"run_shell",
                        "description":"Run a command",
                        "parameters":{"type":"object"}
                    })),
                ))
                .await
                .unwrap(),
        )
        .await;

        let inspected = response_json(
            app.oneshot(authed_request(
                Method::GET,
                &format!("/api/conversations/{conversation_id}?model=openai/test"),
                None,
            ))
            .await
            .unwrap(),
        )
        .await;

        assert_eq!(inspected["system_prompt"], "Use short answers.");
        assert_eq!(inspected["tool_approval_mode"], "auto_approve_attached");
        assert_eq!(inspected["messages"][0]["content"], "hello from api");
        assert_eq!(inspected["active_path"][0]["content"], "hello from api");
        assert_eq!(inspected["model_context"][0]["role"], "system");
        assert_eq!(inspected["tool_schemas"][0]["name"], "run_shell");
        let _ = fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn update_remove_and_schema_routes_share_operations() {
        let db_path = temp_database_path();
        let app = test_app(db_path.clone());
        let created = response_json(
            app.clone()
                .oneshot(authed_request(Method::POST, "/api/conversations", None))
                .await
                .unwrap(),
        )
        .await;
        let conversation_id = created["conversation_id"].as_str().unwrap();
        let inserted = response_json(
            app.clone()
                .oneshot(authed_request(
                    Method::POST,
                    &format!("/api/conversations/{conversation_id}/messages"),
                    Some(json!({"role":"user","text":"before"})),
                ))
                .await
                .unwrap(),
        )
        .await;
        let message_id = inserted["message_id"].as_str().unwrap();

        response_json(
            app.clone()
                .oneshot(authed_request(
                    Method::PATCH,
                    &format!("/api/conversations/{conversation_id}/messages/{message_id}"),
                    Some(json!({"text":"after"})),
                ))
                .await
                .unwrap(),
        )
        .await;
        response_json(
            app.clone()
                .oneshot(authed_request(
                    Method::POST,
                    &format!("/api/conversations/{conversation_id}/tool-schemas"),
                    Some(json!({
                        "name":"first_tool",
                        "description":"First",
                        "parameters":{"type":"object"}
                    })),
                ))
                .await
                .unwrap(),
        )
        .await;
        response_json(
            app.clone()
                .oneshot(authed_request(
                    Method::PATCH,
                    &format!("/api/conversations/{conversation_id}/tool-schemas/first_tool"),
                    Some(json!({
                        "name":"second_tool",
                        "description":"Second",
                        "parameters":{"type":"object","properties":{}}
                    })),
                ))
                .await
                .unwrap(),
        )
        .await;
        response_json(
            app.clone()
                .oneshot(authed_request(
                    Method::DELETE,
                    &format!("/api/conversations/{conversation_id}/tool-schemas/second_tool"),
                    None,
                ))
                .await
                .unwrap(),
        )
        .await;
        response_json(
            app.clone()
                .oneshot(authed_request(
                    Method::DELETE,
                    &format!("/api/conversations/{conversation_id}/messages/{message_id}"),
                    None,
                ))
                .await
                .unwrap(),
        )
        .await;
        response_json(
            app.clone()
                .oneshot(authed_request(
                    Method::DELETE,
                    &format!("/api/conversations/{conversation_id}"),
                    None,
                ))
                .await
                .unwrap(),
        )
        .await;

        let listed = response_json(
            app.oneshot(authed_request(Method::GET, "/api/conversations", None))
                .await
                .unwrap(),
        )
        .await;
        assert!(listed["conversations"].as_array().unwrap().is_empty());
        let _ = fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn context_mutation_routes_can_target_explicit_heads() {
        let db_path = temp_database_path();
        let app = test_app(db_path.clone());
        let mut store = Store::open_at(&db_path).unwrap();
        let conversation_id = store.create_conversation("openai/test").unwrap();
        let root_id = store
            .insert_message(&conversation_id, None, Role::User, "root", None)
            .unwrap();
        let shared_id = store
            .insert_message(&conversation_id, Some(&root_id), Role::User, "shared", None)
            .unwrap();
        let branch_id = store
            .insert_message(
                &conversation_id,
                Some(&shared_id),
                Role::User,
                "branch",
                None,
            )
            .unwrap();
        let sibling_id = store
            .insert_message(
                &conversation_id,
                Some(&shared_id),
                Role::User,
                "sibling",
                None,
            )
            .unwrap();
        store
            .set_active_message(&conversation_id, &sibling_id)
            .unwrap();
        drop(store);

        let prompt_response = response_json(
            app.clone()
                .oneshot(authed_request(
                    Method::PATCH,
                    &format!("/api/conversations/{conversation_id}/system-prompt"),
                    Some(json!({
                        "text": "branch prompt",
                        "head_message_id": branch_id.as_str()
                    })),
                ))
                .await
                .unwrap(),
        )
        .await;
        let tool_response = response_json(
            app.oneshot(authed_request(
                Method::POST,
                &format!("/api/conversations/{conversation_id}/tool-schemas"),
                Some(json!({
                    "name": "branch_tool",
                    "description": "Branch tool",
                    "parameters": {"type": "object"},
                    "head_message_id": branch_id.as_str()
                })),
            ))
            .await
            .unwrap(),
        )
        .await;

        assert_eq!(prompt_response["system_prompt"], "branch prompt");
        assert_eq!(tool_response["name"], "branch_tool");

        let store = Store::open_at(&db_path).unwrap();

        assert_eq!(
            store
                .effective_system_prompt_for_head(&conversation_id, Some(&branch_id))
                .unwrap()
                .as_deref(),
            Some("branch prompt")
        );
        assert!(
            store
                .effective_system_prompt_for_head(&conversation_id, Some(&sibling_id))
                .unwrap()
                .is_none()
        );
        assert_eq!(
            store
                .load_tool_schemas_for_head(&conversation_id, Some(&branch_id))
                .unwrap()[0]
                .name
                .as_str(),
            "branch_tool"
        );
        assert!(
            store
                .load_tool_schemas_for_head(&conversation_id, Some(&sibling_id))
                .unwrap()
                .is_empty()
        );
        let _ = fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn batch_attach_tools_route_attaches_provider_tools() {
        let db_path = temp_database_path();
        let app = test_app_with_tool_registry(
            db_path.clone(),
            Arc::new(registry_with_cached_test_tool()),
        );
        let created = response_json(
            app.clone()
                .oneshot(authed_request(Method::POST, "/api/conversations", None))
                .await
                .unwrap(),
        )
        .await;
        let conversation_id = created["conversation_id"].as_str().unwrap();

        let attached = response_json(
            app.clone()
                .oneshot(authed_request(
                    Method::POST,
                    &format!("/api/conversations/{conversation_id}/tools/batch"),
                    Some(json!({
                        "tools": [
                            {
                                "provider_id": "desktop-commander",
                                "tool_name": "read_file"
                            }
                        ]
                    })),
                ))
                .await
                .unwrap(),
        )
        .await;
        let inspected = response_json(
            app.oneshot(authed_request(
                Method::GET,
                &format!("/api/conversations/{conversation_id}"),
                None,
            ))
            .await
            .unwrap(),
        )
        .await;

        assert_eq!(attached["names"], json!(["desktop_commander__read_file"]));
        assert_eq!(
            inspected["tool_schemas"][0]["name"],
            "desktop_commander__read_file"
        );
        let _ = fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn remove_tool_result_message_deletes_tool_group() {
        let db_path = temp_database_path();
        let app = test_app(db_path.clone());
        let mut store = Store::open_at(&db_path).unwrap();
        let conversation_id = store.create_conversation("openai/test").unwrap();
        let parent_id = store
            .insert_message(&conversation_id, None, Role::User, "one", None)
            .unwrap();
        let assistant_id = store
            .insert_message(
                &conversation_id,
                Some(&parent_id),
                Role::Assistant,
                "",
                Some(&MessageMetadata {
                    tool_calls: vec![ToolCall::function("call_1", "run_shell", "{}")],
                    ..Default::default()
                }),
            )
            .unwrap();
        let message_id = store
            .insert_tool_result_message(
                &conversation_id,
                &assistant_id,
                &ToolCallId::new("call_1"),
                "{}",
            )
            .unwrap();
        drop(store);

        let response = app
            .oneshot(authed_request(
                Method::DELETE,
                &format!("/api/conversations/{conversation_id}/messages/{message_id}"),
                None,
            ))
            .await
            .unwrap();
        let status = response.status();
        let body = response_json_body(response).await;
        let store = Store::open_at(&db_path).unwrap();
        let messages = store.load_messages(&conversation_id).unwrap();

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["deleted"], true);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].id.as_ref(), Some(&parent_id));
        let _ = fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn approve_later_multi_tool_call_records_raw_order_error() {
        let db_path = temp_database_path();
        let app = test_app(db_path.clone());
        let conversation_id = insert_multi_tool_call_assistant(&db_path);
        let head_message_id = Store::open_at(&db_path)
            .unwrap()
            .active_message_id(&conversation_id)
            .unwrap()
            .unwrap();
        let session_id = create_waiting_run(&db_path, &conversation_id, &head_message_id);

        let response = app
            .oneshot(authed_request(
                Method::POST,
                &format!("/api/sessions/{session_id}/approvals/call_2/approve"),
                None,
            ))
            .await
            .unwrap();
        let status = response.status();
        let _body = response_json_body(response).await;
        let session = wait_for_session_status(&db_path, &session_id, SessionStatus::Failed).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(session.status, SessionStatus::Failed);
        assert!(
            session
                .error
                .as_deref()
                .unwrap_or("")
                .contains("tool call must be resolved after previous tool call: call_1")
        );
        let _ = fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn deny_first_multi_tool_call_records_result_without_querying() {
        let db_path = temp_database_path();
        let registry = Arc::new(registry_with_cached_test_tool());
        let app = test_app_with_tool_registry(db_path.clone(), registry.clone());
        let conversation_id = insert_attached_multi_tool_call_assistant(&db_path);
        let head_message_id = Store::open_at(&db_path)
            .unwrap()
            .active_message_id(&conversation_id)
            .unwrap()
            .unwrap();
        let session_id = create_waiting_run(&db_path, &conversation_id, &head_message_id);

        let response = app
            .oneshot(authed_request(
                Method::POST,
                &format!("/api/sessions/{session_id}/approvals/call_1/deny"),
                None,
            ))
            .await
            .unwrap();
        let status = response.status();
        let _body = response_json_body(response).await;
        let events = wait_for_run_event_count(&db_path, &session_id, 2).await;
        let session = Store::open_at(&db_path)
            .unwrap()
            .load_session(&session_id)
            .unwrap();

        assert_eq!(status, StatusCode::OK);
        assert_eq!(session.status, SessionStatus::WaitingForApproval);
        assert!(events.iter().any(|event| matches!(
            event.event,
            crate::session::SessionEvent::ToolResultSaved { .. }
        )));
        assert!(events.iter().any(|event| matches!(
            event.event,
            crate::session::SessionEvent::WaitingForApproval
        )));
        let _ = fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn create_session_records_gateway_error() {
        let db_path = temp_database_path();
        let app = test_app_with_gateway(db_path.clone(), "http://127.0.0.1:1");
        let mut store = Store::open_at(&db_path).unwrap();
        let conversation_id = store.create_conversation("openai/test").unwrap();
        let head_message_id = store
            .insert_message(&conversation_id, None, Role::User, "hello", None)
            .unwrap();
        drop(store);

        let response = app
            .oneshot(authed_request(
                Method::POST,
                &format!("/api/conversations/{conversation_id}/sessions"),
                Some(json!({"head_message_id": head_message_id.as_str(), "model":"openai/test"})),
            ))
            .await
            .unwrap();
        let status = response.status();
        let body = response_json_body(response).await;
        let session_id = SessionId::new(body["id"].as_str().unwrap());
        let session = wait_for_session_status(&db_path, &session_id, SessionStatus::Failed).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(session.status, SessionStatus::Failed);
        assert_eq!(
            session.error.as_deref(),
            Some("Bifrost is not running. Start it with: windie gateway start")
        );
        let _ = fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn query_wakeup_starts_session_from_active_head() {
        let db_path = temp_database_path();
        let app = test_app_with_gateway(db_path.clone(), "http://127.0.0.1:1");
        let mut store = Store::open_at(&db_path).unwrap();
        let conversation_id = store.create_conversation("openai/test").unwrap();
        let head_message_id = store
            .insert_message(&conversation_id, None, Role::User, "hello", None)
            .unwrap();
        drop(store);

        let response = app
            .oneshot(authed_request(
                Method::POST,
                &format!("/api/conversations/{conversation_id}/wakeups/query"),
                Some(json!({"model":"openai/test"})),
            ))
            .await
            .unwrap();
        let status = response.status();
        let body = response_json_body(response).await;
        let session_id = SessionId::new(body["id"].as_str().unwrap());
        let session = wait_for_session_status(&db_path, &session_id, SessionStatus::Failed).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            session.start_head_message_id.as_ref(),
            Some(&head_message_id)
        );
        assert_eq!(
            session.current_head_message_id.as_ref(),
            Some(&head_message_id)
        );
        assert_eq!(session.status, SessionStatus::Failed);
        let _ = fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn session_events_replay_gateway_errors_as_sse_events() {
        let db_path = temp_database_path();
        let app = test_app_with_gateway(db_path.clone(), "http://127.0.0.1:1");
        let mut store = Store::open_at(&db_path).unwrap();
        let conversation_id = store.create_conversation("openai/test").unwrap();
        let head_message_id = store
            .insert_message(&conversation_id, None, Role::User, "hello", None)
            .unwrap();
        drop(store);

        let response = app
            .clone()
            .oneshot(authed_request(
                Method::POST,
                &format!("/api/conversations/{conversation_id}/sessions"),
                Some(json!({"head_message_id": head_message_id.as_str(), "model":"openai/test"})),
            ))
            .await
            .unwrap();
        let body = response_json_body(response).await;
        let session_id = SessionId::new(body["id"].as_str().unwrap());
        let _run = wait_for_session_status(&db_path, &session_id, SessionStatus::Failed).await;

        let response = app
            .oneshot(authed_request(
                Method::GET,
                &format!("/api/sessions/{session_id}/events"),
                None,
            ))
            .await
            .unwrap();
        let status = response.status();
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_string();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body = String::from_utf8(bytes.to_vec()).unwrap();

        assert_eq!(status, StatusCode::OK);
        assert!(content_type.starts_with("text/event-stream"));
        assert!(body.contains("event: failed"));
        assert!(body.contains("Bifrost is not running. Start it with: windie gateway start"));
        let _ = fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn session_survives_event_stream_client_disconnect() {
        let db_path = temp_database_path();
        let mock = spawn_mock_bifrost(Duration::from_millis(100)).await;
        let app = test_app_with_urls(db_path.clone(), &mock.gateway_url, &mock.base_url);
        let mut store = Store::open_at(&db_path).unwrap();
        let conversation_id = store.create_conversation("openai/test").unwrap();
        let head_message_id = store
            .insert_message(&conversation_id, None, Role::User, "hello", None)
            .unwrap();
        drop(store);

        let response = app
            .clone()
            .oneshot(authed_request(
                Method::POST,
                &format!("/api/conversations/{conversation_id}/sessions"),
                Some(json!({"head_message_id": head_message_id.as_str(), "model":"openai/test"})),
            ))
            .await
            .unwrap();
        let body = response_json(response).await;
        let session_id = SessionId::new(body["id"].as_str().unwrap());

        let response = app
            .oneshot(authed_request(
                Method::GET,
                &format!("/api/sessions/{session_id}/events"),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        drop(response);

        let session =
            wait_for_session_status(&db_path, &session_id, SessionStatus::Completed).await;
        let store = Store::open_at(&db_path).unwrap();
        let messages = store.load_messages(&conversation_id).unwrap();
        let events = store.load_session_events_after(&session_id, None).unwrap();

        assert_eq!(session.status, SessionStatus::Completed);
        assert!(session.error.is_none());
        assert!(messages.iter().any(|message| {
            message.role == Role::Assistant && message.content == "after disconnect"
        }));
        assert!(
            events
                .iter()
                .any(|event| matches!(event.event, crate::session::SessionEvent::Completed { .. }))
        );
        let _ = fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn model_list_route_returns_gateway_error_when_gateway_is_offline() {
        let db_path = temp_database_path();
        let app = test_app_with_gateway(db_path.clone(), "http://127.0.0.1:1");

        let response = app
            .oneshot(authed_request(Method::GET, "/api/models", None))
            .await
            .unwrap();
        let status = response.status();
        let body = response_json_body(response).await;

        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(
            body["error"],
            "Bifrost is not running. Start it with: windie gateway start"
        );
        let _ = fs::remove_file(db_path);
    }

    fn test_app(store_path: PathBuf) -> Router {
        test_app_with_gateway(store_path, "http://localhost:8080")
    }

    fn test_app_with_gateway(store_path: PathBuf, gateway_url: &str) -> Router {
        test_app_with_urls(store_path, gateway_url, "http://localhost:8080/v1")
    }

    fn test_app_with_urls(store_path: PathBuf, gateway_url: &str, base_url: &str) -> Router {
        let tool_registry = Arc::new(ToolProviderRegistry::with_persistent_mcp_sessions());
        let session_manager = Arc::new(SessionManager::new(
            Some(store_path.clone()),
            gateway_url.to_string(),
            base_url.to_string(),
            tool_registry.clone(),
        ));
        router(ApiState {
            gateway_url: gateway_url.to_string(),
            base_url: base_url.to_string(),
            model: "openai/test".to_string(),
            api_token: "test-token".to_string(),
            store_path: Some(store_path),
            tool_registry,
            session_manager,
        })
    }

    fn test_app_with_tool_registry(
        store_path: PathBuf,
        tool_registry: Arc<ToolProviderRegistry>,
    ) -> Router {
        let session_manager = Arc::new(SessionManager::new(
            Some(store_path.clone()),
            "http://localhost:8080".to_string(),
            "http://localhost:8080/v1".to_string(),
            tool_registry.clone(),
        ));
        router(ApiState {
            gateway_url: "http://localhost:8080".to_string(),
            base_url: "http://localhost:8080/v1".to_string(),
            model: "openai/test".to_string(),
            api_token: "test-token".to_string(),
            store_path: Some(store_path),
            tool_registry,
            session_manager,
        })
    }

    struct MockBifrost {
        gateway_url: String,
        base_url: String,
    }

    async fn spawn_mock_bifrost(response_delay: Duration) -> MockBifrost {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    break;
                };
                tokio::spawn(handle_mock_bifrost_connection(stream, response_delay));
            }
        });

        MockBifrost {
            gateway_url: format!("http://{address}"),
            base_url: format!("http://{address}/v1"),
        }
    }

    async fn handle_mock_bifrost_connection(
        mut stream: tokio::net::TcpStream,
        response_delay: Duration,
    ) {
        let mut buffer = vec![0_u8; 8192];
        let Ok(size) = stream.read(&mut buffer).await else {
            return;
        };
        let request = String::from_utf8_lossy(&buffer[..size]);

        if request.starts_with("GET /health ") {
            write_mock_response(&mut stream, "text/plain", "ok").await;
        } else if request.starts_with("GET /api/models/parameters") {
            write_mock_response(
                &mut stream,
                "application/json",
                r#"{"model_parameters":[],"supports_reasoning":false,"supports_prompt_caching":false}"#,
            )
            .await;
        } else if request.starts_with("POST /v1/responses ") {
            tokio::time::sleep(response_delay).await;
            write_mock_response(
                &mut stream,
                "text/event-stream",
                concat!(
                    "data: {\"type\":\"response.output_text.delta\",\"delta\":\"after disconnect\"}\n\n",
                    "data: {\"type\":\"response.completed\",\"response\":{}}\n\n",
                    "data: [DONE]\n\n"
                ),
            )
            .await;
        } else {
            write_mock_status(&mut stream, 404, "not found").await;
        }
    }

    async fn write_mock_response(
        stream: &mut tokio::net::TcpStream,
        content_type: &str,
        body: &str,
    ) {
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = stream.write_all(response.as_bytes()).await;
    }

    async fn write_mock_status(stream: &mut tokio::net::TcpStream, status: u16, body: &str) {
        let response = format!(
            "HTTP/1.1 {status} Error\r\ncontent-type: text/plain\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = stream.write_all(response.as_bytes()).await;
    }

    fn authed_request(method: Method, uri: &str, body: Option<Value>) -> HttpRequest<Body> {
        let mut builder = HttpRequest::builder()
            .method(method)
            .uri(uri)
            .header(API_TOKEN_HEADER, "test-token");

        let body = match body {
            Some(value) => {
                builder = builder.header(CONTENT_TYPE, "application/json");
                Body::from(serde_json::to_vec(&value).unwrap())
            }
            None => Body::empty(),
        };

        builder.body(body).unwrap()
    }

    async fn response_json(response: Response) -> Value {
        assert!(
            response.status().is_success(),
            "unexpected response status: {}",
            response.status()
        );
        response_json_body(response).await
    }

    async fn response_json_body(response: Response) -> Value {
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    async fn wait_for_session_status(
        db_path: &PathBuf,
        session_id: &SessionId,
        status: SessionStatus,
    ) -> crate::session::Session {
        for _ in 0..50 {
            let store = Store::open_at(db_path).unwrap();
            let session = store.load_session(session_id).unwrap();
            if session.status == status {
                return session;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }

        Store::open_at(db_path)
            .unwrap()
            .load_session(session_id)
            .unwrap()
    }

    fn create_waiting_run(
        db_path: &PathBuf,
        conversation_id: &ConversationId,
        head_message_id: &MessageId,
    ) -> SessionId {
        let mut store = Store::open_at(db_path).unwrap();
        let session_id = SessionId::fresh();
        store
            .create_session(
                &session_id,
                conversation_id,
                Some(head_message_id),
                "openai/test",
                None,
            )
            .unwrap();
        store
            .update_session_status(&session_id, SessionStatus::WaitingForApproval, None)
            .unwrap();
        session_id
    }

    async fn wait_for_run_event_count(
        db_path: &PathBuf,
        session_id: &SessionId,
        count: usize,
    ) -> Vec<SessionEventRecord> {
        for _ in 0..50 {
            let store = Store::open_at(db_path).unwrap();
            let events = store.load_session_events_after(session_id, None).unwrap();
            if events.len() >= count {
                return events;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }

        Store::open_at(db_path)
            .unwrap()
            .load_session_events_after(session_id, None)
            .unwrap()
    }

    fn registry_with_cached_test_tool() -> ToolProviderRegistry {
        ToolProviderRegistry::with_test_mcp_provider(
            "desktop-commander",
            "desktop_commander",
            "Desktop Commander",
            McpCommand {
                program: "windie-test-unused-mcp-provider",
                args: &[],
                env: &[],
            },
            vec![desktop_commander_read_file_definition()],
        )
    }

    fn desktop_commander_read_file_definition() -> ToolDefinition {
        ToolDefinition {
            schema_name: ToolSchemaName::new("desktop_commander__read_file"),
            display_name: "Desktop Commander read_file".to_string(),
            description: "Read a file through Desktop Commander.".to_string(),
            parameters: json!({"type":"object"}),
            provider: ToolProviderRef::new(
                ToolProviderId::new("desktop-commander"),
                ProviderToolName::new("read_file"),
                ToolProviderKind::Mcp,
            ),
            permissions: vec![ToolPermission::ExternalProcess],
            annotations: ToolAnnotations::default(),
        }
    }

    fn insert_multi_tool_call_assistant(db_path: &PathBuf) -> ConversationId {
        let mut store = Store::open_at(db_path).unwrap();
        let conversation_id = store.create_conversation("openai/test").unwrap();
        let user_id = store
            .insert_message(&conversation_id, None, Role::User, "run commands", None)
            .unwrap();
        store
            .insert_message(
                &conversation_id,
                Some(&user_id),
                Role::Assistant,
                "",
                Some(&MessageMetadata {
                    tool_calls: vec![
                        ToolCall::function("call_1", "run_shell", r#"{"command":"printf first"}"#),
                        ToolCall::function("call_2", "run_shell", r#"{"command":"printf second"}"#),
                    ],
                    ..Default::default()
                }),
            )
            .unwrap();

        conversation_id
    }

    fn insert_attached_multi_tool_call_assistant(db_path: &PathBuf) -> ConversationId {
        let mut store = Store::open_at(db_path).unwrap();
        let conversation_id = store.create_conversation("openai/test").unwrap();
        let registry = registry_with_cached_test_tool();
        operation::attach_tool_with_registry(
            &mut store,
            &conversation_id,
            &ToolProviderId::new("desktop-commander"),
            &ProviderToolName::new("read_file"),
            &registry,
        )
        .unwrap();
        let user_id = store
            .insert_message(&conversation_id, None, Role::User, "read files", None)
            .unwrap();
        store
            .insert_message(
                &conversation_id,
                Some(&user_id),
                Role::Assistant,
                "",
                Some(&MessageMetadata {
                    tool_calls: vec![
                        ToolCall::function(
                            "call_1",
                            "desktop_commander__read_file",
                            r#"{"path":"/tmp/one"}"#,
                        ),
                        ToolCall::function(
                            "call_2",
                            "desktop_commander__read_file",
                            r#"{"path":"/tmp/two"}"#,
                        ),
                    ],
                    ..Default::default()
                }),
            )
            .unwrap();

        conversation_id
    }

    fn temp_database_path() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let counter = TEMP_DB_COUNTER.fetch_add(1, Ordering::Relaxed);

        std::env::temp_dir().join(format!(
            "windie-api-{}-{nanos}-{counter}.db",
            std::process::id()
        ))
    }
}
