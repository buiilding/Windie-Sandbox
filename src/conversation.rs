//! Core conversation data.
//!
//! Defines typed conversation IDs, message IDs, image asset IDs, compaction IDs,
//! message roles, message parts, and messages. This file only models runtime
//! data; it does not save, print, read input, or call the LLM.

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

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
/// Stable identifier for one persisted image asset.
pub struct ImageAssetId(String);

impl ImageAssetId {
    /// Wraps raw ID text so image identity stays type-distinct from messages.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Exposes the stored ID as plain text at persistence and display
    /// boundaries.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ImageAssetId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl std::ops::Deref for ImageAssetId {
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

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
/// Stable identifier for one model-requested tool call.
pub struct ToolCallId(String);

impl ToolCallId {
    /// Wraps raw tool-call ID text returned by the provider.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Exposes the provider tool-call ID as plain text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ToolCallId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl std::ops::Deref for ToolCallId {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// Function call requested by the model.
///
/// Windie stores and displays these calls, but this type does not imply tool
/// execution. Execution must happen later through explicit runtime permission
/// boundaries.
pub struct ToolCall {
    #[serde(default)]
    pub index: u16,
    pub id: ToolCallId,
    #[serde(rename = "type")]
    pub kind: ToolCallKind,
    pub function: ToolCallFunction,
}

impl ToolCall {
    /// Builds a function tool call from the provider ID, function name, and
    /// streamed argument text.
    pub fn function(
        id: impl Into<String>,
        name: impl Into<String>,
        arguments: impl Into<String>,
    ) -> Self {
        Self {
            index: 0,
            id: ToolCallId::new(id),
            kind: ToolCallKind::Function,
            function: ToolCallFunction {
                name: name.into(),
                arguments: arguments.into(),
            },
        }
    }

    /// Returns the requested function name for display and execution planning.
    pub fn name(&self) -> &str {
        &self.function.name
    }

    /// Returns the raw JSON argument string produced by the model.
    pub fn arguments(&self) -> &str {
        &self.function.arguments
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
/// Tool call kind accepted by the OpenAI-compatible chat format.
pub enum ToolCallKind {
    Function,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// Function payload inside a model-requested tool call.
pub struct ToolCallFunction {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// Metadata stored on assistant messages that requested tool calls.
pub struct MessageMetadata {
    pub tool_calls: Vec<ToolCall>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// One typed piece of a model-facing message.
pub enum MessagePart {
    Text(String),
    Image(ImagePart),
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Durable image bytes attached to a message.
pub struct ImagePart {
    pub asset_id: ImageAssetId,
    pub mime_type: String,
    pub bytes: Vec<u8>,
}

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
