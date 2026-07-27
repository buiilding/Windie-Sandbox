//! Embedded static UI served by the local API server.
//!
//! The production inspector is a Create React App build that references its
//! assets with absolute paths (`/static/...`). This module embeds that build
//! into the `windie` binary and serves it from the server root so those paths
//! resolve without a dev server. UI assets are intentionally unauthenticated —
//! the browser must load them before it can attach the API token to `/api/*`
//! requests.

use axum::extract::Path;
use axum::http::{StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use rust_embed::Embed;

#[derive(Embed)]
#[folder = "dev/windie-inspector/build"]
struct InspectorAssets;

/// Serves `index.html` at the server root so React Router can take over.
pub(super) async fn index() -> Response {
    serve_index()
}

/// Serves one embedded asset under `static/`. The wildcard captures the path
/// after `/static/`, so the `static/` prefix is restored before lookup.
pub(super) async fn static_asset(Path(path): Path<String>) -> Response {
    serve_asset(&format!("static/{path}"))
}

/// Serves the build's asset manifest at the root.
pub(super) async fn asset_manifest() -> Response {
    serve_asset("asset-manifest.json")
}

/// Serves the favicon at the root.
pub(super) async fn favicon() -> Response {
    serve_asset("favicon.ico")
}

/// Serves the web app manifest at the root.
pub(super) async fn manifest() -> Response {
    serve_asset("manifest.json")
}

/// Renders the embedded app shell, or a 404 when the build was not embedded.
fn serve_index() -> Response {
    match InspectorAssets::get("index.html") {
        Some(content) => Html(content.data.into_owned()).into_response(),
        None => (
            StatusCode::SERVICE_UNAVAILABLE,
            "inspector build not embedded; run npm run build in dev/windie-inspector",
        )
            .into_response(),
    }
}

/// Looks up one embedded file and maps it to an HTTP response with a best-effort
/// content type. Asset requests that miss return a real 404 so broken imports
/// stay visible instead of silently serving the app shell.
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
