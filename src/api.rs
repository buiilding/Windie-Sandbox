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
use tokio::sync::broadcast;
use tower_http::cors::CorsLayer;
use tower_http::services::ServeDir;
use uuid::Uuid;

use crate::conversation::{
    ConversationId, ImageAssetId, Message, MessageId, MessageMetadata, MessagePart, Role, ToolCall,
    ToolCallId, ToolSchema, ToolSchemaName,
};
use crate::error::{self, WindieErrorKind};
use crate::gateway::GatewayUrl;
use crate::llm::{BaseUrl, ModelInfo, ModelName, ReasoningRequest};
use crate::operation::{self, InspectionReport, MessageInputPart};
use crate::output::{RuntimeOutput, TerminalOutput};
use crate::paths;
use crate::run::{RunEvent, RunEventEnvelope, RunManager, RunSnapshot, RunSubscription};
use crate::runtime::RuntimeEventSink;
use crate::store::{ConversationInfo, Store};
use crate::tool::{
    ProviderToolName, ToolApprovalMode, ToolApprovalRequest, ToolDefinition, ToolExecutionResult,
    ToolProviderId,
};
use crate::tool_provider::ToolProviderRegistry;

const API_TOKEN_HEADER: &str = "x-windie-api-token";
/// Maximum JSON request body accepted by the localhost API.
///
/// The default Axum body limit is too small for clipboard or local image data
/// sent as base64 message parts. This keeps image input practical while staying
/// bounded for a local developer harness.
const API_JSON_BODY_LIMIT_BYTES: usize = 32 * 1024 * 1024;

/// Runs the local developer API server until the process is stopped.
pub async fn serve(
    address: SocketAddr,
    gateway_url: &str,
    base_url: &str,
    model: &str,
) -> Result<()> {
    let output = TerminalOutput;
    let api_token =
        std::env::var("WINDIE_API_TOKEN").unwrap_or_else(|_| Uuid::new_v4().to_string());
    let run_manager = Arc::new(RunManager::new(None)?);
    let state = ApiState {
        gateway_url: gateway_url.to_string(),
        base_url: base_url.to_string(),
        model: model.to_string(),
        api_token,
        store_path: None,
        tool_registry: Arc::new(ToolProviderRegistry::with_persistent_mcp_sessions()),
        run_manager,
    };
    let listener = TcpListener::bind(address)
        .await
        .with_context(|| format!("failed to bind API server at {address}"))?;

    output.api_started(&address, &state.api_token);
    axum::serve(listener, router(state))
        .await
        .context("api server failed")
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

    let router = Router::new()
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
            "/api/conversations/{conversation_id}/approvals",
            get(list_approvals),
        )
        .route(
            "/api/conversations/{conversation_id}/approvals/{tool_call_id}/approve",
            post(approve_tool),
        )
        .route(
            "/api/conversations/{conversation_id}/approvals/{tool_call_id}/deny",
            post(deny_tool),
        )
        .route(
            "/api/conversations/{conversation_id}/input-tokens",
            post(count_input_tokens),
        )
        .route("/api/conversations/{conversation_id}/query", post(query))
        .route(
            "/api/conversations/{conversation_id}/runs",
            post(start_query_run),
        )
        .route(
            "/api/conversations/{conversation_id}/active-run",
            get(active_conversation_run),
        )
        .route(
            "/api/conversations/{conversation_id}/approvals/{tool_call_id}/approve-run",
            post(start_approve_run),
        )
        .route(
            "/api/conversations/{conversation_id}/approvals/{tool_call_id}/deny-run",
            post(start_deny_run),
        )
        .route("/api/runs/{run_id}", get(get_run))
        .route("/api/runs/{run_id}/events", get(run_events))
        .route("/api/runs/{run_id}/cancel", post(cancel_run))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            require_api_token,
        ))
        .layer(DefaultBodyLimit::max(API_JSON_BODY_LIMIT_BYTES))
        .layer(cors)
        .with_state(state);

    match paths::operator_ui_dir() {
        Some(directory) => {
            router.fallback_service(ServeDir::new(directory).append_index_html_on_directories(true))
        }
        None => router,
    }
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
    run_manager: Arc<RunManager>,
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
}

