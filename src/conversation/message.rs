//! Conversation message data.
//!
//! Messages are Windie's internal tree nodes. Provider-specific serialization,
//! path selection, and persistence are handled by other modules.

use serde::{Deserialize, Serialize};

use crate::conversation::{MessageId, MessageMetadata, MessagePart};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
/// Windie's typed role for one conversation message.
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

impl Role {
    /// Returns the exact lowercase role string stored in SQLite and used at
    /// serialization boundaries.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::Tool => "tool",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
/// One conversation message in Windie's runtime model.
///
/// This type stores Windie's internal message shape. Provider-specific request
/// serialization belongs to the LLM boundary.
pub struct Message {
    #[serde(skip)]
    pub id: Option<MessageId>,
    #[serde(skip)]
    #[allow(dead_code)]
    pub parent_message_id: Option<MessageId>,
    pub role: Role,
    pub content: String,
    #[serde(skip)]
    pub parts: Vec<MessagePart>,
    #[serde(skip)]
    #[allow(dead_code)]
    pub metadata: Option<MessageMetadata>,
}
