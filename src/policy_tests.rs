//! Tests for tool execution policy decisions.

use super::*;

use std::collections::HashSet;

use crate::conversation::{ToolCall, ToolSchemaName};

#[test]
fn attached_run_shell_requires_approval() {
    let policy = ToolPolicy;
    let tool_call = ToolCall::function("call_1", "run_shell", r#"{"command":"ls"}"#);
    let attached_tool_names = attached_tool_names(["run_shell"]);

    let decision = policy.decide(&tool_call, &attached_tool_names);

    assert_eq!(
        decision,
        PolicyDecision::Ask {
            reason: "shell tool requires approval".to_string()
        }
    );
}

#[test]
fn detached_tool_is_denied() {
    let policy = ToolPolicy;
    let tool_call = ToolCall::function("call_1", "run_shell", r#"{"command":"ls"}"#);
    let attached_tool_names = HashSet::new();

    let decision = policy.decide(&tool_call, &attached_tool_names);

    assert_eq!(
        decision,
        PolicyDecision::Deny {
            reason: "Tool is not attached: run_shell".to_string()
        }
    );
}

#[test]
fn attached_unknown_tool_executor_is_denied() {
    let policy = ToolPolicy;
    let tool_call = ToolCall::function("call_1", "unknown", "{}");
    let attached_tool_names = attached_tool_names(["unknown"]);

    let decision = policy.decide(&tool_call, &attached_tool_names);

    assert_eq!(
        decision,
        PolicyDecision::Deny {
            reason: "unknown tool: unknown".to_string()
        }
    );
}

fn attached_tool_names(names: impl IntoIterator<Item = &'static str>) -> HashSet<ToolSchemaName> {
    names.into_iter().map(ToolSchemaName::new).collect()
}
