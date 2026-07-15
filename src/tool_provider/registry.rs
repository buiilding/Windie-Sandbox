//! Provider-neutral tool registry.
//!
//! The registry owns catalog caching and dispatch across executable backend
//! families. It does not know backend-specific setup details such as Desktop
//! Commander configuration or MCP result normalization.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use anyhow::{Result, anyhow};

#[cfg(test)]
use super::mcp::McpProviderDefinition;
use super::mcp::{McpToolProvider, approved_mcp_providers};
use crate::conversation::ToolCall;
use crate::error;
#[cfg(test)]
use crate::mcp::McpCommand;
use crate::mcp::McpSessionPool;
use crate::tool::{
    AttachedTool, ProviderToolName, ToolDefinition, ToolExecutionResult, ToolProviderId,
    ToolProviderKind,
};

#[derive(Debug, Clone)]
/// Catalog status for one approved provider.
///
/// The aggregate tool list must not hide provider startup failures. Clients use
/// this record to show which approved providers are ready and which need local
/// setup such as a missing command or provider key.
pub struct ToolProviderStatus {
    pub provider_id: ToolProviderId,
    pub display_name: String,
    pub available: bool,
    pub tool_count: usize,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
/// Registry of tool providers available to this Windie process.
///
/// The registry deliberately exposes provider-neutral operations. Runtime does
/// not branch on shell, MCP, or plugin details; it resolves the conversation's
/// attached tool to a provider reference and calls this registry.
pub struct ToolProviderRegistry {
    pub(super) mcp_providers: Vec<McpToolProvider>,
    pub(super) mcp_session_pool: Option<McpSessionPool>,
    pub(super) catalog_cache: Arc<Mutex<HashMap<ToolProviderId, Vec<ToolDefinition>>>>,
}

impl ToolProviderRegistry {
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

    /// Lists every provider tool that clients may attach to conversations.
    ///
    /// Availability does not grant model access. Clients still need to attach a
    /// returned definition before the model sees the function schema. Provider
    /// catalogs loaded here are cached for later attachment requests in the same
    /// process.
    pub fn list_available_tools(&self) -> Result<Vec<ToolDefinition>> {
        let mut tools = Vec::new();
        for provider in &self.mcp_providers {
            if let Ok(provider_tools) = self.list_provider_tools(provider.id()) {
                tools.extend(provider_tools);
            }
        }

        Ok(tools)
    }

    /// Lists every approved provider with either a loaded tool count or a
    /// concrete availability error.
    pub fn list_provider_statuses(&self) -> Vec<ToolProviderStatus> {
        self.mcp_providers
            .iter()
            .map(|provider| match self.list_provider_tools(provider.id()) {
                Ok(tools) => ToolProviderStatus {
                    provider_id: provider.provider_id.clone(),
                    display_name: provider.display_name.to_string(),
                    available: true,
                    tool_count: tools.len(),
                    error: None,
                },
                Err(error) => ToolProviderStatus {
                    provider_id: provider.provider_id.clone(),
                    display_name: provider.display_name.to_string(),
                    available: false,
                    tool_count: 0,
                    error: Some(error.to_string()),
                },
            })
            .collect()
    }

    /// Lists available tools for one provider ID.
    ///
    /// MCP provider catalogs can require starting a provider process for
    /// `tools/list`. The API server keeps one registry for the process, so this
    /// method caches successful catalog loads and lets later attachment
    /// resolution reuse the backend-owned schema copy.
    pub fn list_provider_tools(&self, provider_id: &ToolProviderId) -> Result<Vec<ToolDefinition>> {
        if let Some(tools) = self.cached_provider_tools(provider_id)? {
            return Ok(tools);
        }
        if let Some(provider) = self.mcp_provider(provider_id) {
            let tools = provider.list_tools()?;
            self.cache_provider_tools(provider_id, &tools)?;
            return Ok(tools);
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
        match attached_tool.provider.kind {
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

    /// Returns a cached provider catalog when this process has already loaded
    /// one.
    fn cached_provider_tools(
        &self,
        provider_id: &ToolProviderId,
    ) -> Result<Option<Vec<ToolDefinition>>> {
        let cache = self
            .catalog_cache
            .lock()
            .map_err(|_| anyhow!("tool provider catalog cache lock was poisoned"))?;

        Ok(cache.get(provider_id).cloned())
    }

    /// Stores one backend-owned provider catalog for reuse by later operations.
    fn cache_provider_tools(
        &self,
        provider_id: &ToolProviderId,
        tools: &[ToolDefinition],
    ) -> Result<()> {
        let mut cache = self
            .catalog_cache
            .lock()
            .map_err(|_| anyhow!("tool provider catalog cache lock was poisoned"))?;
        cache.insert(provider_id.clone(), tools.to_vec());

        Ok(())
    }

    /// Builds a test registry with one fake MCP provider and an already-loaded
    /// catalog.
    ///
    /// Runtime tests use this to exercise provider dispatch without depending
    /// on user-installed MCP binaries.
    #[cfg(test)]
    pub(crate) fn with_test_mcp_provider(
        provider_id: &'static str,
        schema_prefix: &'static str,
        display_name: &'static str,
        command: McpCommand,
        tools: Vec<ToolDefinition>,
    ) -> Self {
        let provider_id_value = ToolProviderId::new(provider_id);
        let catalog_cache = Arc::new(Mutex::new(HashMap::from([(provider_id_value, tools)])));

        Self {
            mcp_providers: vec![McpToolProvider::new(McpProviderDefinition {
                provider_id,
                schema_prefix,
                display_name,
                command,
                shutdown_command: None,
                setup: None,
            })],
            mcp_session_pool: None,
            catalog_cache,
        }
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
            catalog_cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}
