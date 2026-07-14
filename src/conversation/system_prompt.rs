//! Path-scoped system prompt data.
//!
//! System prompts are stored as ordered path records. Replaying the records that
//! apply to a selected conversation path gives the effective prompt text sent to
//! the model.

use crate::conversation::{ConversationId, MessageId};

#[derive(Debug, Clone, PartialEq, Eq)]
/// Stable identifier for one persisted system prompt record.
pub struct SystemPromptId(String);

impl SystemPromptId {
    /// Wraps raw ID text so prompt record identity stays type-distinct.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Exposes the stored ID as plain text at persistence boundaries.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SystemPromptId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl std::ops::Deref for SystemPromptId {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// One system prompt record anchored to a conversation path.
///
/// `message_id = None` means the record belongs to the root state before any
/// messages exist. `Some(message_id)` means the record applies only to paths
/// containing that message.
pub enum SystemPrompt {
    Set {
        id: SystemPromptId,
        conversation_id: ConversationId,
        message_id: Option<MessageId>,
        text: String,
        created_at: i64,
    },
    Removed {
        id: SystemPromptId,
        conversation_id: ConversationId,
        message_id: Option<MessageId>,
        created_at: i64,
    },
}

impl SystemPrompt {
    /// Builds a prompt-setting record.
    pub fn set(
        id: SystemPromptId,
        conversation_id: ConversationId,
        message_id: Option<MessageId>,
        text: String,
        created_at: i64,
    ) -> Self {
        Self::Set {
            id,
            conversation_id,
            message_id,
            text,
            created_at,
        }
    }

    /// Builds a prompt-removal record.
    pub fn removed(
        id: SystemPromptId,
        conversation_id: ConversationId,
        message_id: Option<MessageId>,
        created_at: i64,
    ) -> Self {
        Self::Removed {
            id,
            conversation_id,
            message_id,
            created_at,
        }
    }

    /// Returns the persisted prompt record ID.
    pub fn id(&self) -> &SystemPromptId {
        match self {
            Self::Set { id, .. } | Self::Removed { id, .. } => id,
        }
    }

    /// Returns the optional message anchor for this prompt record.
    pub fn message_id(&self) -> Option<&MessageId> {
        match self {
            Self::Set { message_id, .. } | Self::Removed { message_id, .. } => message_id.as_ref(),
        }
    }

    /// Returns the creation timestamp used for deterministic replay.
    pub fn created_at(&self) -> i64 {
        match self {
            Self::Set { created_at, .. } | Self::Removed { created_at, .. } => *created_at,
        }
    }

    /// Returns the text for a prompt-setting record.
    pub fn text(&self) -> Option<&str> {
        match self {
            Self::Set { text, .. } => Some(text),
            Self::Removed { .. } => None,
        }
    }

}