/// Lists provider tools clients may attach to conversations.
async fn list_tools(State(state): State<ApiState>) -> ApiResult<ToolCatalogResponse> {
    Ok(Json(ToolCatalogResponse {
        tools: operation::available_tools_with_registry(&state.tool_registry)?,
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
    let mut store = open_store(&state)?;

    operation::set_system_prompt(&mut store, &conversation_id, &request.text)?;

    Ok(Json(SystemPromptResponse {
        system_prompt: store.system_prompt(&conversation_id)?,
    }))
}

/// Removes the conversation-level system prompt.
async fn remove_system_prompt(
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
}

impl ToolSchemaRequest {
    /// Converts API JSON into the typed tool schema contract.
    fn into_tool_schema(self) -> ToolSchema {
        ToolSchema {
            name: ToolSchemaName::new(self.name),
            description: self.description,
            parameters: self.parameters,
        }
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
}

#[derive(Debug, Deserialize)]
/// Request body for attaching multiple available provider tools.
struct AttachToolsRequest {
    tools: Vec<AttachToolRequest>,
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
    let mut store = open_store(&state)?;
    let schema_name = operation::attach_tool_with_registry(
        &mut store,
        &conversation_id,
        &provider_id,
        &tool_name,
        &state.tool_registry,
    )?;

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
    let schema_names = operation::attach_tools_with_registry(
        &mut store,
        &conversation_id,
        &requests,
        &state.tool_registry,
    )?;

    Ok(Json(ToolSchemasResponse {
        names: schema_names
            .into_iter()
            .map(|name| name.as_str().to_string())
            .collect(),
    }))
}

/// Inserts one conversation-level tool schema.
async fn insert_tool_schema(
    State(state): State<ApiState>,
    Path(conversation_id): Path<String>,
    Json(request): Json<ToolSchemaRequest>,
) -> ApiResult<ToolSchemaResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let tool_schema = request.into_tool_schema();
    let mut store = open_store(&state)?;

    operation::insert_tool_schema(&mut store, &conversation_id, &tool_schema)?;

    Ok(Json(ToolSchemaResponse {
        name: tool_schema.name.as_str().to_string(),
    }))
}

/// Updates one conversation-level tool schema.
async fn update_tool_schema(
    State(state): State<ApiState>,
    Path((conversation_id, name)): Path<(String, String)>,
    Json(request): Json<ToolSchemaRequest>,
) -> ApiResult<ToolSchemaResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let current_name = ToolSchemaName::new(name);
    let tool_schema = request.into_tool_schema();
    let mut store = open_store(&state)?;

    operation::update_tool_schema(&mut store, &conversation_id, &current_name, &tool_schema)?;

    Ok(Json(ToolSchemaResponse {
        name: tool_schema.name.as_str().to_string(),
    }))
}

/// Removes one conversation-level tool schema.
async fn remove_tool_schema(
    State(state): State<ApiState>,
    Path((conversation_id, name)): Path<(String, String)>,
) -> ApiResult<DeletedResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let name = ToolSchemaName::new(name);
    let mut store = open_store(&state)?;

    operation::remove_tool_schema(&mut store, &conversation_id, &name)?;

    Ok(Json(DeletedResponse { deleted: true }))
}

/// Detaches one provider-backed tool schema from a conversation.
async fn detach_tool(
    State(state): State<ApiState>,
    Path((conversation_id, schema_name)): Path<(String, String)>,
) -> ApiResult<DeletedResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let schema_name = ToolSchemaName::new(schema_name);
    let mut store = open_store(&state)?;

    operation::detach_tool(&mut store, &conversation_id, &schema_name)?;

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
/// Response body for pending tool approvals.
struct ApprovalListResponse {
    approvals: Vec<ApprovalResponse>,
}

#[derive(Debug, Serialize)]
/// One pending approval returned to UI clients.
struct ApprovalResponse {
    assistant_message_id: String,
    tool_call_id: String,
    tool_name: String,
    arguments: String,
    reason: String,
}

impl From<ToolApprovalRequest> for ApprovalResponse {
    fn from(approval: ToolApprovalRequest) -> Self {
        Self {
            assistant_message_id: approval.assistant_message_id.as_str().to_string(),
            tool_call_id: approval.tool_call.id.as_str().to_string(),
            tool_name: approval.tool_call.name().to_string(),
            arguments: approval.tool_call.arguments().to_string(),
            reason: approval.reason,
        }
    }
}

/// Lists pending tool calls waiting for approval.
async fn list_approvals(
    State(state): State<ApiState>,
    Path(conversation_id): Path<String>,
) -> ApiResult<ApprovalListResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let store = open_store(&state)?;
    let approvals = operation::list_tool_approvals_with_registry(
        &store,
        &conversation_id,
        &state.tool_registry,
    )?
    .into_iter()
    .map(ApprovalResponse::from)
    .collect();

    Ok(Json(ApprovalListResponse { approvals }))
}

