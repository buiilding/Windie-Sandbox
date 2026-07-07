//! Tool execution policy boundary.
//!
//! This module decides whether a model-requested tool call may execute. The
//! first policy is intentionally small: detached tools are denied, attached
//! tools with no registered executor are denied, and executable local tools
//! either ask for explicit user approval or run under a conversation-level
//! auto-approval mode.

use crate::conversation::ToolCall;
use crate::tool::{AttachedTool, ToolApprovalMode, ToolPermission};

#[derive(Debug, Clone, PartialEq, Eq)]
/// Policy decision for one model-requested tool call.
pub enum PolicyDecision {
    Allow,
    Ask { reason: String },
    Deny { reason: String },
}

#[derive(Debug, Default, Clone, Copy)]
/// Minimal Windie-native policy for local tool execution.
pub struct ToolPolicy;

impl ToolPolicy {
    /// Decides whether Windie may execute a model-requested tool call.
    ///
    /// The attached tool is the conversation-level permission boundary: Windie
    /// may only consider executing tools explicitly attached to the current
    /// conversation. Provider executability is passed in separately so policy
    /// can reject raw/manual attachments or missing providers before approval.
    pub fn decide(
        &self,
        tool_call: &ToolCall,
        attached_tool: Option<&AttachedTool>,
        provider_can_execute: bool,
        approval_mode: ToolApprovalMode,
    ) -> PolicyDecision {
        let name = tool_call.name();

        let Some(attached_tool) = attached_tool else {
            return PolicyDecision::Deny {
                reason: format!("Tool is not attached: {name}"),
            };
        };

        if !provider_can_execute {
            return PolicyDecision::Deny {
                reason: format!("unknown tool: {name}"),
            };
        }

        if approval_mode == ToolApprovalMode::AutoApproveAttached {
            return PolicyDecision::Allow;
        }

        if attached_tool
            .permissions
            .contains(&ToolPermission::LocalShell)
        {
            PolicyDecision::Ask {
                reason: "shell tool requires approval".to_string(),
            }
        } else {
            PolicyDecision::Ask {
                reason: "tool requires approval".to_string(),
            }
        }
    }
}

#[cfg(test)]
#[path = "policy_tests.rs"]
mod tests;
