//! Local developer API server.
//!
//! This module exposes Windie's existing runtime and store primitives over a
//! localhost-only JSON API. It is a test harness boundary for clients such as
//! `windie-inspector`; persistence, context construction, gateway checks, and
//! model requests still flow through the same modules used by the CLI.

use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};
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

use crate::context::{ContextBuilder, ContextParts};
use crate::conversation::{
    ConversationId, Message, MessageId, MessageMetadata, MessagePart, Role, ToolCall, ToolCallId,
    ToolSchema, ToolSchemaName,
};
use crate::gateway::BifrostGateway;
use crate::image_input::read_image_input;
use crate::llm::{BaseUrl, BifrostClient, ModelName};
use crate::output::{InspectionReport, RuntimeOutput};
use crate::runtime::{
    approve_tool_call, deny_tool_call, pending_tool_approvals, query_conversation_once,
};
use crate::store::{ConversationInfo, ImagePayload, MessagePayload, Store};
use crate::tool::{ToolApprovalRequest, ToolExecutionResult};
use crate::tool_catalog::available_tool_schemas;

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
    };
    let listener = TcpListener::bind(address)
        .await
        .with_context(|| format!("failed to bind API server at {address}"))?;

    println!("windie api listening on http://{address}");
    println!("windie api token: {}", state.api_token);
    axum::serve(listener, router(state))
        .await
        .context("api server failed")
}

/// Builds the route table for the local API surface.
///
/// Each handler opens the SQLite store directly and calls the same primitive
/// methods used by the CLI. The router only owns HTTP mapping.
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
}

#[derive(Debug, Serialize)]
/// Stable error response returned by failed API operations.
struct ErrorResponse {
    error: String,
}

/// Error wrapper that maps Windie failures into JSON HTTP responses.
struct ApiError(anyhow::Error);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let message = self.0.to_string();
        let status = if message.contains("does not exist") {
            StatusCode::NOT_FOUND
        } else if message.contains("invalid")
            || message.contains("requires")
            || message.contains("missing")
        {
            StatusCode::BAD_REQUEST
        } else {
            StatusCode::INTERNAL_SERVER_ERROR
        };

        (status, Json(ErrorResponse { error: message })).into_response()
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(error: anyhow::Error) -> Self {
        Self(error)
    }
}

type ApiResult<T> = std::result::Result<Json<T>, ApiError>;

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
/// API response for Windie's built-in tool catalog.
struct ToolCatalogResponse {
    tools: Vec<ToolSchema>,
}