#[derive(Debug, Serialize)]
/// Response for resolving one pending tool call without continuing the model run.
struct ToolExecutionResponse {
    tool_call_id: String,
    tool_name: String,
    content: String,
    success: bool,
}

impl From<ToolExecutionResult> for ToolExecutionResponse {
    fn from(result: ToolExecutionResult) -> Self {
        Self {
            tool_call_id: result.tool_call_id.as_str().to_string(),
            tool_name: result.tool_name,
            content: result.content,
            success: result.success,
        }
    }
}

/// Executes one approved pending tool call and persists its result.
async fn approve_tool(
    State(state): State<ApiState>,
    Path((conversation_id, tool_call_id)): Path<(String, String)>,
) -> ApiResult<ToolExecutionResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let tool_call_id = ToolCallId::new(tool_call_id);
    let mut store = open_store(&state)?;
    let result = operation::approve_tool_with_registry(
        &mut store,
        &conversation_id,
        &tool_call_id,
        &state.tool_registry,
    )
    .await?;

    Ok(Json(result.into()))
}

/// Stores a rejected result for one pending tool call.
async fn deny_tool(
    State(state): State<ApiState>,
    Path((conversation_id, tool_call_id)): Path<(String, String)>,
) -> ApiResult<ToolExecutionResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let tool_call_id = ToolCallId::new(tool_call_id);
    let mut store = open_store(&state)?;
    let result = operation::deny_tool(&mut store, &conversation_id, &tool_call_id)?;

    Ok(Json(result.into()))
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

#[derive(Debug, Deserialize)]
/// Request body for a one-shot runtime query.
struct QueryRequest {
    model: Option<String>,
    reasoning: Option<ReasoningRequest>,
}

impl QueryRequest {
    /// Returns the optional model override as a typed value.
    fn model_override(&self) -> Option<ModelName> {
        self.model.clone().map(ModelName::new)
    }

    /// Returns a non-empty reasoning request when the client selected one.
    fn reasoning(&self) -> Option<ReasoningRequest> {
        self.reasoning
            .clone()
            .filter(|reasoning| !reasoning.is_empty())
    }
}

/// Runs one model query against the current active path.
async fn query(
    axum::extract::State(state): axum::extract::State<ApiState>,
    Path(conversation_id): Path<String>,
    Json(request): Json<QueryRequest>,
) -> ApiResult<MessageResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let mut store = open_store(&state)?;
    let runtime = runtime_turn_config(&state, request.model_override(), request.reasoning());
    let message = operation::query_conversation_with_registry(
        &ApiOutput,
        &mut store,
        &conversation_id,
        runtime,
    )
    .await?;

    Ok(Json(MessageResponse::from_message(message)))
}

/// Runtime action that can be driven through the shared event stream.
enum RuntimeStreamAction {
    Query {
        conversation_id: ConversationId,
        model_override: Option<ModelName>,
        reasoning: Option<ReasoningRequest>,
    },
    ApproveTool {
        conversation_id: ConversationId,
        tool_call_id: ToolCallId,
    },
    DenyTool {
        conversation_id: ConversationId,
        tool_call_id: ToolCallId,
    },
}

