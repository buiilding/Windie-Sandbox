//! Core conversation data.
//!
//! Defines typed conversation IDs, message IDs, image asset IDs, compaction IDs,
//! tool schema names, message roles, message parts, message metadata, and
//! messages. This file only models runtime data; it does not save, print, read
//! input, or call the LLM.

use serde::{Deserialize, Serialize};
use serde_json::Value;

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

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
/// Stable name for one conversation-level tool schema.
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
/// Conversation-level tool definition that can be sent to the model.
///
/// This is only the schema. It does not execute the tool and does not grant any
/// permission. Execution must go through future explicit runtime boundaries.
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
/// Tool call kind accepted by the OpenAI-compatible Responses format.
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
#[serde(rename_all = "snake_case")]
/// Bifrost-normalized reasoning detail kind for assistant thinking output.
pub enum ReasoningDetailKind {
    #[serde(rename = "reasoning.summary")]
    Summary,
    #[serde(rename = "reasoning.encrypted")]
    Encrypted,
    #[serde(rename = "reasoning.text")]
    Text,
    #[serde(rename = "reasoning.content_blocks")]
    ContentBlocks,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// One structured assistant reasoning block returned by the provider adapter.
pub struct ReasoningDetail {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub index: u16,
    #[serde(rename = "type")]
    pub kind: ReasoningDetailKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// Assistant audio output metadata returned by audio-capable models.
pub struct AssistantAudio {
    pub id: String,
    pub data: String,
    pub expires_at: i64,
    pub transcript: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
/// Citation or annotation attached to an assistant message.
pub struct AssistantAnnotation {
    #[serde(rename = "type")]
    pub kind: String,
    pub url_citation: AssistantCitation,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
/// URL citation payload inside an assistant annotation.
pub struct AssistantCitation {
    pub start_index: i64,
    pub end_index: i64,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sources: Option<Value>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
/// Provider-reported token usage for one completed assistant model call.
///
/// The three common token totals are first-class fields because they are stable
/// across OpenAI-compatible Responses usage payloads. `raw` preserves the full
/// Bifrost/provider usage object so Windie does not lose newer or
/// provider-specific accounting details before it has typed contracts for them.
pub struct TokenUsage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<u64>,
    pub raw: Value,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
/// Metadata stored on messages outside normal visible text.
///
/// Assistant fields stay in separate output lanes so future UIs can render
/// text, tool calls, reasoning, refusals, audio, and citations separately.
/// `tool_call_id` is used by `role: tool` messages to link a tool result back
/// to the assistant tool call that requested it.
pub struct MessageMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<ToolCallId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refusal: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reasoning_details: Vec<ReasoningDetail>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<AssistantAudio>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub annotations: Vec<AssistantAnnotation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,
}

impl MessageMetadata {
    /// Returns whether the message has any metadata lane populated.
    pub fn is_empty(&self) -> bool {
        self.tool_call_id.is_none()
            && self.tool_calls.is_empty()
            && self.refusal.is_none()
            && self.reasoning.is_none()
            && self.reasoning_details.is_empty()
            && self.audio.is_none()
            && self.annotations.is_empty()
            && self.usage.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// One typed piece of a model-facing message.
pub enum MessagePart {
    Text(String),
    Image(ImagePart),
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// One typed message part before it has been copied into durable storage.
///
/// Unsaved parts carry raw bytes only. `store.rs` is responsible for assigning
/// durable asset IDs when it writes the message.
pub enum UnsavedMessagePart {
    Text(String),
    Image(UnsavedImagePart),
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Durable image bytes attached to a message.
pub struct ImagePart {
    pub asset_id: ImageAssetId,
    pub mime_type: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Image bytes that have not yet been copied into durable image asset storage.
pub struct UnsavedImagePart {
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
    pub parent_message_id: Option<MessageId>,
    pub role: Role,
    pub content: String,
    #[serde(skip)]
    pub parts: Vec<MessagePart>,
    #[serde(skip)]
    pub metadata: Option<MessageMetadata>,
}
