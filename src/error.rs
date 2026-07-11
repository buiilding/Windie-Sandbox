//! Typed Windie errors shared across client boundaries.
//!
//! Runtime and storage code may still use `anyhow::Error` for propagation, but
//! user-facing Windie failures should carry this typed kind when clients need to
//! make a protocol decision such as choosing an HTTP status code.

use std::error::Error;
use std::fmt;

/// Stable category for errors that cross client boundaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindieErrorKind {
    NotFound,
    InvalidRequest,
    Conflict,
}

/// User-facing Windie error with a machine-readable category.
#[derive(Debug)]
pub struct WindieError {
    kind: WindieErrorKind,
    message: String,
}

impl WindieError {
    /// Creates a typed Windie error while preserving the exact display message.
    pub fn new(kind: WindieErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    /// Returns the stable category clients can use for protocol mapping.
    pub fn kind(&self) -> WindieErrorKind {
        self.kind
    }
}

impl fmt::Display for WindieError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for WindieError {}

/// Creates a typed not-found error with the provided raw display message.
pub fn not_found(message: impl Into<String>) -> anyhow::Error {
    WindieError::new(WindieErrorKind::NotFound, message).into()
}

/// Creates a typed invalid-request error with the provided raw display message.
pub fn invalid_request(message: impl Into<String>) -> anyhow::Error {
    WindieError::new(WindieErrorKind::InvalidRequest, message).into()
}

/// Creates a typed conflict error for stale or competing operations.
pub fn conflict(message: impl Into<String>) -> anyhow::Error {
    WindieError::new(WindieErrorKind::Conflict, message).into()
}

/// Finds the first typed Windie error in an anyhow cause chain.
pub fn kind_from_error(error: &anyhow::Error) -> Option<WindieErrorKind> {
    error
        .chain()
        .find_map(|cause| cause.downcast_ref::<WindieError>().map(WindieError::kind))
}
