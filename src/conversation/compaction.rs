//! Conversation compaction identifiers.
//!
//! Compaction content and persistence live in `store.rs`; this module only
//! defines the typed ID shared across storage and output boundaries.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
/// Stable identifier for a saved history compaction checkpoint.
pub struct CompactionId(String);

impl CompactionId {
    /// Wraps raw ID text so compaction identity stays separate from messages and
    /// conversations.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Exposes the stored ID as plain text at persistence and display
    /// boundaries.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for CompactionId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl std::ops::Deref for CompactionId {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}