impl RuntimeStreamAction {
    /// Returns the conversation that owns this runtime action.
    fn conversation_id(&self) -> &ConversationId {
        match self {
            Self::Query {
                conversation_id, ..
            }
            | Self::ApproveTool {
                conversation_id, ..
            }
            | Self::DenyTool {
                conversation_id, ..
            } => conversation_id,
        }
    }
}

/// Starts a backend-owned query and returns immediately with its durable ID.
async fn start_query_run(
    State(state): State<ApiState>,
    Path(conversation_id): Path<String>,
    Json(request): Json<QueryRequest>,
) -> ApiResult<RunSnapshot> {
    let snapshot = start_runtime_run(
        state,
        RuntimeStreamAction::Query {
            conversation_id: ConversationId::new(conversation_id),
            model_override: request.model_override(),
            reasoning: request.reasoning(),
        },
    )?;

    Ok(Json(snapshot))
}

/// Starts a backend-owned approval continuation.
async fn start_approve_run(
    State(state): State<ApiState>,
    Path((conversation_id, tool_call_id)): Path<(String, String)>,
) -> ApiResult<RunSnapshot> {
    let snapshot = start_runtime_run(
        state,
        RuntimeStreamAction::ApproveTool {
            conversation_id: ConversationId::new(conversation_id),
            tool_call_id: ToolCallId::new(tool_call_id),
        },
    )?;

    Ok(Json(snapshot))
}

/// Starts a backend-owned denial continuation.
async fn start_deny_run(
    State(state): State<ApiState>,
    Path((conversation_id, tool_call_id)): Path<(String, String)>,
) -> ApiResult<RunSnapshot> {
    let snapshot = start_runtime_run(
        state,
        RuntimeStreamAction::DenyTool {
            conversation_id: ConversationId::new(conversation_id),
            tool_call_id: ToolCallId::new(tool_call_id),
        },
    )?;

    Ok(Json(snapshot))
}

/// Returns current state for a durable run.
async fn get_run(
    State(state): State<ApiState>,
    Path(run_id): Path<String>,
) -> ApiResult<RunSnapshot> {
    Ok(Json(state.run_manager.snapshot(&run_id)?))
}

#[derive(Debug, Serialize)]
/// Nullable active-run response used when an inspector reloads.
struct ActiveRunResponse {
    run: Option<RunSnapshot>,
}

/// Returns the active backend run for one conversation.
async fn active_conversation_run(
    State(state): State<ApiState>,
    Path(conversation_id): Path<String>,
) -> ApiResult<ActiveRunResponse> {
    Ok(Json(ActiveRunResponse {
        run: state
            .run_manager
            .active_for_conversation(&ConversationId::new(conversation_id))?,
    }))
}

/// Explicitly cancels one backend-owned run.
async fn cancel_run(
    State(state): State<ApiState>,
    Path(run_id): Path<String>,
) -> ApiResult<RunSnapshot> {
    Ok(Json(state.run_manager.cancel(&run_id)?))
}

