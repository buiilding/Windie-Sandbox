//! Tool execution result type.
//!
//! A tool execution result is runtime-ready data that can be persisted as a
//! `role: tool` message on the conversation path.

use serde::Serialize;

use crate::conversation::{ToolCallId, UnsavedMessagePart};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
/// Result of one tool execution ready to be stored as a `role: tool` message.
pub struct ToolExecutionResult {
    pub tool_call_id: ToolCallId,
    pub tool_name: String,
    /// Compact visible preview stored on the message row.
    pub content: String,
    /// Ordered model-facing content parts. Text-only tools can leave this empty
    /// and use `content`; rich tools such as MCP screenshot providers store
    /// text/image parts here so Responses can replay them without base64 text
    /// bloat.
    #[serde(skip)]
    pub parts: Vec<UnsavedMessagePart>,
    pub success: bool,
}

impl ToolExecutionResult {
    /// Creates a successful rich tool result with ordered model-facing parts.
    pub fn success_with_parts(
        tool_call_id: ToolCallId,
        tool_name: impl Into<String>,
        content: String,
        parts: Vec<UnsavedMessagePart>,
    ) -> Self {
        Self {
            tool_call_id,
            tool_name: tool_name.into(),
            content,
            parts,
            success: true,
        }
    }

    /// Creates a failed tool result with a short model-facing error.
    pub fn failure(
        tool_call_id: ToolCallId,
        tool_name: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            tool_call_id,
            tool_name: tool_name.into(),
            content: serde_json::json!({ "error": reason.into() }).to_string(),
            parts: Vec::new(),
            success: false,
        }
    }
}
