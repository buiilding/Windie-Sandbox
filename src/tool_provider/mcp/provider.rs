//! Generic MCP tool provider adapter.
//!
//! This adapter knows how to list tools from one approved MCP stdio server and
//! expose them as Windie tool definitions. Executing an already-approved MCP
//! call lives in `execution.rs`.

use anyhow::Result;
use serde_json::json;

use super::{basic_memory, desktop_commander};
use crate::mcp::{self, McpCommand, McpTool, McpTransport};
use crate::tool::{
    ProviderToolName, ToolAnnotations, ToolDefinition, ToolPermission, ToolProviderId,
    ToolProviderKind, ToolProviderRef, ToolSchemaName,
};
use crate::tool_provider::ProviderCleanup;
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
    pub(in crate::tool_provider) transport: McpTransport,
    pub(in crate::tool_provider) package_command: Option<McpCommand>,
    pub(in crate::tool_provider) readiness_probe: Option<McpProviderReadinessProbe>,
    pub(in crate::tool_provider) setup: Option<McpProviderSetup>,
    pub(in crate::tool_provider) cleanup: ProviderCleanup,
}

#[derive(Debug, Clone, Copy)]
/// A safe provider-native operation used only by an explicit health check.
pub(in crate::tool_provider) enum McpProviderReadinessProbe {
    /// Calls a read-only MCP tool with an empty argument object.
    Tool(&'static str),
}

#[derive(Debug, Clone, Copy)]
/// Provider-specific setup Windie runs before starting an MCP process.
pub(in crate::tool_provider) enum McpProviderSetup {
    BasicMemoryProject,
    DesktopCommanderConfig,
}

#[derive(Debug, Clone)]
/// Provider for an approved local or hosted MCP server.
pub(in crate::tool_provider) struct McpToolProvider {
    manifest: ProviderManifest,
    pub(in crate::tool_provider) provider_id: ToolProviderId,
    pub(in crate::tool_provider) schema_prefix: &'static str,
    pub(in crate::tool_provider) display_name: &'static str,
    pub(in crate::tool_provider) transport: McpTransport,
    pub(in crate::tool_provider) package_command: Option<McpCommand>,
    readiness_probe: Option<McpProviderReadinessProbe>,
    setup: Option<McpProviderSetup>,
    cleanup: ProviderCleanup,
}

impl McpToolProvider {
    /// Builds a runtime provider from a code-approved provider definition.
    pub(in crate::tool_provider) fn new(definition: McpProviderDefinition) -> Self {
        Self {
            manifest: definition.manifest,
            provider_id: ToolProviderId::new(definition.provider_id),
            schema_prefix: definition.schema_prefix,
            display_name: definition.display_name,
            transport: definition.transport,
            package_command: definition.package_command,
            readiness_probe: definition.readiness_probe,
            setup: definition.setup,
            cleanup: definition.cleanup,
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
        Ok(mcp::list_tools_with_transport(self.transport)?
            .into_iter()
            .map(|tool| self.definition_from_mcp_tool(tool))
            .collect())
    }

    /// Lists tools through the async transport path used by runtime execution.
    pub(in crate::tool_provider) async fn list_tools_async(&self) -> Result<Vec<ToolDefinition>> {
        self.prepare()?;
        Ok(mcp::list_tools_with_transport_async(self.transport)
            .await?
            .into_iter()
            .map(|tool| self.definition_from_mcp_tool(tool))
            .collect())
    }

    /// Prepares the provider package without starting its MCP protocol.
    pub(in crate::tool_provider) fn prepare_package(&self) -> Result<()> {
        let Some(command) = self.package_command else {
            return Ok(());
        };

        mcp::run_preparation_command(command)
    }

    /// Runs the provider's optional non-mutating browser/service readiness
    /// probe. Catalog discovery intentionally remains separate because some
    /// MCP servers expose tools before starting their external application.
    pub(in crate::tool_provider) fn check_readiness(&self) -> Result<()> {
        let Some(McpProviderReadinessProbe::Tool(tool_name)) = self.readiness_probe else {
            return Ok(());
        };

        let result = mcp::call_tool_with_transport(self.transport, tool_name, json!({}))?;
        if result
            .get("isError")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
        {
            return Err(anyhow::anyhow!(
                "MCP readiness probe reported an error: {tool_name}"
            ));
        }

        Ok(())
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
            permissions: self.tool_permissions(),
            annotations: ToolAnnotations {
                title: None,
                read_only: Some(read_only),
            },
        }
    }

    /// Runs provider-specific setup before Windie starts the MCP process.
    pub(in crate::tool_provider) fn prepare(&self) -> Result<()> {
        match self.setup {
            Some(McpProviderSetup::BasicMemoryProject) => basic_memory::prepare(),
            Some(McpProviderSetup::DesktopCommanderConfig) => desktop_commander::prepare(),
            None => Ok(()),
        }
    }

    /// Removes this provider's Windie-owned local runtime and configuration.
    pub(in crate::tool_provider) fn uninstall(&self, remove_runtime: bool) -> Result<()> {
        match self.cleanup {
            ProviderCleanup::None => {}
            ProviderCleanup::CuaDriver => crate::local::uninstall_cua_driver()?,
            ProviderCleanup::WindieDirectories(paths) => {
                crate::local::remove_windie_directories(paths)?;
            }
            ProviderCleanup::BasicMemory => basic_memory::uninstall()?,
        }

        if remove_runtime {
            crate::local::remove_managed_runtime(self.manifest.runtime)?;
        }

        Ok(())
    }

    /// Returns the permission lane required by the provider transport.
    fn tool_permissions(&self) -> Vec<ToolPermission> {
        match self.transport {
            McpTransport::Stdio { .. } => vec![ToolPermission::ExternalProcess],
            McpTransport::StreamableHttp { .. } => vec![ToolPermission::Network],
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
