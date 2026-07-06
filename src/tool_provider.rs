//! Tool provider registry and dispatch.
//!
//! This module is the execution boundary for tool providers. Runtime asks this
//! registry which tools are available and asks it to execute an approved tool
//! call through the provider reference stored on the conversation attachment.
//! Built-ins, MCP servers, and future plugins should enter runtime through this
//! same registry shape.

use anyhow::Result;
use serde_json::{Value, json};

use crate::conversation::{ToolCall, ToolSchemaName};
use crate::error;
use crate::mcp::{self, McpCommand, McpTool};
use crate::shell::ShellExecutor;
use crate::tool::{
    AttachedTool, ProviderToolName, ToolAnnotations, ToolDefinition, ToolExecutionResult,
    ToolPermission, ToolProviderId, ToolProviderKind, ToolProviderRef,
};

const WINDIE_PROVIDER_ID: &str = "windie";
const RUN_SHELL_TOOL_NAME: &str = "run_shell";
const CUA_DRIVER_PROVIDER_ID: &str = "cua-driver";
const CUA_DRIVER_SCHEMA_PREFIX: &str = "cua_driver";
const CUA_DRIVER_COMMAND: McpCommand = McpCommand {
    program: "cua-driver",
    args: &["mcp"],
};

#[derive(Debug, Clone)]
/// Registry of tool providers available to this Windie process.
///
/// The registry deliberately exposes provider-neutral operations. Runtime does
/// not branch on shell, MCP, or plugin details; it resolves the conversation's
/// attached tool to a provider reference and calls this registry.
pub struct ToolProviderRegistry {
    built_in: BuiltInToolProvider,
    cua_driver: McpToolProvider,
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
    pub fn list_available_tools(&self) -> Result<Vec<ToolDefinition>> {
        let mut tools = self.built_in.list_tools();
        tools.extend(self.cua_driver.list_tools()?);

        Ok(tools)
    }

    /// Lists available tools for one provider ID.
    pub fn list_provider_tools(&self, provider_id: &ToolProviderId) -> Result<Vec<ToolDefinition>> {
        if provider_id.as_str() == WINDIE_PROVIDER_ID {
            return Ok(self.built_in.list_tools());
        }
        if provider_id.as_str() == CUA_DRIVER_PROVIDER_ID {
            return self.cua_driver.list_tools();
        }

        Ok(Vec::new())
    }

    /// Finds one available provider tool by provider ID and provider-native
    /// tool name.
    pub fn find_tool(
        &self,
        provider_id: &ToolProviderId,
        tool_name: &ProviderToolName,
    ) -> Result<Option<ToolDefinition>> {
        Ok(self
            .list_provider_tools(provider_id)?
            .into_iter()
            .find(|tool| tool.provider.tool_name == *tool_name))
    }

    /// Returns whether this process has an executor for the attached provider
    /// tool.
    pub fn can_execute(&self, attached_tool: &AttachedTool) -> bool {
        matches!(
            attached_tool.provider.provider_id.as_str(),
            WINDIE_PROVIDER_ID | CUA_DRIVER_PROVIDER_ID
        )
    }

    /// Executes one approved model tool call through its attached provider.
    pub async fn call_tool(
        &self,
        attached_tool: &AttachedTool,
        tool_call: &ToolCall,
    ) -> Result<ToolExecutionResult> {
        match attached_tool.provider.kind {
            ToolProviderKind::BuiltIn => self.built_in.call_tool(attached_tool, tool_call).await,
            ToolProviderKind::Mcp => self.cua_driver.call_tool(attached_tool, tool_call).await,
            ToolProviderKind::Plugin => Err(error::invalid_request(format!(
                "unknown tool: {}",
                tool_call.name()
            ))),
        }
    }
}

