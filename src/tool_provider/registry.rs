//! Provider-neutral tool registry.
//!
//! The registry owns live discovery and dispatch across executable backend
//! families. The Store owns persisted provider catalogs. This module does not
//! know backend-specific setup details such as Desktop Commander configuration
//! or MCP result normalization.

use anyhow::Result;

use super::builtin;
use super::mcp::McpProviderDefinition;
use super::mcp::{McpToolProvider, approved_mcp_providers};
use crate::conversation::ToolCall;
use crate::error;
use crate::local;
use crate::mcp::McpCommand;
use crate::mcp::McpSessionPool;
use crate::mcp::McpTransport;
use crate::tool::{
    AttachedTool, ToolDefinition, ToolExecutionResult, ToolProviderId, ToolProviderKind,
    ToolSchemaName,
};
use crate::tool_provider::ProviderRuntime;

#[derive(Debug, Clone)]
/// Registry of tool providers available to this Windie process.
///
/// The registry deliberately exposes provider-neutral operations. Runtime does
/// not branch on shell, MCP, or plugin details; it resolves the conversation's
/// attached tool to a provider reference and calls this registry.
pub struct ToolProviderRegistry {
    pub(super) mcp_providers: Vec<McpToolProvider>,
    pub(super) mcp_session_pool: Option<McpSessionPool>,
}

impl ToolProviderRegistry {
    /// Builds the default registry for the local Windie process.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns Windie-owned control tools that are always model-visible.
    pub fn builtin_tools(&self) -> Vec<ToolDefinition> {
        builtin::definitions()
    }

    /// Finds one Windie-owned control tool by its model-facing schema name.
    pub fn builtin_tool(&self, schema_name: &ToolSchemaName) -> Option<ToolDefinition> {
        self.builtin_tools()
            .into_iter()
            .find(|tool| tool.schema_name == *schema_name)
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
            ProviderRuntime::Native => local::install_target(provider_id.as_str()).map(|_| ()),
            runtime => local::ensure_runtime(runtime).map(|_| ()),
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
            ToolProviderKind::Builtin => true,
            ToolProviderKind::Mcp => self
                .mcp_provider(&attached_tool.provider.provider_id)
                .is_some(),
            ToolProviderKind::Plugin => false,
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
            ToolProviderKind::Plugin => Err(error::invalid_request(format!(
                "unknown tool: {}",
                tool_call.name()
            ))),
        }
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
                manifest: crate::tool_provider::ProviderManifest::mcp_stdio(
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

impl Default for ToolProviderRegistry {
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
