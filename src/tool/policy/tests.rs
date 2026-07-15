//! Tests for tool execution policy decisions.

use super::*;

use crate::conversation::ToolCall;
use crate::tool::{
    AttachedTool, ProviderToolName, ToolAnnotations, ToolApprovalMode, ToolPermission,
    ToolProviderId, ToolProviderKind, ToolProviderRef, ToolSchema, ToolSchemaName,
};

#[test]
fn attached_executable_tool_requires_approval() {
    let policy = ToolPolicy;
    let tool_call = ToolCall::function("call_1", "desktop_commander__read_file", "{}");
    let attached_tool = test_attached_mcp_tool("desktop_commander__read_file", "read_file");

    let decision = policy.decide(
        &tool_call,
        Some(&attached_tool),
        true,
        ToolApprovalMode::Manual,
    );

    assert_eq!(
        decision,
        PolicyDecision::Ask {
            reason: "tool requires approval".to_string()
        }
    );
}

#[test]
fn detached_tool_is_denied() {
    let policy = ToolPolicy;
    let tool_call = ToolCall::function("call_1", "run_shell", r#"{"command":"ls"}"#);

    let decision = policy.decide(&tool_call, None, false, ToolApprovalMode::Manual);

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

    let decision = policy.decide(
        &tool_call,
        Some(&attached_tool),
        false,
        ToolApprovalMode::Manual,
    );

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

    let decision = policy.decide(
        &tool_call,
        Some(&attached_tool),
        true,
        ToolApprovalMode::Manual,
    );

    assert_eq!(
        decision,
        PolicyDecision::Ask {
            reason: "tool requires approval".to_string()
        }
    );
}

#[test]
fn auto_approve_attached_allows_executable_tool() {
    let policy = ToolPolicy;
    let tool_call = ToolCall::function("call_1", "desktop_commander__read_file", "{}");
    let attached_tool = test_attached_mcp_tool("desktop_commander__read_file", "read_file");

    let decision = policy.decide(
        &tool_call,
        Some(&attached_tool),
        true,
        ToolApprovalMode::AutoApproveAttached,
    );

    assert_eq!(decision, PolicyDecision::Allow);
}

#[test]
fn auto_approve_attached_still_denies_detached_tool() {
    let policy = ToolPolicy;
    let tool_call = ToolCall::function("call_1", "run_shell", r#"{"command":"ls"}"#);

    let decision = policy.decide(
        &tool_call,
        None,
        false,
        ToolApprovalMode::AutoApproveAttached,
    );

    assert_eq!(
        decision,
        PolicyDecision::Deny {
            reason: "Tool is not attached: run_shell".to_string()
        }
    );
}

fn test_attached_mcp_tool(schema_name: &str, provider_tool_name: &str) -> AttachedTool {
    AttachedTool {
        schema_name: ToolSchemaName::new(schema_name),
        description: "Test MCP tool".to_string(),
        parameters: serde_json::json!({"type":"object"}),
        provider: ToolProviderRef::new(
            ToolProviderId::new("desktop-commander"),
            ProviderToolName::new(provider_tool_name),
            ToolProviderKind::Mcp,
        ),
        permissions: vec![ToolPermission::ExternalProcess],
        annotations: ToolAnnotations::default(),
    }
}

#[test]
fn auto_approve_attached_still_denies_unknown_executor() {
    let policy = ToolPolicy;
    let tool_call = ToolCall::function("call_1", "unknown", "{}");
    let attached_tool = AttachedTool::manual(ToolSchema {
        name: ToolSchemaName::new("unknown"),
        description: "Unknown tool".to_string(),
        parameters: serde_json::json!({"type":"object"}),
    });

    let decision = policy.decide(
        &tool_call,
        Some(&attached_tool),
        false,
        ToolApprovalMode::AutoApproveAttached,
    );

    assert_eq!(
        decision,
        PolicyDecision::Deny {
            reason: "unknown tool: unknown".to_string()
        }
    );
}
