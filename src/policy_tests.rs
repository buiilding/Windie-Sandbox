//! Tests for tool execution policy decisions.

use super::*;

use crate::conversation::ToolCall;

#[test]
fn run_shell_requires_approval() {
    let policy = ToolPolicy;
    let tool_call = ToolCall::function("call_1", "run_shell", r#"{"command":"ls"}"#);

    let decision = policy.decide(&tool_call);

    assert_eq!(
        decision,
        PolicyDecision::Ask {
            reason: "shell command requires approval".to_string()
        }
    );
}

#[test]
fn unknown_tool_is_denied() {
    let policy = ToolPolicy;
    let tool_call = ToolCall::function("call_1", "unknown", "{}");

    let decision = policy.decide(&tool_call);

    assert_eq!(
        decision,
        PolicyDecision::Deny {
            reason: "unknown tool: unknown".to_string()
        }
    );
}
