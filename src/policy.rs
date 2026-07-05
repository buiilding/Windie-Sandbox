//! Tool execution policy boundary.
//!
//! This module decides whether a model-requested tool call may execute. The
//! first policy is intentionally small: unknown tools are denied and shell
//! execution always asks for explicit user approval.

use crate::conversation::ToolCall;

#[derive(Debug, Clone, PartialEq, Eq)]
/// Policy decision for one model-requested tool call.
pub enum PolicyDecision {
    Ask { reason: String },
    Deny { reason: String },
}

#[derive(Debug, Default, Clone, Copy)]
/// Minimal Windie-native policy for local tool execution.
pub struct ToolPolicy;

impl ToolPolicy {
    /// Decides whether Windie may execute a model-requested tool call.
    pub fn decide(&self, tool_call: &ToolCall) -> PolicyDecision {
        match tool_call.name() {
            "run_shell" => PolicyDecision::Ask {
                reason: "shell command requires approval".to_string(),
            },
            name => PolicyDecision::Deny {
                reason: format!("unknown tool: {name}"),
            },
        }
    }
}

#[cfg(test)]
#[path = "policy_tests.rs"]
mod tests;