/// Starts one task whose lifetime is independent from HTTP subscribers.
fn start_runtime_run(state: ApiState, action: RuntimeStreamAction) -> Result<RunSnapshot> {
    let snapshot = state.run_manager.begin(action.conversation_id())?;
    let run_id = snapshot.id.clone();
    let task_run_id = run_id.clone();
    let manager = state.run_manager.clone();
    let registration_manager = manager.clone();
    let task = tokio::spawn(async move {
        let result = async {
            let mut store = open_store(&state)?;
            let events = PersistentRunEventSink {
                manager: manager.clone(),
                run_id: task_run_id.clone(),
            };
            let output = PersistentRunOutput {
                manager: manager.clone(),
                run_id: task_run_id.clone(),
            };
            let message = match action {
                RuntimeStreamAction::Query {
                    conversation_id,
                    model_override,
                    reasoning,
                } => {
                    let runtime = runtime_turn_config(&state, model_override, reasoning);
                    operation::query_runtime_turn(
                        &output,
                        &events,
                        &mut store,
                        &conversation_id,
                        runtime,
                    )
                    .await
                    .map(Some)?
                }
                RuntimeStreamAction::ApproveTool {
                    conversation_id,
                    tool_call_id,
                } => {
                    let runtime = runtime_turn_config(&state, None, None);
                    operation::approve_tool_turn(
                        &output,
                        &events,
                        &mut store,
                        &conversation_id,
                        &tool_call_id,
                        runtime,
                    )
                    .await?
                }
                RuntimeStreamAction::DenyTool {
                    conversation_id,
                    tool_call_id,
                } => {
                    let runtime = runtime_turn_config(&state, None, None);
                    operation::deny_tool_turn(
                        &output,
                        &events,
                        &mut store,
                        &conversation_id,
                        &tool_call_id,
                        runtime,
                    )
                    .await?
                }
            };

            Ok::<Option<Message>, anyhow::Error>(message)
        }
        .await;

        match result {
            Ok(message) => {
                let message_id = message
                    .and_then(|message| message.id)
                    .map(|id| id.as_str().to_string());
                if let Err(error) = manager.complete(&task_run_id, message_id) {
                    log_api_error(&error);
                }
            }
            Err(error) => {
                log_api_error(&error);
                if let Err(persist_error) = manager.fail(
                    &task_run_id,
                    raw_error_message(&error),
                    error_causes(&error),
                ) {
                    log_api_error(&persist_error);
                }
            }
        }
    });
    registration_manager.register_task(&run_id, task.abort_handle())?;

    Ok(snapshot)
}

/// Builds shared runtime settings for an API-driven runtime action.
fn runtime_turn_config<'a>(
    state: &'a ApiState,
    model_override: Option<ModelName>,
    reasoning: Option<ReasoningRequest>,
) -> operation::RuntimeTurnConfig<'a> {
    operation::RuntimeTurnConfig::new(
        GatewayUrl::new(state.gateway_url.clone()),
        BaseUrl::new(state.base_url.clone()),
        model_override,
        reasoning,
        state.tool_registry.as_ref(),
    )
}

#[derive(Debug, Deserialize)]
/// Cursor used to replay only events a client has not already rendered.
struct RunEventsQuery {
    #[serde(default)]
    after: u64,
}

/// Replays stored events and then follows the active run until terminal state.
async fn run_events(
    State(state): State<ApiState>,
    Path(run_id): Path<String>,
    Query(query): Query<RunEventsQuery>,
) -> std::result::Result<
    Sse<impl futures_util::Stream<Item = std::result::Result<Event, Infallible>>>,
    ApiError,
> {
    let subscription = state.run_manager.subscribe(&run_id, query.after)?;

    Ok(persistent_run_event_sse(
        subscription,
        state.run_manager,
        run_id,
        query.after,
    ))
}

