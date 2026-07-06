//! Tool provider registry and dispatch.
//!
//! This module is the execution boundary for tool providers. Runtime asks this
//! registry which tools are available and asks it to execute an approved tool
//! call through the provider reference stored on the conversation attachment.
//! Built-ins, MCP servers, and future plugins should enter runtime through this
//! same registry shape.

use anyhow::Result;
use serde_json::json;

use crate::conversation::{ToolCall, ToolSchemaName};
use crate::error;
use crate::shell::ShellExecutor;
use crate::tool::{
    AttachedTool, ProviderToolName, ToolAnnotations, ToolDefinition, ToolExecutionResult,
    ToolPermission, ToolProviderId, ToolProviderKind, ToolProviderRef,
};

const WINDIE_PROVIDER_ID: &str = "windie";
const RUN_SHELL_TOOL_NAME: &str = "run_shell";

#[derive(Debug, Default, Clone)]
/// Registry of tool providers available to this Windie process.
///
/// The registry deliberately exposes provider-neutral operations. Runtime does
/// not branch on shell, MCP, or plugin details; it resolves the conversation's
/// attached tool to a provider reference and calls this registry.
pub struct ToolProviderRegistry {
    built_in: BuiltInToolProvider,
}

impl ToolProviderRegistry {
    /// Builds the default registry for the local Windie process.
    pub fn new() -> Self {
        Self::default()
    }

    /// Lists every provider tool that clients may attach to conversations.
    ///
    /// Availability does not grant model access. Clients still need to attach a
    /// returned definition before the model sees the function schema.
    pub fn list_available_tools(&self) -> Vec<ToolDefinition> {
        self.built_in.list_tools()
    }

    /// Lists available tools for one provider ID.
    pub fn list_provider_tools(&self, provider_id: &ToolProviderId) -> Vec<ToolDefinition> {
        self.list_available_tools()
            .into_iter()
            .filter(|tool| tool.provider.provider_id == *provider_id)
            .collect()
    }

    /// Finds one available provider tool by provider ID and provider-native
    /// tool name.
    pub fn find_tool(
        &self,
        provider_id: &ToolProviderId,
        tool_name: &ProviderToolName,
    ) -> Option<ToolDefinition> {
        self.list_available_tools().into_iter().find(|tool| {
            tool.provider.provider_id == *provider_id && tool.provider.tool_name == *tool_name
        })
    }

    /// Returns whether this process has an executor for the attached provider
    /// tool.
    pub fn can_execute(&self, attached_tool: &AttachedTool) -> bool {
        self.find_tool(
            &attached_tool.provider.provider_id,
            &attached_tool.provider.tool_name,
        )
        .is_some()
    }

    /// Executes one approved model tool call through its attached provider.
    pub async fn call_tool(
        &self,
        attached_tool: &AttachedTool,
        tool_call: &ToolCall,
    ) -> Result<ToolExecutionResult> {
        match attached_tool.provider.kind {
            ToolProviderKind::BuiltIn => self.built_in.call_tool(attached_tool, tool_call).await,
            ToolProviderKind::Mcp | ToolProviderKind::Plugin => Err(error::invalid_request(
                format!("unknown tool: {}", tool_call.name()),
            )),
        }
    }
}

#[derive(Debug, Default, Clone, Copy)]
/// Provider for Windie-native tools compiled into the local runtime.
pub struct BuiltInToolProvider;

impl BuiltInToolProvider {
    /// Lists Windie's built-in provider tools.
    pub fn list_tools(&self) -> Vec<ToolDefinition> {
        vec![run_shell_tool_definition()]
    }

    /// Executes one built-in provider tool.
    async fn call_tool(
        &self,
        attached_tool: &AttachedTool,
        tool_call: &ToolCall,
    ) -> Result<ToolExecutionResult> {
        if attached_tool.provider.provider_id.as_str() != WINDIE_PROVIDER_ID
            || attached_tool.provider.tool_name.as_str() != RUN_SHELL_TOOL_NAME
            || tool_call.name() != attached_tool.schema_name.as_str()
        {
            return Err(error::invalid_request(format!(
                "unknown tool: {}",
                tool_call.name()
            )));
        }

        Ok(ShellExecutor::default().execute_tool_call(tool_call).await)
    }
}

/// Builds the provider-backed definition for Windie's bounded shell executor.
fn run_shell_tool_definition() -> ToolDefinition {
    ToolDefinition {
        schema_name: ToolSchemaName::new(RUN_SHELL_TOOL_NAME),
        display_name: "Run shell".to_string(),
        description: "Run a bounded local shell command after explicit user approval.".to_string(),
        parameters: json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "command": {
                    "type": "string",
                    "description": "Shell command to run through the user's default shell."
                },
                "cwd": {
                    "type": "string",
                    "description": "Optional working directory for the command."
                },
                "timeout_ms": {
                    "type": "integer",
                    "description": "Optional timeout in milliseconds, capped by Windie."
                }
            },
            "required": ["command"]
        }),
        provider: ToolProviderRef::new(
            ToolProviderId::new(WINDIE_PROVIDER_ID),
            ProviderToolName::new(RUN_SHELL_TOOL_NAME),
            ToolProviderKind::BuiltIn,
        ),
        permissions: vec![ToolPermission::LocalShell],
        annotations: ToolAnnotations {
            title: Some("Run shell".to_string()),
            read_only: Some(false),
        },
    }
}
