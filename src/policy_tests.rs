//! Tests for tool execution policy decisions.

use super::*;

use crate::conversation::{ToolCall, ToolSchema, ToolSchemaName};
use crate::tool::{AttachedTool, ToolPermission};
use crate::tool_provider::ToolProviderRegistry;

#[test]
fn attached_run_shell_requires_approval() {
    let policy = ToolPolicy;
    let tool_call = ToolCall::function("call_1", "run_shell", r#"{"command":"ls"}"#);
    let registry = ToolProviderRegistry::new();
    let attached_tool = registry
        .find_tool(
            &crate::tool::ToolProviderId::new("windie"),
            &crate::tool::ProviderToolName::new("run_shell"),
        )
        .unwrap()
        .unwrap()
        .attached_tool();

    let decision = policy.decide(&tool_call, Some(&attached_tool), true);

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

    let decision = policy.decide(&tool_call, None, false);

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
    let attached_tool = AttachedTool::manual(ToolSchema {
        name: ToolSchemaName::new("unknown"),
        description: "Unknown tool".to_string(),
        parameters: serde_json::json!({"type":"object"}),
    });

    let decision = policy.decide(&tool_call, Some(&attached_tool), false);

    assert_eq!(
        decision,
        PolicyDecision::Deny {
            reason: "unknown tool: unknown".to_string()
        }
    );
}

#[test]
fn attached_non_shell_tool_requires_generic_approval() {
    let policy = ToolPolicy;
    let tool_call = ToolCall::function("call_1", "future_tool", "{}");
    let mut attached_tool = AttachedTool::manual(ToolSchema {
        name: ToolSchemaName::new("future_tool"),
        description: "Future tool".to_string(),
        parameters: serde_json::json!({"type":"object"}),
    });
    attached_tool.permissions = vec![ToolPermission::ExternalProcess];

    let decision = policy.decide(&tool_call, Some(&attached_tool), true);

    assert_eq!(
        decision,
        PolicyDecision::Ask {
            reason: "tool requires approval".to_string()
        }
    );
}
