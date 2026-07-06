//! Tool execution policy boundary.
//!
//! This module decides whether a model-requested tool call may execute. The
//! first policy is intentionally small: detached tools are denied, unknown
//! executors are denied, and shell execution asks for explicit user approval.

use std::collections::HashSet;

use crate::conversation::{ToolCall, ToolSchemaName};

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
    ///
    /// The attached schema set is the conversation-level permission boundary:
    /// Windie may only consider executing tools explicitly attached to the
    /// current conversation. A schema can be attached without Windie having a
    /// local executor yet, which is why unknown executors are denied separately.
    pub fn decide(
        &self,
        tool_call: &ToolCall,
        attached_tool_names: &HashSet<ToolSchemaName>,
    ) -> PolicyDecision {
        let name = tool_call.name();

        if !attached_tool_names.contains(&ToolSchemaName::new(name)) {
            return PolicyDecision::Deny {
                reason: format!("Tool is not attached: {name}"),
            };
        }

        match name {
            "run_shell" => PolicyDecision::Ask {
                reason: "shell tool requires approval".to_string(),
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
