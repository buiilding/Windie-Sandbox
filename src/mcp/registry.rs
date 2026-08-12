//! Provider-neutral tool registry.
//!
//! The registry owns live discovery and dispatch across executable backend
//! families. The Store owns persisted provider catalogs. This module does not
//! know backend-specific setup details such as Desktop Commander configuration
//! or MCP result normalization.

use anyhow::Result;

use super::{ChromeDevToolsConnectionMode, ProviderRuntime};
use crate::conversation::ToolCall;
use crate::error;
use crate::mcp::servers::{McpProviderDefinition, McpToolProvider, approved_mcp_providers};
use crate::mcp::{McpCommand, McpSessionPool, McpTransport, ProviderInstallState};
use crate::store::{ProviderCatalogStatus, Store};
use crate::tool::{
    AttachedTool, ToolDefinition, ToolExecutionResult, ToolProviderId, ToolProviderKind,
    ToolSchemaName,
};

#[derive(Debug, Clone)]
/// Registry of tool providers available to this Windie process.
///
/// The registry deliberately exposes provider-neutral operations. Runtime does
/// not branch on shell, MCP, or plugin details; it resolves the conversation's
/// attached tool to a provider reference and calls this registry.
pub struct McpRegistry {
    pub(super) mcp_providers: Vec<McpToolProvider>,
    pub(super) mcp_session_pool: Option<McpSessionPool>,
}

impl McpRegistry {
    /// Builds the default registry for the local Windie process.
    pub fn new() -> Self {
        Self::default()
    }

    /// Builds a registry whose MCP tool calls reuse persistent provider
    /// sessions.
    ///
    /// The API server uses this shape because it lives long enough for idle
    /// cleanup to matter. CLI commands keep the default short-lived execution
    /// path because each CLI invocation is a separate process.
    pub fn with_persistent_mcp_sessions() -> Self {
        Self {
            mcp_session_pool: Some(McpSessionPool::new()),
            ..Self::default()
        }
    }

    /// Returns manifests for every provider known to this registry.
    pub fn provider_manifests(&self) -> Vec<super::manifest::ProviderManifest> {
        self.mcp_providers
            .iter()
            .map(|provider| provider.manifest().clone())
            .collect()
    }

    /// Applies the persisted Chrome DevTools connection mode to the live
    /// provider definition. The caller must stop any active session first.
    pub fn set_chrome_devtools_mode(&self, mode: ChromeDevToolsConnectionMode) -> Result<()> {
        let provider = self
            .mcp_provider(&ToolProviderId::new("chrome-devtools"))
            .ok_or_else(|| error::not_found("provider does not exist: chrome-devtools"))?;
        provider.set_chrome_devtools_mode(mode);
        Ok(())
    }

    /// Returns one known provider manifest by stable provider ID.
    pub fn provider_manifest(
        &self,
        provider_id: &ToolProviderId,
    ) -> Option<super::manifest::ProviderManifest> {
        self.mcp_providers
            .iter()
            .find(|provider| provider.id() == provider_id)
            .map(|provider| provider.manifest().clone())
    }

