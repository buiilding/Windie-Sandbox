//! Generic MCP tool provider adapter.
//!
//! This adapter knows how to list tools from one approved MCP stdio server and
//! expose them as Windie tool definitions. Executing an already-approved MCP
//! call lives in `execution.rs`.

use anyhow::Result;
use serde_json::json;
use std::sync::{Arc, RwLock};

use super::chrome_devtools;
use crate::mcp::{self, McpCommand, McpOwnedCommand, McpTool, McpTransport};
use crate::tool::ProviderManifest;
use crate::tool::{
    ProviderToolName, ToolAnnotations, ToolDefinition, ToolPermission, ToolProviderId,
    ToolProviderKind, ToolProviderRef, ToolSchemaName,
};

#[derive(Debug, Clone)]
/// Static compatibility definition for one not-yet-migrated MCP provider.
///
/// This is intentionally data, not runtime state. Adding a future approved MCP
/// provider should add one server definition while keeping `McpToolProvider`
/// generic.
pub(crate) struct McpProviderDefinition {
    pub(crate) manifest: ProviderManifest,
    pub(crate) provider_id: String,
    pub(crate) schema_prefix: String,
    pub(crate) display_name: String,
    pub(crate) transport: McpTransport,
    pub(crate) package_command: Option<McpCommand>,
    pub(crate) owned_package_command: Option<McpOwnedCommand>,
    pub(crate) readiness_probe: Option<String>,
}

#[derive(Debug, Clone)]
/// Provider for one local or hosted MCP server.
pub(crate) struct McpToolProvider {
    manifest: ProviderManifest,
    pub(crate) provider_id: ToolProviderId,
    pub(crate) schema_prefix: String,
    pub(crate) display_name: String,
    transport: Arc<RwLock<McpTransport>>,
    pub(crate) package_command: Option<McpCommand>,
    owned_package_command: Option<McpOwnedCommand>,
    readiness_probe: Option<String>,
}

impl McpToolProvider {
    /// Builds a runtime provider from a static or package-owned definition.
    pub(crate) fn new(definition: McpProviderDefinition) -> Self {
        Self {
            manifest: definition.manifest,
            provider_id: ToolProviderId::new(definition.provider_id),
            schema_prefix: definition.schema_prefix,
            display_name: definition.display_name,
            transport: Arc::new(RwLock::new(definition.transport)),
            package_command: definition.package_command,
            owned_package_command: definition.owned_package_command,
            readiness_probe: definition.readiness_probe,
        }
    }

    /// Returns the stable provider ID used by attachments and dispatch.
    pub(crate) fn id(&self) -> &ToolProviderId {
        &self.provider_id
    }

    /// Returns the metadata contract for this provider.
    pub(crate) fn manifest(&self) -> &ProviderManifest {
        &self.manifest
    }

    /// Replaces the runtime transport used by this provider. Existing MCP
    /// sessions are stopped by the registry before this method is called.
    pub(crate) fn set_transport(&self, transport: McpTransport) {
        *self
            .transport
            .write()
            .expect("provider transport lock poisoned") = transport;
    }

    /// Applies the persisted Chrome DevTools mode to its package-owned launcher.
    pub(crate) fn set_chrome_devtools_mode(
        &self,
        mode: chrome_devtools::ChromeDevToolsConnectionMode,
    ) -> bool {
        if self.provider_id.as_str() != "chrome-devtools" {
            return false;
        }
        let transport = self.transport();
        match transport {
            McpTransport::PackagedStdio {
                mut command,
                shutdown_command,
            } => {
                let mut updated = false;
                for (name, value) in &mut command.env {
                    if name == chrome_devtools::CHROME_DEVTOOLS_CONNECTION_MODE_ENV {
                        *value = mode.as_storage().to_string();
                        updated = true;
                    }
                }
                if !updated {
                    command.env.push((
                        chrome_devtools::CHROME_DEVTOOLS_CONNECTION_MODE_ENV.to_string(),
                        mode.as_storage().to_string(),
                    ));
                }
                self.set_transport(McpTransport::PackagedStdio {
                    command,
                    shutdown_command,
                });
            }
            _ => return false,
        }
        true
    }

    /// Reads the current runtime transport for one MCP operation.
    pub(crate) fn transport(&self) -> McpTransport {
        self.transport
            .read()
            .expect("provider transport lock poisoned")
            .clone()
    }

    /// Lists tools from the MCP server and maps them into Windie definitions.
    pub(crate) fn list_tools(&self) -> Result<Vec<ToolDefinition>> {
        self.prepare()?;
        Ok(mcp::list_tools_with_transport(self.transport())?
            .into_iter()
            .map(|tool| self.definition_from_mcp_tool(tool))
            .collect())
    }

    /// Lists tools through the async transport path used by runtime execution.
    pub(crate) async fn list_tools_async(&self) -> Result<Vec<ToolDefinition>> {
        self.prepare()?;
        Ok(mcp::list_tools_with_transport_async(self.transport())
            .await?
            .into_iter()
            .map(|tool| self.definition_from_mcp_tool(tool))
            .collect())
    }

    /// Prepares the provider package without starting its MCP protocol.
    pub(crate) fn prepare_package(&self) -> Result<()> {
        if let Some(command) = self.package_command {
            return mcp::run_preparation_command(command);
        }
        if let Some(command) = self.owned_package_command.clone() {
            return mcp::run_owned_preparation_command(command);
        }
        Ok(())
    }

    /// Runs the provider's optional non-mutating browser/service readiness
    /// probe. Catalog discovery intentionally remains separate because some
    /// MCP servers expose tools before starting their external application.
    pub(crate) fn check_readiness(&self) -> Result<()> {
        let Some(tool_name) = &self.readiness_probe else {
            return Ok(());
        };

        let result = mcp::call_tool_with_transport(self.transport(), tool_name, json!({}))?;
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
    pub(crate) fn definition_from_mcp_tool(&self, tool: McpTool) -> ToolDefinition {
        let read_only = tool
            .annotations
            .as_ref()
            .and_then(|annotations| annotations.read_only_hint)
            .unwrap_or(false);

        ToolDefinition {
            schema_name: ToolSchemaName::new(mcp_schema_name(&self.schema_prefix, &tool.name)),
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
    pub(crate) fn prepare(&self) -> Result<()> {
        Ok(())
    }

    /// Removes this provider's Windie-owned managed runtime.
    pub(crate) fn uninstall(&self, remove_runtime: bool) -> Result<()> {
        if remove_runtime {
            crate::managed_runtime::remove_managed_runtime(self.manifest.runtime)?;
        }

        Ok(())
    }

    /// Returns the permission lane required by the provider transport.
    fn tool_permissions(&self) -> Vec<ToolPermission> {
        match self.transport() {
            McpTransport::Stdio { .. } | McpTransport::PackagedStdio { .. } => {
                vec![ToolPermission::ExternalProcess]
            }
            McpTransport::StreamableHttp { .. } => vec![ToolPermission::Network],
        }
    }
}

/// Builds the model-facing schema name for one MCP provider tool.
pub(crate) fn mcp_schema_name(schema_prefix: &str, tool_name: &str) -> String {
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
