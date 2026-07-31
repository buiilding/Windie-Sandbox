//! Standalone Windie Inspector server.
//!
//! The Inspector is an independent localhost process. It serves the compiled
//! browser assets on port 3000 and talks to the Windie API on port 8787. The
//! API does not embed these files or authenticate browser requests with a
//! token; process lifecycle is managed by the CLI.

use std::net::SocketAddr;

use anyhow::{Context, Result};
use axum::extract::Path;
use axum::http::{StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::{Router, serve};
use rust_embed::Embed;
use tokio::net::TcpListener;

const INSPECTOR_ADDRESS: &str = "127.0.0.1:3000";

#[derive(Embed)]
#[folder = "dev/windie-inspector/build"]
struct InspectorAssets;

/// Starts the standalone Inspector HTTP server.
#[tokio::main]
async fn main() -> Result<()> {
    let address = std::env::var("WINDIE_INSPECTOR_ADDRESS")
        .unwrap_or_else(|_| INSPECTOR_ADDRESS.to_string())
        .parse::<SocketAddr>()
        .context("invalid WINDIE_INSPECTOR_ADDRESS")?;
    let listener = TcpListener::bind(address)
        .await
        .with_context(|| format!("failed to bind Inspector at {address}"))?;

    println!("windie inspector listening on http://{address}");
    serve(listener, router())
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("Inspector server failed")
}

fn router() -> Router {
    Router::new()
        .route("/", get(index))
        .route("/static/{*path}", get(static_asset))
        .route("/asset-manifest.json", get(asset_manifest))
        .route("/favicon.ico", get(favicon))
        .route("/manifest.json", get(manifest))
}

async fn index() -> Response {
    serve_index()
}

async fn static_asset(Path(path): Path<String>) -> Response {
    serve_asset(&format!("static/{path}"))
}

async fn asset_manifest() -> Response {
    serve_asset("asset-manifest.json")
}

async fn favicon() -> Response {
    serve_asset("favicon.ico")
}

async fn manifest() -> Response {
    serve_asset("manifest.json")
}

fn serve_index() -> Response {
    match InspectorAssets::get("index.html") {
        Some(content) => Html(content.data.into_owned()).into_response(),
        None => (
            StatusCode::SERVICE_UNAVAILABLE,
            "Inspector build is missing; run npm run build in dev/windie-inspector",
        )
            .into_response(),
    }
}

fn serve_asset(path: &str) -> Response {
    let normalized = path.trim_start_matches('/');
    match InspectorAssets::get(normalized) {
        Some(content) => {
            let mime = mime_guess::from_path(normalized).first_or_octet_stream();
            (
                [(header::CONTENT_TYPE, mime.as_ref())],
                content.data.into_owned(),
            )
                .into_response()
        }
        None => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}

async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(error) = tokio::signal::ctrl_c().await {
            eprintln!("failed to install Inspector Ctrl-C handler: {error}");
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(error) => {
                eprintln!("failed to install Inspector terminate handler: {error}");
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
