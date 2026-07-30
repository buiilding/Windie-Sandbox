//! Local developer API server.
//!
//! This module exposes Windie's existing runtime and store primitives over a
//! localhost-only JSON API. It is a test harness boundary for clients such as
//! `windie-inspector`; persistence, context construction, gateway checks, and
//! model requests still flow through the same modules used by the CLI.

use std::collections::VecDeque;
use std::convert::Infallible;
use std::fs;
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

use crate::conversation::{ConversationId, ImageAssetId, MessageId, Role, ToolCallId};
use crate::error::{self as windie_error, WindieErrorKind};
use crate::gateway::GatewayUrl;
use crate::llm::{BaseUrl, InputTokenCount, ModelInfo, ModelName, ReasoningRequest};
use crate::local;
use crate::operation::{self, InspectionReport, MessageInputPart};
use crate::output::TerminalOutput;
use crate::session::{
    Session, SessionEventRecord, SessionId, SessionManager, SessionStatus, SessionSubscription,
};
use crate::store::{ConversationInfo, Store};
use crate::tool::{
    ProviderToolName, ToolApprovalMode, ToolDefinition, ToolProviderId, ToolSchema, ToolSchemaName,
};
use crate::tool_provider::{ToolProviderRegistry, ToolProviderStatus};

mod auth;
mod conversation;
mod env;
mod error;
mod gateway;
mod health;
mod inspection;
mod message;
mod provider;
mod router;
mod session;
mod session_approval;
mod sse;
mod state;
mod tool;
mod ui;

use auth::*;
use conversation::*;
use env::*;
use error::*;
use gateway::*;
use health::*;
use inspection::*;
use message::*;
use provider::*;
use router::router;
use session::*;
use session_approval::*;
use sse::*;
use state::*;
use tool::*;

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
        Err(_) => match local::ensure_api_token() {
            Ok(token) => token,
            Err(error) => {
                cleanup_started_gateway(gateway_start, gateway_url).await;
                return Err(error);
            }
        },
    };
    let tool_registry = Arc::new(ToolProviderRegistry::with_persistent_mcp_sessions());
    let session_manager = Arc::new(SessionManager::new(
        None,
        gateway_url.to_string(),
        base_url.to_string(),
        tool_registry.clone(),
    ));
    if let Err(error) = session_manager.recover_interrupted_sessions() {
        cleanup_started_gateway(gateway_start, gateway_url).await;
        return Err(error);
    }
    let state = ApiState {
        gateway_url: gateway_url.to_string(),
        base_url: base_url.to_string(),
        model: model.to_string(),
        api_token,
        store_path: None,
        tool_registry,
        session_manager,
    };
    let listener = match TcpListener::bind(address).await {
        Ok(listener) => listener,
        Err(error) => {
            if gateway_start == crate::gateway::GatewayStart::Started
                && let Err(cleanup_error) =
                    operation::stop_gateway(GatewayUrl::new(gateway_url)).await
            {
                eprintln!("failed to clean up Bifrost after API bind failure: {cleanup_error}");
            }
            return Err(error).with_context(|| format!("failed to bind API server at {address}"));
        }
    };

    let api_pid_file = match local::windie_home_dir() {
        Ok(home) => home.join("windie-api.pid"),
        Err(error) => {
            cleanup_started_gateway(gateway_start, gateway_url).await;
            return Err(error);
        }
    };
    if let Err(error) = write_process_pid_file(&api_pid_file) {
        cleanup_started_gateway(gateway_start, gateway_url).await;
        return Err(error);
    }

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

    let _ = fs::remove_file(&api_pid_file);

    server_result
}

fn write_process_pid_file(path: &std::path::Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create API PID directory {}", parent.display()))?;
    }
    let temporary = path.with_extension("pid.tmp");
    fs::write(&temporary, format!("{}\n", std::process::id()))
        .with_context(|| format!("failed to write API PID file {}", path.display()))?;
    fs::rename(&temporary, path)
        .with_context(|| format!("failed to publish API PID file {}", path.display()))
}

/// Stops only the Bifrost process this API invocation started.
async fn cleanup_started_gateway(gateway_start: crate::gateway::GatewayStart, gateway_url: &str) {
    if gateway_start == crate::gateway::GatewayStart::Started
        && let Err(error) = operation::stop_gateway(GatewayUrl::new(gateway_url)).await
    {
        eprintln!("failed to clean up Bifrost after API startup failure: {error}");
    }
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

#[cfg(test)]
mod tests;
