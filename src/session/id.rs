//! Session identity type.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
/// Stable identifier for one backend-owned runtime session.
pub struct SessionId(String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
/// Stable identifier for one queued input accepted by a runtime session.
pub struct SessionInputId(String);

impl SessionInputId {
    /// Builds an input ID from its persisted representation.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Creates a fresh input ID.
    pub fn fresh() -> Self {
        Self(Uuid::new_v4().to_string())
    }

    /// Returns the stable string representation.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SessionInputId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

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
