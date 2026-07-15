//! Health and runtime status API handlers.

use super::*;

#[derive(Debug, Serialize)]
/// Health payload for UI startup checks.
pub(super) struct HealthResponse {
    pub(super) ok: bool,
}

/// Confirms that the API server process is reachable.
pub(super) async fn health() -> ApiResult<HealthResponse> {
    Ok(Json(HealthResponse { ok: true }))
}

#[derive(Debug, Serialize)]
/// Local runtime readiness as seen from the API process.
pub(super) struct StatusResponse {
    pub(super) gateway_running: bool,
}

/// Returns current local gateway readiness.
pub(super) async fn status(
    axum::extract::State(state): axum::extract::State<ApiState>,
) -> ApiResult<StatusResponse> {
    Ok(Json(StatusResponse {
        gateway_running: operation::gateway_status(GatewayUrl::new(state.gateway_url)).await,
    }))
}
