//! Local developer API server.
//!
//! This module exposes Windie's existing runtime and store primitives over a
//! localhost-only JSON API. It is a test harness boundary for clients such as
//! `windie-inspector`; persistence, context construction, gateway checks, and
//! model requests still flow through the same modules used by the CLI.

use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{Context, Result};
use axum::extract::{Path, Request, State};
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderName, HeaderValue, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::net::TcpListener;
use tower_http::cors::CorsLayer;
use uuid::Uuid;

use crate::conversation::{
    ConversationId, Message, MessageId, MessageMetadata, MessagePart, Role, ToolCall, ToolCallId,
    ToolSchema, ToolSchemaName,
};
use crate::error::{self, WindieErrorKind};
use crate::gateway::GatewayUrl;
use crate::llm::ModelName;
use crate::operation::{self, InspectionReport, MessageInputPart};
use crate::output::{RuntimeOutput, TerminalOutput};
use crate::store::{ConversationInfo, Store};
use crate::tool::{
    ProviderToolName, ToolApprovalRequest, ToolDefinition, ToolExecutionResult, ToolProviderId,
};

const API_TOKEN_HEADER: &str = "x-windie-api-token";

/// Runs the local developer API server until the process is stopped.
pub async fn serve(
    address: SocketAddr,
    gateway_url: &str,
    base_url: &str,
    model: &str,
) -> Result<()> {
    let api_token =
        std::env::var("WINDIE_API_TOKEN").unwrap_or_else(|_| Uuid::new_v4().to_string());
    let state = ApiState {
        gateway_url: gateway_url.to_string(),
        base_url: base_url.to_string(),
        model: model.to_string(),
        api_token,
        store_path: None,
    };
    let listener = TcpListener::bind(address)
        .await
        .with_context(|| format!("failed to bind API server at {address}"))?;

    let output = TerminalOutput;
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

    Router::new()
        .route("/api/health", get(health))
        .route("/api/status", get(status))
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
            "/api/conversations/{conversation_id}/system-prompt",
            patch(set_system_prompt).delete(remove_system_prompt),
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
        .route("/api/conversations/{conversation_id}/query", post(query))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            require_api_token,
        ))
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
async fn list_tools() -> ApiResult<ToolCatalogResponse> {
    Ok(Json(ToolCatalogResponse {
        tools: operation::available_tools(),
    }))
}

