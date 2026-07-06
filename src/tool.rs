//! Tool execution data boundary.
//!
//! This module owns Windie's typed tool approval request and execution result
//! shapes. Tool schemas and model-requested tool calls live in
//! `conversation.rs`; the built-in schema catalog lives in `tool_catalog.rs`.

use serde::Serialize;

use crate::conversation::{MessageId, ToolCall, ToolCallId};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
/// One pending model-requested tool call that requires user approval.
///
/// The assistant message ID identifies the assistant turn that requested the
/// call. Runtime decides the exact parent for the later `role: tool` result so
/// multi-tool results can stay on one linear active path.
pub struct ToolApprovalRequest {
    pub assistant_message_id: MessageId,
    pub tool_call: ToolCall,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
/// Result of one tool execution ready to be stored as a `role: tool` message.
pub struct ToolExecutionResult {
    pub tool_call_id: ToolCallId,
    pub tool_name: String,
    pub content: String,
    pub success: bool,
}

impl ToolExecutionResult {
    /// Creates a successful tool result with model-facing content.
    pub fn success(
        tool_call_id: ToolCallId,
        tool_name: impl Into<String>,
        content: String,
    ) -> Self {
        Self {
            tool_call_id,
            tool_name: tool_name.into(),
            content,
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
            success: false,
        }
    }
}
