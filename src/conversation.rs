//! Core conversation data.
//!
//! Defines typed conversation IDs, message IDs, image asset IDs, compaction IDs,
//! message roles, message parts, and messages. This file only models runtime
//! data; it does not save, print, read input, or call the LLM.

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize, Serializer, ser::SerializeStruct};

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

#[derive(Debug, Clone, Deserialize)]
/// One conversation message in Windie's runtime model.
///
/// Only `role` and `content` serialize to the LLM request by default. Assistant
/// tool calls stored in typed metadata also serialize because OpenAI-compatible
/// context needs them to continue tool-call conversations.
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

impl Serialize for Message {
    /// Serializes only model-facing fields, plus assistant tool calls when
    /// stored in metadata.
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let tool_calls = self
            .metadata
            .as_ref()
            .filter(|_| self.role == Role::Assistant)
            .map(|metadata| metadata.tool_calls.as_slice())
            .unwrap_or(&[]);

        let mut state =
            serializer.serialize_struct("Message", if tool_calls.is_empty() { 2 } else { 3 })?;
        state.serialize_field("role", &self.role)?;
        if self.parts.is_empty() {
            state.serialize_field("content", &self.content)?;
        } else {
            state.serialize_field("content", &model_content_parts(&self.parts))?;
        }
        if !tool_calls.is_empty() {
            state.serialize_field("tool_calls", tool_calls)?;
        }
        state.end()
    }
}

#[derive(Serialize)]
#[serde(untagged)]
/// One OpenAI-compatible content part emitted in chat requests.
enum ModelContentPart<'a> {
    Text(ModelTextPart<'a>),
    Image(ModelImagePart),
}

#[derive(Serialize)]
/// OpenAI-compatible text content part.
struct ModelTextPart<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    text: &'a str,
}

#[derive(Serialize)]
/// OpenAI-compatible image content part.
struct ModelImagePart {
    #[serde(rename = "type")]
    kind: &'static str,
    image_url: ModelImageUrl,
}

#[derive(Serialize)]
/// OpenAI-compatible image URL payload.
struct ModelImageUrl {
    url: String,
}

/// Converts stored message parts into OpenAI-compatible content parts.
fn model_content_parts(parts: &[MessagePart]) -> Vec<ModelContentPart<'_>> {
    parts
        .iter()
        .map(|part| match part {
            MessagePart::Text(text) => ModelContentPart::Text(ModelTextPart { kind: "text", text }),
            MessagePart::Image(image) => ModelContentPart::Image(ModelImagePart {
                kind: "image_url",
                image_url: ModelImageUrl {
                    url: format!(
                        "data:{};base64,{}",
                        image.mime_type,
                        STANDARD.encode(&image.bytes)
                    ),
                },
            }),
        })
        .collect()
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
            parts: Vec::new(),
            metadata: None,
        };

        let value = serde_json::to_value(message).unwrap();

        assert_eq!(
            value,
            serde_json::json!({"role": "user", "content": "hello"})
        );
    }

    #[test]
    fn serializes_assistant_tool_calls_from_metadata() {
        let message = Message {
            id: Some(MessageId::new("message-id")),
            parent_message_id: Some(MessageId::new("parent-id")),
            role: Role::Assistant,
            content: String::new(),
            parts: Vec::new(),
            metadata: Some(MessageMetadata {
                tool_calls: vec![ToolCall {
                    index: 0,
                    id: ToolCallId::new("call-id"),
                    kind: ToolCallKind::Function,
                    function: ToolCallFunction {
                        name: "run_shell".to_string(),
                        arguments: r#"{"command":"ls"}"#.to_string(),
                    },
                }],
            }),
        };

        let value = serde_json::to_value(message).unwrap();

        assert_eq!(
            value,
            serde_json::json!({
                "role": "assistant",
                "content": "",
                "tool_calls": [{
                    "index": 0,
                    "id": "call-id",
                    "type": "function",
                    "function": {
                        "name": "run_shell",
                        "arguments": "{\"command\":\"ls\"}"
                    }
                }]
            })
        );
    }

    #[test]
    fn serializes_user_image_parts_for_model_context() {
        let message = Message {
            id: Some(MessageId::new("message-id")),
            parent_message_id: None,
            role: Role::User,
            content: "what is this?".to_string(),
            parts: vec![
                MessagePart::Text("what is this?".to_string()),
                MessagePart::Image(ImagePart {
                    asset_id: ImageAssetId::new("image-id"),
                    mime_type: "image/png".to_string(),
                    bytes: vec![1, 2, 3],
                }),
            ],
            metadata: None,
        };

        let value = serde_json::to_value(message).unwrap();

        assert_eq!(
            value,
            serde_json::json!({
                "role": "user",
                "content": [
                    {"type": "text", "text": "what is this?"},
                    {"type": "image_url", "image_url": {"url": "data:image/png;base64,AQID"}}
                ]
            })
        );
    }
}
