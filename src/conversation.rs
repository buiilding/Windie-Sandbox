//! Core conversation data.
//!
//! Defines typed conversation IDs, message IDs, compaction IDs, message roles,
//! and messages. This file only models runtime data; it does not save, print,
//! read input, or call the LLM.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
/// Stable identifier for one persisted conversation.
pub struct ConversationId(String);

impl ConversationId {
    /// Wraps raw ID text so callers cannot accidentally pass a message ID where
    /// a conversation ID is expected.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Exposes the stored ID as plain text for SQLite, HTTP output, and CLI
    /// printing boundaries.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ConversationId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl std::ops::Deref for ConversationId {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
/// Stable identifier for one persisted message.
pub struct MessageId(String);

impl MessageId {
    /// Wraps raw ID text so message identity stays type-distinct from other IDs.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Exposes the stored ID as plain text at persistence and display
    /// boundaries.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for MessageId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl std::ops::Deref for MessageId {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
/// Message role accepted by the OpenAI-compatible chat request format.
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

impl Role {
    /// Returns the exact lowercase role string expected by Bifrost/OpenAI and
    /// stored in SQLite.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::Tool => "tool",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// One conversation message in Windie's runtime model.
///
/// Only `role` and `content` serialize to the LLM request. Local identifiers,
/// parent links, and metadata are persistence/runtime fields and are skipped.
pub struct Message {
    #[serde(skip)]
    pub id: Option<MessageId>,
    #[serde(skip)]
    #[allow(dead_code)]
    pub parent_message_id: Option<MessageId>,
    pub role: Role,
    pub content: String,
    #[serde(skip)]
    #[allow(dead_code)]
    pub metadata: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_only_model_fields() {
        let message = Message {
            id: Some(MessageId::new("message-id")),
            parent_message_id: Some(MessageId::new("parent-id")),
            role: Role::User,
            content: "hello".to_string(),
            metadata: Some(r#"{"tool_calls":[]}"#.to_string()),
        };

        let value = serde_json::to_value(message).unwrap();

        assert_eq!(
            value,
            serde_json::json!({"role": "user", "content": "hello"})
        );
    }
}
