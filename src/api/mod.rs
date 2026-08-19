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
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderValue, Method, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use futures_util::stream;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::net::TcpListener;
use tokio::sync::watch;
use tower_http::cors::CorsLayer;

use crate::conversation::{ConversationId, ImageAssetId, MessageId, Role, ToolCallId};
use crate::error::{self as windie_error, WindieErrorKind};
use crate::llm::gateway::GatewayUrl;
use crate::llm::{BaseUrl, InputTokenCount, ModelInfo, ModelName, ReasoningRequest};
use crate::local;
use crate::operation::{self, InspectionReport, MessageInputPart, ToolProviderStatus};
use crate::output::TerminalOutput;
use crate::session::{
    Session, SessionEventRecord, SessionId, SessionManager, SessionStatus, SessionSubscription,
};
use crate::store::{ConversationInfo, Store};
use crate::tool::ToolProviderRegistry;
use crate::tool::{
    ProviderToolName, ToolApprovalMode, ToolDefinition, ToolProviderId, ToolSchema, ToolSchemaName,
};

mod component;
mod conversation;
mod env;
mod error;
mod event;
mod gateway;
mod health;
mod inspection;
mod message;
mod plugin;
mod router;
mod session;
mod session_approval;
mod shutdown;
mod sse;
mod state;
mod tool;

use component::*;
use conversation::*;
use env::*;
use error::*;
use event::*;
use gateway::*;
use health::*;
use inspection::*;
use message::*;
use plugin::*;
use router::router;
use session::*;
use session_approval::*;
use shutdown::*;
use sse::*;
use state::*;
use tool::*;

/// Maximum JSON request body accepted by the localhost API.
///
/// The default Axum body limit is too small for clipboard or local image data
/// sent as base64 message parts. This keeps image input practical while staying
/// bounded for a local developer harness.
const API_JSON_BODY_LIMIT_BYTES: usize = 32 * 1024 * 1024;

/// Sessions the local developer API server until the process is stopped.
pub async fn serve(address: SocketAddr, gateway_url: &str, base_url: &str) -> Result<()> {
    let output = TerminalOutput;
    let tool_registry = Arc::new(ToolProviderRegistry::with_persistent_mcp_sessions());
    let plugin_store = Arc::new(crate::plugin::PluginStore::default_store()?);
    for plugin in plugin_store.installed_plugins()? {
        tool_registry.register_plugin(&plugin)?;
    }
    let marketplace_index_url = crate::config::marketplace_index_url();
    let marketplace_url_for_worker = marketplace_index_url.clone();
    let marketplace = tokio::task::spawn_blocking(move || {
        crate::plugin::MarketplaceInstaller::default()
            .fetch_index(&marketplace_url_for_worker)
            .or_else(|_| crate::plugin::bundled_index())
    })
    .await
    .context("marketplace startup worker failed")??;
    let plugin_catalog = Arc::new(crate::plugin::PluginCatalog::new(
        plugin_store.clone(),
        marketplace,
    ));
    let startup_store = Store::open()?;
    if tool_registry
        .provider_manifest(&ToolProviderId::new("chrome-devtools"))
        .is_some()
    {
        tool_registry.set_chrome_devtools_mode(
            startup_store
                .load_chrome_devtools_mode()?
                .unwrap_or_default(),
        )?;
    }
    let session_manager = Arc::new(
        SessionManager::new(
            None,
            gateway_url.to_string(),
            base_url.to_string(),
            tool_registry.clone(),
        )
        .with_plugin_catalog(plugin_catalog.clone()),
    );
    session_manager.recover_interrupted_sessions()?;
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let state = ApiState {
        gateway_url: gateway_url.to_string(),
        base_url: base_url.to_string(),
        model: None,
        store_path: None,
        marketplace_index_url: Some(marketplace_index_url),
        plugin_store,
        plugin_catalog,
        tool_registry,
        session_manager,
        shutdown_tx: shutdown_tx.clone(),
    };
    let listener = match TcpListener::bind(address).await {
        Ok(listener) => listener,
        Err(error) => {
            return Err(error).with_context(|| format!("failed to bind API server at {address}"));
        }
    };

    let api_pid_file = match local::windie_home_dir() {
        Ok(home) => home.join("windie-api.pid"),
        Err(error) => return Err(error),
    };
    write_process_pid_file(&api_pid_file)?;

    output.api_started(&address);
    let server_result = axum::serve(listener, router(state))
        .with_graceful_shutdown(shutdown_signal(shutdown_rx))
        .await
        .context("api server failed");

    let _ = fs::remove_file(&api_pid_file);

    server_result
}

/// Builds the production route table against an isolated store for local
/// benchmark and integration callers.
pub(crate) fn benchmark_router(store_path: PathBuf) -> Router {
    let tool_registry = Arc::new(ToolProviderRegistry::with_persistent_mcp_sessions());
    let plugin_store = crate::plugin::PluginStore::new(store_path.with_extension("plugins"));
    let plugin_store = Arc::new(plugin_store);
    let plugin_catalog = Arc::new(crate::plugin::PluginCatalog::new(
        plugin_store.clone(),
        crate::plugin::bundled_index().expect("bundled marketplace index should parse"),
    ));
    let startup_store = Store::open_at(&store_path).expect("benchmark store should open");
    if tool_registry
        .provider_manifest(&ToolProviderId::new("chrome-devtools"))
        .is_some()
    {
        tool_registry
            .set_chrome_devtools_mode(
                startup_store
                    .load_chrome_devtools_mode()
                    .expect("benchmark Chrome DevTools settings should load")
                    .unwrap_or_default(),
            )
            .expect("installed Chrome DevTools provider should accept its mode");
    }
    let session_manager = Arc::new(
        SessionManager::new(
            Some(store_path.clone()),
            "http://127.0.0.1:8080".to_string(),
            "http://127.0.0.1:8080/v1".to_string(),
            tool_registry.clone(),
        )
        .with_plugin_catalog(plugin_catalog.clone()),
    );
    let (shutdown_tx, _) = watch::channel(false);
    router(ApiState {
        gateway_url: "http://127.0.0.1:8080".to_string(),
        base_url: "http://127.0.0.1:8080/v1".to_string(),
        model: Some("openai/test".to_string()),
        store_path: Some(store_path),
        marketplace_index_url: None,
        plugin_store,
        plugin_catalog,
        tool_registry,
        session_manager,
        shutdown_tx,
    })
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

/// Waits for the process-level shutdown signal used by the API server.
///
/// Bifrost is independent; stopping this process never changes the gateway.
async fn shutdown_signal(mut shutdown_rx: watch::Receiver<bool>) {
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

    let requested = async move {
        if *shutdown_rx.borrow() {
            return;
        }
        while shutdown_rx.changed().await.is_ok() {
            if *shutdown_rx.borrow() {
                return;
            }
        }
        std::future::pending::<()>().await;
    };

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
        _ = requested => {},
    }
}

#[cfg(test)]
mod tests;