/// Converts persisted and live run events into reconnectable SSE frames.
fn persistent_run_event_sse(
    subscription: RunSubscription,
    manager: Arc<RunManager>,
    run_id: String,
    after: u64,
) -> Sse<impl futures_util::Stream<Item = std::result::Result<Event, Infallible>>> {
    let stream = stream::unfold(
        PersistentRunSseState {
            pending: VecDeque::from(subscription.history),
            receiver: subscription.receiver,
            manager,
            run_id,
            after,
            terminal_sent: false,
        },
        |mut state| async move {
            loop {
                if state.terminal_sent {
                    return None;
                }

                if let Some(envelope) = state.pending.pop_front() {
                    if envelope.sequence <= state.after {
                        continue;
                    }
                    state.after = envelope.sequence;
                    state.terminal_sent = envelope.event.is_terminal();
                    let event = run_event_frame(&envelope);
                    return Some((Ok::<Event, Infallible>(event), state));
                }

                match state.receiver.recv().await {
                    Ok(envelope) => {
                        if envelope.sequence > state.after {
                            state.pending.push_back(envelope);
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        match state.manager.events_after(&state.run_id, state.after) {
                            Ok(events) => state.pending.extend(events),
                            Err(error) => {
                                state.terminal_sent = true;
                                let envelope = RunEventEnvelope {
                                    run_id: state.run_id.clone(),
                                    sequence: state.after.saturating_add(1),
                                    event: RunEvent::QueryError {
                                        error: raw_error_message(&error),
                                        causes: error_causes(&error),
                                    },
                                };
                                return Some((
                                    Ok::<Event, Infallible>(run_event_frame(&envelope)),
                                    state,
                                ));
                            }
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        match state.manager.events_after(&state.run_id, state.after) {
                            Ok(events) if !events.is_empty() => state.pending.extend(events),
                            _ => return None,
                        }
                    }
                }
            }
        },
    );

    Sse::new(stream).keep_alive(KeepAlive::default())
}

/// Serializes one run event with both SSE and JSON sequence metadata.
fn run_event_frame(envelope: &RunEventEnvelope) -> Event {
    let data = serde_json::to_string(envelope).unwrap_or_else(|error| {
        serde_json::json!({
            "run_id": envelope.run_id,
            "sequence": envelope.sequence,
            "type": "query_error",
            "error": format!("failed to serialize runtime event: {error}"),
            "causes": [format!("failed to serialize runtime event: {error}")],
        })
        .to_string()
    });

    Event::default()
        .id(envelope.sequence.to_string())
        .event(envelope.event.event_name())
        .data(data)
}

/// Subscriber state survives for one HTTP connection but owns no runtime task.
struct PersistentRunSseState {
    pending: VecDeque<RunEventEnvelope>,
    receiver: broadcast::Receiver<RunEventEnvelope>,
    manager: Arc<RunManager>,
    run_id: String,
    after: u64,
    terminal_sent: bool,
}

/// Runtime output sink used by non-streaming API query execution.
///
/// The plain `query` endpoint returns one final JSON message, so live model
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

/// Persists durable message notifications for a backend-owned run.
struct PersistentRunEventSink {
    manager: Arc<RunManager>,
    run_id: String,
}

impl RuntimeEventSink for PersistentRunEventSink {
    fn assistant_message_saved(&self, message_id: &MessageId) {
        if let Err(error) = self.manager.publish(
            &self.run_id,
            RunEvent::AssistantMessageSaved {
                message_id: message_id.as_str().to_string(),
            },
        ) {
            log_api_error(&error);
        }
    }

    fn tool_result_saved(&self, message_id: &MessageId) {
        if let Err(error) = self.manager.publish(
            &self.run_id,
            RunEvent::ToolResultSaved {
                message_id: message_id.as_str().to_string(),
            },
        ) {
            log_api_error(&error);
        }
    }
}

/// Persists live display deltas so a reloaded UI can replay the active output.
struct PersistentRunOutput {
    manager: Arc<RunManager>,
    run_id: String,
}

impl RuntimeOutput for PersistentRunOutput {
    fn start_assistant_message(&self) {}

    fn assistant_delta(&self, text: &str) -> Result<()> {
        self.manager.publish(
            &self.run_id,
            RunEvent::AssistantDelta {
                text: text.to_string(),
            },
        )?;
        Ok(())
    }

    fn reasoning_delta(&self, text: &str) -> Result<()> {
        self.manager.publish(
            &self.run_id,
            RunEvent::ReasoningDelta {
                text: text.to_string(),
            },
        )?;
        Ok(())
    }

    fn tool_call_delta(
        &self,
        index: u16,
        id: Option<&str>,
        name: Option<&str>,
        arguments_delta: Option<&str>,
    ) -> Result<()> {
        self.manager.publish(
            &self.run_id,
            RunEvent::ToolCallDelta {
                index,
                id: id.map(str::to_string),
                name: name.map(str::to_string),
                arguments_delta: arguments_delta.map(str::to_string),
            },
        )?;
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

#[cfg(test)]
#[path = "api_tests.rs"]
mod tests;
