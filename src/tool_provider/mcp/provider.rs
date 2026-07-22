//! Generic MCP tool provider adapter.
//!
//! This adapter knows how to list tools from one approved MCP stdio server and
//! expose them as Windie tool definitions. Executing an already-approved MCP
//! call lives in `execution.rs`.

use anyhow::Result;

use super::desktop_commander;
use crate::mcp::{self, McpCommand, McpTool};
use crate::tool::{
    ProviderToolName, ToolAnnotations, ToolDefinition, ToolPermission, ToolProviderId,
    ToolProviderKind, ToolProviderRef, ToolSchemaName,
};
use crate::tool_provider::manifest::ProviderManifest;

#[derive(Debug, Clone)]
/// Static definition for one code-approved MCP provider.
///
/// This is intentionally data, not runtime state. Adding a future approved MCP
/// provider should add one server definition while keeping `McpToolProvider`
/// generic.
pub(in crate::tool_provider) struct McpProviderDefinition {
    pub(in crate::tool_provider) manifest: ProviderManifest,
    pub(in crate::tool_provider) provider_id: &'static str,
    pub(in crate::tool_provider) schema_prefix: &'static str,
    pub(in crate::tool_provider) display_name: &'static str,
    pub(in crate::tool_provider) command: McpCommand,
    pub(in crate::tool_provider) shutdown_command: Option<McpCommand>,
    pub(in crate::tool_provider) setup: Option<McpProviderSetup>,
}

#[derive(Debug, Clone, Copy)]
/// Provider-specific setup Windie runs before starting an MCP process.
pub(in crate::tool_provider) enum McpProviderSetup {
    DesktopCommanderConfig,
}

#[derive(Debug, Clone)]
/// Provider for an approved MCP stdio server.
pub(in crate::tool_provider) struct McpToolProvider {
    manifest: ProviderManifest,
    pub(in crate::tool_provider) provider_id: ToolProviderId,
    pub(in crate::tool_provider) schema_prefix: &'static str,
    pub(in crate::tool_provider) display_name: &'static str,
    pub(in crate::tool_provider) command: McpCommand,
    pub(in crate::tool_provider) shutdown_command: Option<McpCommand>,
    setup: Option<McpProviderSetup>,
}

impl McpToolProvider {
    /// Builds a runtime provider from a code-approved provider definition.
    pub(in crate::tool_provider) fn new(definition: McpProviderDefinition) -> Self {
        Self {
            manifest: definition.manifest,
            provider_id: ToolProviderId::new(definition.provider_id),
            schema_prefix: definition.schema_prefix,
            display_name: definition.display_name,
            command: definition.command,
            shutdown_command: definition.shutdown_command,
            setup: definition.setup,
        }
    }

    /// Returns the stable provider ID used by attachments and dispatch.
    pub(in crate::tool_provider) fn id(&self) -> &ToolProviderId {
        &self.provider_id
    }

    /// Returns the metadata contract for this provider.
    pub(in crate::tool_provider) fn manifest(&self) -> &ProviderManifest {
        &self.manifest
    }

    /// Lists tools from the MCP server and maps them into Windie definitions.
    pub(in crate::tool_provider) fn list_tools(&self) -> Result<Vec<ToolDefinition>> {
        self.prepare()?;
        Ok(
            mcp::list_tools_with_shutdown(self.command, self.shutdown_command)?
                .into_iter()
                .map(|tool| self.definition_from_mcp_tool(tool))
                .collect(),
        )
    }

    /// Converts one MCP tool into Windie's provider-backed tool definition.
    pub(in crate::tool_provider) fn definition_from_mcp_tool(
        &self,
        tool: McpTool,
    ) -> ToolDefinition {
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

    /// Runs provider-specific setup before Windie starts the MCP process.
    pub(in crate::tool_provider) fn prepare(&self) -> Result<()> {
        match self.setup {
            Some(McpProviderSetup::DesktopCommanderConfig) => desktop_commander::prepare(),
            None => Ok(()),
        }
    }
}

/// Builds the model-facing schema name for one MCP provider tool.
pub(in crate::tool_provider) fn mcp_schema_name(schema_prefix: &str, tool_name: &str) -> String {
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