    /// Attaches one enabled MCP server's persisted tool catalog to a
    /// conversation. Discovery itself is separate and happens during setup
    /// or an explicit refresh.
    pub fn attach_provider_tools(
        &self,
        store: &mut Store,
        conversation_id: &crate::conversation::ConversationId,
        provider_id: &ToolProviderId,
    ) -> Result<Vec<ToolSchemaName>> {
        if self.provider_manifest(provider_id).is_none() {
            return Err(error::not_found(format!(
                "MCP server does not exist: {provider_id}"
            )));
        }
        let Some(installation) = store.load_installed_provider(provider_id)? else {
            return Err(error::invalid_request(format!(
                "MCP server is not installed: {provider_id}"
            )));
        };
        if installation.state != ProviderInstallState::Enabled || installation.error.is_some() {
            return Err(error::invalid_request(format!(
                "MCP server is not enabled and healthy: {provider_id}"
            )));
        }
        let Some(catalog) = store.load_provider_tool_catalog(provider_id)? else {
            return Err(error::invalid_request(format!(
                "MCP server has no discovered tool catalog: {provider_id}"
            )));
        };
        if catalog.status == ProviderCatalogStatus::Unavailable {
            return Err(error::invalid_request(format!(
                "MCP server tool catalog is unavailable: {provider_id}"
            )));
        }
        let existing_names = store
            .load_attached_tools(conversation_id)?
            .into_iter()
            .map(|tool| tool.schema_name)
            .collect::<std::collections::HashSet<_>>();
        let new_tools = catalog
            .tools
            .iter()
            .filter(|tool| !existing_names.contains(&tool.schema_name))
            .map(|tool| tool.attached_tool())
            .collect::<Vec<_>>();
        let names = new_tools
            .iter()
            .map(|tool| tool.schema_name.clone())
            .collect::<Vec<_>>();
        store.insert_attached_tools(conversation_id, &new_tools)?;
        Ok(names)
    }

    /// Runs provider-specific configuration without starting MCP.
    pub fn prepare_provider_configuration(&self, provider_id: &ToolProviderId) -> Result<()> {
        let provider = self
            .mcp_provider(provider_id)
            .ok_or_else(|| error::not_found(format!("provider does not exist: {provider_id}")))?;
        provider.prepare()
    }

    /// Installs or verifies the local runtime declared by one provider.
    pub fn prepare_provider_runtime(&self, provider_id: &ToolProviderId) -> Result<()> {
        let provider = self
            .mcp_provider(provider_id)
            .ok_or_else(|| error::not_found(format!("provider does not exist: {provider_id}")))?;
        if provider.manifest().dependencies.is_empty() {
            return Ok(());
        }

        match provider.manifest().runtime {
            ProviderRuntime::Native => crate::mcp::install_target(provider_id.as_str()).map(|_| ()),
            runtime => crate::mcp::ensure_runtime(runtime).map(|_| ()),
        }
    }

    /// Prefetches the provider package without starting its MCP protocol.
    pub fn prepare_provider_package(&self, provider_id: &ToolProviderId) -> Result<()> {
        let provider = self
            .mcp_provider(provider_id)
            .ok_or_else(|| error::not_found(format!("provider does not exist: {provider_id}")))?;
        provider.prepare_package()
    }

    /// Runs a provider-declared, non-mutating readiness probe after catalog
    /// discovery. Providers without a probe remain catalog-only health checks.
    pub fn check_provider_readiness(&self, provider_id: &ToolProviderId) -> Result<()> {
        let provider = self
            .mcp_provider(provider_id)
            .ok_or_else(|| error::not_found(format!("provider does not exist: {provider_id}")))?;
        provider.check_readiness()
    }

    /// Returns whether provider setup has a package-prefetch phase.
    pub fn provider_requires_package_preparation(
        &self,
        provider_id: &ToolProviderId,
    ) -> Result<bool> {
        let provider = self
            .mcp_provider(provider_id)
            .ok_or_else(|| error::not_found(format!("provider does not exist: {provider_id}")))?;
        Ok(provider.manifest().package.is_some())
    }

    /// Discovers one provider's tools by starting its MCP backend.
    ///
    /// This is an explicit refresh operation used by provider setup, repair,
    /// and health checks. Normal catalog reads go through SQLite instead of
    /// starting provider processes.
    pub fn discover_provider_tools(
        &self,
        provider_id: &ToolProviderId,
    ) -> Result<Vec<ToolDefinition>> {
        if let Some(provider) = self.mcp_provider(provider_id) {
            return provider.list_tools();
        }

        Err(error::not_found(format!(
            "provider does not exist: {provider_id}"
        )))
    }

    /// Discovers one provider's tools through the async transport path.
    pub async fn discover_provider_tools_async(
        &self,
        provider_id: &ToolProviderId,
    ) -> Result<Vec<ToolDefinition>> {
        if let Some(provider) = self.mcp_provider(provider_id) {
            return provider.list_tools_async().await;
        }

        Err(error::not_found(format!(
            "provider does not exist: {provider_id}"
        )))
    }

