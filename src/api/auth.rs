//! Local API token middleware.

use super::{
    API_TOKEN_HEADER, ApiState, ErrorResponse, IntoResponse, Json, Method, Next, Request, Response,
    State, StatusCode,
};

/// Requires the current local API token before executing non-health requests.
///
/// The browser UI sends this token in `X-Windie-Api-Token`. Preflight requests
/// and health checks stay open so clients can detect that the server exists
/// before they have a token configured.
pub(super) async fn require_api_token(
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
