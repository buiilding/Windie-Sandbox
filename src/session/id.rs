//! Session identity type.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
/// Stable identifier for one backend-owned runtime session.
pub struct SessionId(String);

impl SessionId {
    /// Creates a fresh session ID.
    pub fn fresh() -> Self {
        Self(Uuid::new_v4().to_string())
    }

    /// Wraps raw ID text from API or storage.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Exposes the ID at persistence and protocol boundaries.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SessionId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}