impl Default for ToolProviderRegistry {
    fn default() -> Self {
        Self {
            built_in: BuiltInToolProvider,
            cua_driver: McpToolProvider::cua_driver(),
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

#[derive(Debug, Clone)]
/// Provider for an approved MCP stdio server.
pub struct McpToolProvider {
    provider_id: ToolProviderId,
    schema_prefix: &'static str,
    display_name: &'static str,
    command: McpCommand,
}

impl McpToolProvider {
    /// Builds the approved CUA driver MCP provider.
    pub fn cua_driver() -> Self {
        Self {
            provider_id: ToolProviderId::new(CUA_DRIVER_PROVIDER_ID),
            schema_prefix: CUA_DRIVER_SCHEMA_PREFIX,
            display_name: "CUA Driver",
            command: CUA_DRIVER_COMMAND,
        }
    }

    /// Lists tools from the MCP server and maps them into Windie definitions.
    pub fn list_tools(&self) -> Result<Vec<ToolDefinition>> {
        Ok(mcp::list_tools(self.command)?
            .into_iter()
            .map(|tool| self.definition_from_mcp_tool(tool))
            .collect())
    }

    /// Executes one approved MCP tool call.
    async fn call_tool(
        &self,
        attached_tool: &AttachedTool,
        tool_call: &ToolCall,
    ) -> Result<ToolExecutionResult> {
        if attached_tool.provider.provider_id != self.provider_id
            || tool_call.name() != attached_tool.schema_name.as_str()
        {
            return Err(error::invalid_request(format!(
                "unknown tool: {}",
                tool_call.name()
            )));
        }
        let arguments = match serde_json::from_str::<Value>(tool_call.arguments()) {
            Ok(arguments) => arguments,
            Err(error) => {
                return Ok(ToolExecutionResult::failure(
                    tool_call.id.clone(),
                    tool_call.name(),
                    format!("invalid tool arguments: {error}"),
                ));
            }
        };
        let result = mcp::call_tool(
            self.command,
            attached_tool.provider.tool_name.as_str(),
            arguments,
        )?;
        let success = !result
            .get("isError")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        Ok(ToolExecutionResult {
            tool_call_id: tool_call.id.clone(),
            tool_name: tool_call.name().to_string(),
            content: result.to_string(),
            success,
        })
    }

    /// Converts one MCP tool into Windie's provider-backed tool definition.
    fn definition_from_mcp_tool(&self, tool: McpTool) -> ToolDefinition {
        let read_only = tool
            .annotations
            .as_ref()
            .and_then(|annotations| annotations.read_only_hint)
            .unwrap_or(false);

        ToolDefinition {
            schema_name: ToolSchemaName::new(mcp_schema_name(self.schema_prefix, &tool.name)),
            display_name: format!("{} {}", self.display_name, tool.name),
            description: tool.description,
            parameters: tool.input_schema,
            provider: ToolProviderRef::new(
                self.provider_id.clone(),
                ProviderToolName::new(tool.name),
                ToolProviderKind::Mcp,
            ),
            permissions: vec![ToolPermission::ExternalProcess],
            annotations: ToolAnnotations {
                title: None,
                read_only: Some(read_only),
            },
        }
    }
}

/// Builds the model-facing schema name for one MCP provider tool.
fn mcp_schema_name(schema_prefix: &str, tool_name: &str) -> String {
    format!(
        "{schema_prefix}__{}",
        tool_name
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() || character == '_' || character == '-' {
                    character
                } else {
                    '_'
                }
            })
            .collect::<String>()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_schema_names_are_provider_prefixed() {
        assert_eq!(mcp_schema_name("cua_driver", "click"), "cua_driver__click");
        assert_eq!(
            mcp_schema_name("cua_driver", "type text"),
            "cua_driver__type_text"
        );
    }

    #[test]
    fn cua_mcp_tools_map_to_provider_backed_definitions() {
        let provider = McpToolProvider::cua_driver();
        let definition = provider.definition_from_mcp_tool(McpTool {
            name: "click".to_string(),
            description: "Click somewhere".to_string(),
            input_schema: json!({"type":"object"}),
            annotations: Some(mcp::McpToolAnnotations {
                read_only_hint: Some(false),
            }),
        });

        assert_eq!(definition.schema_name.as_str(), "cua_driver__click");
        assert_eq!(definition.provider.provider_id.as_str(), "cua-driver");
        assert_eq!(definition.provider.tool_name.as_str(), "click");
        assert_eq!(definition.provider.kind, ToolProviderKind::Mcp);
        assert_eq!(
            definition.permissions,
            vec![ToolPermission::ExternalProcess]
        );
        assert_eq!(definition.annotations.read_only, Some(false));
    }
}
