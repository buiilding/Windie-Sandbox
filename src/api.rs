//! Local developer API server.
//!
//! This module exposes Windie's existing runtime and store primitives over a
//! localhost-only JSON API. It is a test harness boundary for clients such as
//! `windie-inspector`; persistence, context construction, gateway checks, and
//! model requests still flow through the same modules used by the CLI.

mod auth;
mod conversations;
mod gateway;
mod runs;
mod tools;

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
use crate::store::{ConversationInfo, RuntimeRunAction, Store};
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
        .merge(gateway::routes())
        .merge(tools::routes())
        .merge(conversations::routes())
        .merge(runs::routes())
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth::require_api_token,
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

/// Runs one complete synchronous store operation on Tokio's blocking pool.
/// The store is opened inside the closure so its SQLite connection never
/// crosses an async suspension point or occupies an API runtime worker.
async fn run_store<T, F>(state: &ApiState, operation: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce(&mut Store) -> Result<T> + Send + 'static,
{
    let state = state.clone();
    tokio::task::spawn_blocking(move || {
        let mut store = open_store(&state)?;
        operation(&mut store)
    })
    .await
    .context("store operation task stopped")?
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
            Some(WindieErrorKind::Conflict) => StatusCode::CONFLICT,
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

#[derive(Debug, Serialize)]
/// Generic deletion response shared by conversation and tool routes.
struct DeletedResponse {
    deleted: bool,
}

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

/// Builds shared runtime settings for an API-driven runtime action.
fn runtime_turn_config<'a>(
    state: &'a ApiState,
    run_id: &'a str,
    model_override: Option<ModelName>,
    reasoning: Option<ReasoningRequest>,
) -> Result<operation::RuntimeTurnConfig<'a>> {
    Ok(operation::RuntimeTurnConfig::new(
        run_id,
        state.run_manager.cancellation(run_id)?,
        GatewayUrl::new(state.gateway_url.clone()),
        BaseUrl::new(state.base_url.clone()),
        model_override,
        reasoning,
        state.tool_registry.as_ref(),
    ))
}

#[cfg(test)]
mod tests;