/// Lists available tools for one provider.
async fn list_provider_tools(Path(provider_id): Path<String>) -> ApiResult<ToolCatalogResponse> {
    let provider_id = ToolProviderId::new(provider_id);

    Ok(Json(ToolCatalogResponse {
        tools: operation::available_provider_tools(&provider_id),
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
    message_count: i64,
}

impl From<ConversationInfo> for ConversationSummary {
    fn from(info: ConversationInfo) -> Self {
        Self {
            id: info.id.as_str().to_string(),
            title: info.title,
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
    let conversation_id = operation::create_conversation(&store)?;

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
    let model = query.model.clone().unwrap_or_else(|| state.model.clone());
    let store = open_store(&state)?;
    let report = operation::inspect_conversation(&store, &conversation_id, &ModelName::new(model))?;

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
            InsertMessagePart::Image { path } => Ok(MessageInputPart::Image(path)),
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

#[derive(Debug, Deserialize)]
/// Request body for attaching an available provider tool to a conversation.
struct AttachToolRequest {
    provider_id: String,
    tool_name: String,
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
    let schema_name =
        operation::attach_tool(&mut store, &conversation_id, &provider_id, &tool_name)?;

    Ok(Json(ToolSchemaResponse {
        name: schema_name.as_str().to_string(),
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
    let approvals = operation::list_tool_approvals(&store, &conversation_id)?
        .into_iter()
        .map(ApprovalResponse::from)
        .collect();

    Ok(Json(ApprovalListResponse { approvals }))
}

#[derive(Debug, Serialize)]
/// Response for approval resolution endpoints.
struct ToolResultResponse {
    tool_call_id: String,
    tool_name: String,
    content: String,
    success: bool,
}

impl From<ToolExecutionResult> for ToolResultResponse {
    fn from(result: ToolExecutionResult) -> Self {
        Self {
            tool_call_id: result.tool_call_id.as_str().to_string(),
            tool_name: result.tool_name,
            content: result.content,
            success: result.success,
        }
    }
}

/// Executes one approved pending tool call.
async fn approve_tool(
    State(state): State<ApiState>,
    Path((conversation_id, tool_call_id)): Path<(String, String)>,
) -> ApiResult<ToolResultResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let tool_call_id = ToolCallId::new(tool_call_id);
    let mut store = open_store(&state)?;
    let result = operation::approve_tool(&mut store, &conversation_id, &tool_call_id).await?;

    Ok(Json(ToolResultResponse::from(result)))
}

/// Stores a rejected result for one pending tool call.
async fn deny_tool(
    State(state): State<ApiState>,
    Path((conversation_id, tool_call_id)): Path<(String, String)>,
) -> ApiResult<ToolResultResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let tool_call_id = ToolCallId::new(tool_call_id);
    let mut store = open_store(&state)?;
    let result = operation::deny_tool(&mut store, &conversation_id, &tool_call_id)?;

    Ok(Json(ToolResultResponse::from(result)))
}

#[derive(Debug, Deserialize)]
/// Request body for a one-shot runtime query.
struct QueryRequest {
    model: Option<String>,
}

/// Runs one model query against the current active path.
async fn query(
    axum::extract::State(state): axum::extract::State<ApiState>,
    Path(conversation_id): Path<String>,
    Json(request): Json<QueryRequest>,
) -> ApiResult<MessageResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let mut store = open_store(&state)?;
    let model = request.model.unwrap_or_else(|| state.model.clone());
    let message = operation::query_conversation(
        &ApiOutput,
        &mut store,
        &conversation_id,
        GatewayUrl::new(state.gateway_url.clone()),
        crate::llm::BaseUrl::new(state.base_url.clone()),
        ModelName::new(model),
    )
    .await?;

    Ok(Json(MessageResponse::from_message(message)))
}

/// Runtime output sink used by API query execution.
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{Body, to_bytes};
    use axum::http::Request as HttpRequest;
    use serde_json::json;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    use tower::ServiceExt;

    static TEMP_DB_COUNTER: AtomicU64 = AtomicU64::new(0);

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
    async fn remove_message_returns_raw_delete_policy_error() {
        let db_path = temp_database_path();
        let app = test_app(db_path.clone());
        let mut store = Store::open_at(&db_path).unwrap();
        let conversation_id = store.create_conversation().unwrap();
        let parent_id = store
            .insert_message(&conversation_id, None, Role::User, "one", None)
            .unwrap();
        let metadata = MessageMetadata {
            tool_call_id: Some(ToolCallId::new("call_1")),
            ..Default::default()
        };
        let message_id = store
            .insert_message(
                &conversation_id,
                Some(&parent_id),
                Role::Tool,
                "{}",
                Some(&metadata),
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
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(
            body["error"],
            "cannot remove role: tool message because its parent is not an assistant tool-call message"
        );
        assert_eq!(
            body["causes"].as_array().unwrap().last().unwrap(),
            "cannot remove role: tool message because its parent is not an assistant tool-call message"
        );
        let _ = fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn approve_later_multi_tool_call_returns_raw_order_error() {
        let db_path = temp_database_path();
        let app = test_app(db_path.clone());
        let conversation_id = insert_multi_tool_call_assistant(&db_path);

        let response = app
            .oneshot(authed_request(
                Method::POST,
                &format!("/api/conversations/{conversation_id}/approvals/call_2/approve"),
                None,
            ))
            .await
            .unwrap();
        let status = response.status();
        let body = response_json_body(response).await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(
            body["error"],
            "tool call must be resolved after previous tool call: call_1"
        );
        let _ = fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn query_with_unresolved_tool_call_returns_gateway_error_before_runtime_query() {
        let db_path = temp_database_path();
        let app = test_app_with_gateway(db_path.clone(), "http://127.0.0.1:1");
        let conversation_id = insert_multi_tool_call_assistant(&db_path);

        let response = app
            .oneshot(authed_request(
                Method::POST,
                &format!("/api/conversations/{conversation_id}/query"),
                Some(json!({"model":"openai/test"})),
            ))
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
        router(ApiState {
            gateway_url: gateway_url.to_string(),
            base_url: "http://localhost:8080/v1".to_string(),
            model: "openai/test".to_string(),
            api_token: "test-token".to_string(),
            store_path: Some(store_path),
        })
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

    fn insert_multi_tool_call_assistant(db_path: &PathBuf) -> ConversationId {
        let mut store = Store::open_at(db_path).unwrap();
        let conversation_id = store.create_conversation().unwrap();
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
