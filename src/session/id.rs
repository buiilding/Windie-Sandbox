//! Session identity type.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
/// Stable identifier for one backend-owned runtime session.
pub struct SessionId(String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
/// Stable identifier for one queued input accepted by a runtime session.
pub struct SessionInputId(String);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
/// Unique fencing token for one attempt to execute a durable session.
///
/// A session may be executed repeatedly over its lifetime. Every attempt gets
/// a new claim ID so a cancelled or otherwise superseded runner cannot write
/// through a newer API or CLI claim that happens to have the same owner kind.
pub struct SessionExecutionClaimId(String);

impl SessionExecutionClaimId {
    /// Builds a claim ID from its persisted representation.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Creates a fresh fencing token for one execution attempt.
    pub fn fresh() -> Self {
        Self(Uuid::new_v4().to_string())
    }

    /// Returns the stable SQLite representation.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SessionExecutionClaimId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

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
