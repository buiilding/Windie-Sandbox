//! Local API shutdown handling.
//!
//! This module owns the localhost route that requests a graceful API stop.
//! It only signals the server task; `api::serve` remains responsible for
//! draining Axum without changing the independent Bifrost process.

use super::*;

#[derive(Debug, Serialize)]
/// Result returned after a graceful shutdown request was accepted.
pub(super) struct ShutdownResponse {
    pub(super) stopping: bool,
}

/// Requests graceful shutdown of the Windie API process.
pub(super) async fn shutdown(State(state): State<ApiState>) -> ApiResult<ShutdownResponse> {
    let _ = state.shutdown_tx.send(true);
    Ok(Json(ShutdownResponse { stopping: true }))
}