    /// Returns whether this process has an executor for the attached provider
    /// tool.
    pub fn can_execute(&self, attached_tool: &AttachedTool) -> bool {
        match attached_tool.provider.kind {
            ToolProviderKind::Builtin => false,
            ToolProviderKind::Mcp => self
                .mcp_provider(&attached_tool.provider.provider_id)
                .is_some(),
            ToolProviderKind::Manual => false,
        }
    }

    /// Executes one approved model tool call through its attached provider.
    pub async fn call_tool(
        &self,
        attached_tool: &AttachedTool,
        tool_call: &ToolCall,
    ) -> Result<ToolExecutionResult> {
        match attached_tool.provider.kind {
            ToolProviderKind::Builtin => Err(error::invalid_request(
                "built-in tools must be executed by the Windie runtime",
            )),
            ToolProviderKind::Mcp => {
                let Some(provider) = self.mcp_provider(&attached_tool.provider.provider_id) else {
                    return Err(error::invalid_request(format!(
                        "unknown tool: {}",
                        tool_call.name()
                    )));
                };

                provider
                    .call_tool(attached_tool, tool_call, self.mcp_session_pool.as_ref())
                    .await
            }
            ToolProviderKind::Manual => Err(error::invalid_request(format!(
                "unknown tool: {}",
                tool_call.name()
            ))),
        }
    }

    /// Stops one provider's persistent MCP session before its runtime is
    /// removed.
    pub async fn stop_provider_sessions(&self, provider_id: &ToolProviderId) {
        if let Some(session_pool) = &self.mcp_session_pool {
            session_pool.stop_provider(provider_id.as_str()).await;
        }
    }

    /// Removes one provider's Windie-owned runtime after its sessions stop.
    pub fn uninstall_provider_runtime(
        &self,
        provider_id: &ToolProviderId,
        remove_runtime: bool,
    ) -> Result<()> {
        let provider = self
            .mcp_provider(provider_id)
            .ok_or_else(|| error::not_found(format!("provider does not exist: {provider_id}")))?;
        provider.uninstall(remove_runtime)
    }

    /// Finds one approved MCP provider by its stable provider ID.
    fn mcp_provider(&self, provider_id: &ToolProviderId) -> Option<&McpToolProvider> {
        self.mcp_providers
            .iter()
            .find(|provider| provider.id() == provider_id)
    }

    /// Builds a deterministic fake MCP registry for provider-path benchmarks.
    ///
    /// The fake command still crosses the same registry, provider adapter, and
    /// MCP executor boundaries as a real approved provider; only its stdio
    /// command is deterministic and local.
    pub(crate) fn with_benchmark_mcp_provider(
        provider_id: &'static str,
        schema_prefix: &'static str,
        display_name: &'static str,
        command: McpCommand,
    ) -> Self {
        Self {
            mcp_providers: vec![McpToolProvider::new(McpProviderDefinition {
                manifest: crate::mcp::ProviderManifest::mcp_stdio(
                    provider_id,
                    display_name,
                    "Test MCP provider.",
                    command.program,
                    command.args,
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                ),
                provider_id,
                schema_prefix,
                display_name,
                transport: McpTransport::stdio(command),
                package_command: None,
                readiness_probe: None,
                setup: None,
                cleanup: crate::mcp::ProviderCleanup::None,
            })],
            mcp_session_pool: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_test_mcp_provider(
        provider_id: &'static str,
        schema_prefix: &'static str,
        display_name: &'static str,
        command: McpCommand,
    ) -> Self {
        Self::with_benchmark_mcp_provider(provider_id, schema_prefix, display_name, command)
    }
}

impl Default for McpRegistry {
    fn default() -> Self {
        Self {
            mcp_providers: approved_mcp_providers()
                .into_iter()
                .map(McpToolProvider::new)
                .collect(),
            mcp_session_pool: None,
        }
    }
}
