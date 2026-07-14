//! Model-facing tool schema data.
//!
//! These types describe tools as the model sees them. Attachment, approval,
//! execution, and provider mapping live outside the conversation model.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
/// Stable name for one path-scoped tool schema.
pub struct ToolSchemaName(String);

impl ToolSchemaName {
    /// Wraps raw tool schema name text so tool schema identity stays
    /// type-distinct from general strings.
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    /// Exposes the tool schema name as plain text at persistence, request, and
    /// display boundaries.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Returns whether the name is valid for OpenAI-compatible function tool
    /// schemas.
    ///
    /// Windie keeps this rule on the typed name so CLI parsing, persistence,
    /// and future clients can share one contract.
    pub fn is_valid(&self) -> bool {
        let name = self.as_str();

        !name.is_empty()
            && name.len() <= 64
            && name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    }
}

impl std::fmt::Display for ToolSchemaName {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl std::ops::Deref for ToolSchemaName {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
/// Tool definition that can be sent to the model.
///
/// This is only the schema. It does not execute the tool and does not grant any
/// permission. Execution must go through explicit runtime boundaries.
pub struct ToolSchema {
    pub name: ToolSchemaName,
    pub description: String,
    pub parameters: Value,
}

impl ToolSchema {
    /// Returns whether the human-facing description carries meaningful text.
    pub fn has_valid_description(&self) -> bool {
        !self.description.trim().is_empty()
    }
}
