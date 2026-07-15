//! Tool approval contracts.
//!
//! These types describe when a tool call needs user approval and how pending
//! approval requests are surfaced. Session orchestration decides how approvals
//! are listed, approved, denied, and resumed.

use serde::{Deserialize, Serialize};

use crate::conversation::{MessageId, ToolCall};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Conversation-level default for tool execution approval.
///
/// The mode only applies after the requested model-facing tool is exposed on
/// the conversation path and backed by a registered provider. Unexposed tools
/// and missing executors are still denied by policy.
pub enum ToolApprovalMode {
    Manual,
    AutoApproveAttached,
}

impl ToolApprovalMode {
    /// Converts persisted approval mode text into the typed enum.
    pub fn from_storage(value: &str) -> Option<Self> {
        match value {
            "manual" => Some(Self::Manual),
            "auto_approve_attached" => Some(Self::AutoApproveAttached),
            _ => None,
        }
    }

    /// Returns the stable storage/API representation.
    pub fn as_storage(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::AutoApproveAttached => "auto_approve_attached",
        }
    }
}

impl std::fmt::Display for ToolApprovalMode {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_storage())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
/// One pending model-requested tool call that requires user approval.
///
/// The assistant message ID identifies the assistant turn that requested the
/// call. Runtime decides the exact parent for the later `role: tool` result so
/// multi-tool results can stay on one linear path.
pub struct ToolApprovalRequest {
    pub assistant_message_id: MessageId,
    pub tool_call: ToolCall,
    pub reason: String,
}
