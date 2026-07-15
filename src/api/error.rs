//! JSON error mapping for the local API boundary.

use super::*;

#[derive(Debug, Serialize)]
/// Stable error response returned by failed API operations.
pub(super) struct ErrorResponse {
    pub(super) error: String,
    pub(super) causes: Vec<String>,
}

/// Error wrapper that maps Windie failures into JSON HTTP responses.
pub(super) struct ApiError(anyhow::Error);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        log_api_error(&self.0);

        let causes = error_causes(&self.0);
        let message = raw_error_message(&self.0);
        let status = match windie_error::kind_from_error(&self.0) {
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

pub(super) type ApiResult<T> = std::result::Result<Json<T>, ApiError>;

/// Prints one API error chain to stderr for local developer visibility.
pub(super) fn log_api_error(error: &anyhow::Error) {
    eprintln!("api error:");
    for cause in error.chain() {
        eprintln!("  {cause}");
    }
}

/// Returns the root cause text that clients should display first.
pub(super) fn raw_error_message(error: &anyhow::Error) -> String {
    error
        .chain()
        .last()
        .map(ToString::to_string)
        .unwrap_or_else(|| error.to_string())
}

/// Returns the full context chain from outer boundary to root cause.
pub(super) fn error_causes(error: &anyhow::Error) -> Vec<String> {
    error.chain().map(ToString::to_string).collect()
}