/// Lists built-in tool schemas clients may attach to conversations.
async fn list_tools() -> ApiResult<ToolCatalogResponse> {
    Ok(Json(ToolCatalogResponse {
        tools: available_tool_schemas(),
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
    let gateway = BifrostGateway::new(crate::gateway::GatewayUrl::new(state.gateway_url));

    Ok(Json(StatusResponse {
        gateway_running: gateway.is_running().await,
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
    let gateway = BifrostGateway::new(crate::gateway::GatewayUrl::new(state.gateway_url));
    let status = gateway.start().await.context("failed to start gateway")?;
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
    let gateway = BifrostGateway::new(crate::gateway::GatewayUrl::new(state.gateway_url));
    let status = gateway.stop().await.context("failed to stop gateway")?;
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
async fn list_conversations() -> ApiResult<ConversationListResponse> {
    let store = Store::open().context("failed to open store")?;
    let conversations = store
        .list_conversations()
        .context("failed to list conversations")?
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
async fn create_conversation() -> ApiResult<ConversationIdResponse> {
    let store = Store::open().context("failed to open store")?;
    let conversation_id = store
        .create_conversation()
        .context("failed to create conversation")?;

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
    let report = build_inspection_report(&conversation_id, &model)?;

    Ok(Json(report))
}

#[derive(Debug, Deserialize)]
/// Optional query parameters for inspection.
struct InspectQuery {
    model: Option<String>,
}

/// Builds the same inspection report used by the CLI JSON output.
fn build_inspection_report(
    conversation_id: &ConversationId,
    model: &str,
) -> Result<InspectionReport> {
    let store = Store::open().context("failed to open store")?;
    let active_message_id = store
        .active_message_id(conversation_id)
        .context("failed to load active message")?;
    let messages = store
        .load_message_tree(conversation_id)
        .with_context(|| format!("failed to inspect conversation tree {conversation_id}"))?;
    let tool_schemas = store
        .load_tool_schemas(conversation_id)
        .context("failed to load tool schemas")?;
    let context_parts = ContextBuilder::load_parts(&store, conversation_id)
        .context("failed to load model context parts")?;
    let model_context = ContextBuilder::flatten(ContextParts {
        active_path: context_parts.active_path.clone(),
        system_prompt: context_parts.system_prompt.clone(),
        compaction: context_parts.compaction.clone(),
    });

    Ok(InspectionReport::new(
        conversation_id,
        active_message_id.as_ref(),
        model,
        context_parts.system_prompt,
        tool_schemas,
        messages,
        context_parts.active_path,
        model_context,
        context_parts.compaction,
    ))
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
    Path(conversation_id): Path<String>,
    Json(request): Json<MessageIdRequest>,
) -> ApiResult<ActiveMessageResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let message_id = MessageId::new(request.message_id);
    let mut store = Store::open().context("failed to open store")?;

    store
        .set_active_message(&conversation_id, &message_id)
        .context("failed to activate message")?;

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
    Path(conversation_id): Path<String>,
    Json(request): Json<InsertMessageRequest>,
) -> ApiResult<MessageIdResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let mut store = Store::open().context("failed to open store")?;
    let parent_message_id = store
        .active_message_id(&conversation_id)
        .context("failed to load active message")?;
    let parts = normalize_insert_parts(request.text, request.parts)?;
    let has_image = parts
        .iter()
        .any(|part| matches!(part, LoadedInsertPart::Image(_)));
    let content = insert_content(&parts);
    let message_id = if has_image || parts.len() > 1 {
        if request.role != Role::User {
            return Err(anyhow!("multi-part input is only supported for user messages").into());
        }
        let payloads = parts
            .iter()
            .map(|part| match part {
                LoadedInsertPart::Text(text) => MessagePayload::Text(text.as_str()),
                LoadedInsertPart::Image(image) => MessagePayload::Image(ImagePayload {
                    mime_type: image.mime_type.as_str(),
                    bytes: image.bytes.as_slice(),
                }),
            })
            .collect::<Vec<_>>();

        store
            .insert_user_message_with_parts(
                &conversation_id,
                parent_message_id.as_ref(),
                &content,
                &payloads,
            )
            .context("failed to insert multi-part message")?
    } else {
        store
            .insert_message(
                &conversation_id,
                parent_message_id.as_ref(),
                request.role,
                &content,
                None,
            )
            .context("failed to insert message")?
    };

    Ok(Json(MessageIdResponse {
        message_id: message_id.as_str().to_string(),
    }))
}

/// Loaded API message part ready to become store payloads.
enum LoadedInsertPart {
    Text(String),
    Image(crate::image_input::ImageInput),
}

/// Converts request text and part fields into one ordered part list.
fn normalize_insert_parts(
    text: Option<String>,
    parts: Vec<InsertMessagePart>,
) -> Result<Vec<LoadedInsertPart>> {
    let parts = if parts.is_empty() {
        text.map(|text| vec![InsertMessagePart::Text { text }])
            .unwrap_or_default()
    } else {
        parts
    };

    if parts.is_empty() {
        return Err(anyhow!("message requires text or parts"));
    }

    let loaded = parts
        .into_iter()
        .map(|part| match part {
            InsertMessagePart::Text { text } => Ok(LoadedInsertPart::Text(text)),
            InsertMessagePart::Image { path } => {
                read_image_input(&path).map(LoadedInsertPart::Image)
            }
        })
        .collect::<Result<Vec<_>>>()?;

    if loaded
        .iter()
        .all(|part| matches!(part, LoadedInsertPart::Text(text) if text.is_empty()))
    {
        return Err(anyhow!("message requires non-empty text or an image"));
    }

    Ok(loaded)
}

/// Builds the plain text preview for one stored message.
fn insert_content(parts: &[LoadedInsertPart]) -> String {
    parts
        .iter()
        .filter_map(|part| match part {
            LoadedInsertPart::Text(text) => Some(text.as_str()),
            LoadedInsertPart::Image(_) => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[derive(Debug, Deserialize)]
/// Request body for replacing one message.
struct UpdateMessageRequest {
    text: String,
}

/// Replaces one message's text content.
async fn update_message(
    Path((conversation_id, message_id)): Path<(String, String)>,
    Json(request): Json<UpdateMessageRequest>,
) -> ApiResult<MessageIdResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let message_id = MessageId::new(message_id);
    let mut store = Store::open().context("failed to open store")?;

    store
        .replace_message(&conversation_id, &message_id, &request.text)
        .context("failed to update message")?;

    Ok(Json(MessageIdResponse {
        message_id: message_id.as_str().to_string(),
    }))
}

/// Removes one message and its descendants.
async fn remove_message(
    Path((conversation_id, message_id)): Path<(String, String)>,
) -> ApiResult<DeletedResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let message_id = MessageId::new(message_id);
    let mut store = Store::open().context("failed to open store")?;

    store
        .remove_message(&conversation_id, &message_id)
        .context("failed to remove message")?;

    Ok(Json(DeletedResponse { deleted: true }))
}

#[derive(Debug, Serialize)]
/// Generic deletion response.
struct DeletedResponse {
    deleted: bool,
}

/// Removes one conversation and all owned persisted data.
async fn remove_conversation(Path(conversation_id): Path<String>) -> ApiResult<DeletedResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let mut store = Store::open().context("failed to open store")?;

    store
        .remove_conversation(&conversation_id)
        .context("failed to remove conversation")?;

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
    Path(conversation_id): Path<String>,
    Json(request): Json<SystemPromptRequest>,
) -> ApiResult<SystemPromptResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let mut store = Store::open().context("failed to open store")?;

    store
        .set_system_prompt(&conversation_id, &request.text)
        .context("failed to set system prompt")?;

    Ok(Json(SystemPromptResponse {
        system_prompt: store.system_prompt(&conversation_id)?,
    }))
}

/// Removes the conversation-level system prompt.
async fn remove_system_prompt(
    Path(conversation_id): Path<String>,
) -> ApiResult<SystemPromptResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let mut store = Store::open().context("failed to open store")?;

    store
        .remove_system_prompt(&conversation_id)
        .context("failed to remove system prompt")?;

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

/// Inserts one conversation-level tool schema.
async fn insert_tool_schema(
    Path(conversation_id): Path<String>,
    Json(request): Json<ToolSchemaRequest>,
) -> ApiResult<ToolSchemaResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let tool_schema = request.into_tool_schema();
    let mut store = Store::open().context("failed to open store")?;

    store
        .insert_tool_schema(&conversation_id, &tool_schema)
        .context("failed to insert tool schema")?;

    Ok(Json(ToolSchemaResponse {
        name: tool_schema.name.as_str().to_string(),
    }))
}

/// Updates one conversation-level tool schema.
async fn update_tool_schema(
    Path((conversation_id, name)): Path<(String, String)>,
    Json(request): Json<ToolSchemaRequest>,
) -> ApiResult<ToolSchemaResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let current_name = ToolSchemaName::new(name);
    let tool_schema = request.into_tool_schema();
    let mut store = Store::open().context("failed to open store")?;

    store
        .update_tool_schema(&conversation_id, &current_name, &tool_schema)
        .context("failed to update tool schema")?;

    Ok(Json(ToolSchemaResponse {
        name: tool_schema.name.as_str().to_string(),
    }))
}

/// Removes one conversation-level tool schema.
async fn remove_tool_schema(
    Path((conversation_id, name)): Path<(String, String)>,
) -> ApiResult<DeletedResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let name = ToolSchemaName::new(name);
    let mut store = Store::open().context("failed to open store")?;

    store
        .remove_tool_schema(&conversation_id, &name)
        .context("failed to remove tool schema")?;

    Ok(Json(DeletedResponse { deleted: true }))
}

/// Deletes descendants after one checkpoint message.
async fn truncate_conversation(
    Path(conversation_id): Path<String>,
    Json(request): Json<MessageIdRequest>,
) -> ApiResult<ActiveMessageResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let message_id = MessageId::new(request.message_id);
    let mut store = Store::open().context("failed to open store")?;

    store
        .truncate_after_message(&conversation_id, &message_id)
        .context("failed to truncate conversation")?;

    Ok(Json(ActiveMessageResponse {
        active_message_id: store
            .active_message_id(&conversation_id)?
            .map(|id| id.as_str().to_string())
            .unwrap_or_default(),
    }))
}

/// Creates a new conversation copied through a checkpoint message.
async fn fork_conversation(
    Path(conversation_id): Path<String>,
    Json(request): Json<MessageIdRequest>,
) -> ApiResult<ConversationIdResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let message_id = MessageId::new(request.message_id);
    let mut store = Store::open().context("failed to open store")?;
    let forked_conversation_id = store
        .fork_conversation_at_message(&conversation_id, &message_id)
        .context("failed to fork conversation")?;

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

/// Lists unresolved tool calls waiting for approval.
async fn list_approvals(Path(conversation_id): Path<String>) -> ApiResult<ApprovalListResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let store = Store::open().context("failed to open store")?;
    let approvals = pending_tool_approvals(&store, &conversation_id)
        .context("failed to load pending approvals")?
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
    Path((conversation_id, tool_call_id)): Path<(String, String)>,
) -> ApiResult<ToolResultResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let tool_call_id = ToolCallId::new(tool_call_id);
    let mut store = Store::open().context("failed to open store")?;
    let result = approve_tool_call(&mut store, &conversation_id, &tool_call_id)
        .await
        .context("failed to approve tool call")?;

    Ok(Json(ToolResultResponse::from(result)))
}

/// Stores a rejected result for one pending tool call.
async fn deny_tool(
    Path((conversation_id, tool_call_id)): Path<(String, String)>,
) -> ApiResult<ToolResultResponse> {
    let conversation_id = ConversationId::new(conversation_id);
    let tool_call_id = ToolCallId::new(tool_call_id);
    let mut store = Store::open().context("failed to open store")?;
    let result = deny_tool_call(&mut store, &conversation_id, &tool_call_id)
        .context("failed to deny tool call")?;

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
    let gateway = BifrostGateway::new(crate::gateway::GatewayUrl::new(state.gateway_url));
    gateway
        .require_running()
        .await
        .context("failed to prepare Bifrost gateway")?;
    let model = request.model.unwrap_or(state.model);
    let llm = BifrostClient::new(BaseUrl::new(state.base_url), ModelName::new(model));
    let mut store = Store::open().context("failed to open store")?;
    let message = query_conversation_once(&ApiOutput, &llm, &mut store, &conversation_id)
        .await
        .context("failed to query conversation")?;

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
