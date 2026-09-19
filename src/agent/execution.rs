//! Agent adapter from a fenced hosted assignment to existing local execution.
//!
//! This module owns no MCP protocol implementation. It re-loads the local
//! package, SQLite catalog, and provider registry at execution time so a
//! removed, disabled, or changed plugin is refused before executor entry.

use anyhow::{Result, anyhow};

use crate::{
    conversation::ToolCall,
    device::{DeviceWork, DeviceWorkAssignment, DeviceWorkResult},
    plugin::{PluginCatalog, PluginState, PluginStore, bundled_index},
    store::Store,
    tool::{ToolExecutionResult, ToolProviderRegistry},
};

/// Executes exactly one already-started assignment through Windie's existing
/// package reader or provider registry. Expected execution failures are
/// returned as normalized tool results so the hosted model can explain them.
pub(crate) async fn execute(assignment: &DeviceWorkAssignment) -> DeviceWorkResult {
    let result = match &assignment.work {
        DeviceWork::McpCall {
            tool_call_id,
            plugin_id,
            component_id,
            provider_id,
            schema_name,
            tool_name,
            schema,
            arguments,
        } => {
            execute_mcp(
                tool_call_id,
                plugin_id,
                component_id,
                provider_id,
                schema_name,
                tool_name,
                schema,
                arguments,
            )
            .await
        }
        DeviceWork::ReadSkill {
            tool_call_id,
            plugin_id,
            skill_id,
        } => read_skill(tool_call_id, plugin_id, skill_id),
    };
    match result {
        Ok(result) => wire_result(result),
        Err(error) => DeviceWorkResult {
            success: false,
            content: serde_json::json!({"error": error.to_string()}).to_string(),
            parts: Vec::new(),
        },
    }
}

async fn execute_mcp(
    tool_call_id: &str,
    plugin_id: &str,
    component_id: &str,
    provider_id: &str,
    schema_name: &str,
    tool_name: &str,
    schema: &crate::tool::ToolSchema,
    arguments: &str,
) -> Result<ToolExecutionResult> {
    let store = Store::open()?;
    let plugins = std::sync::Arc::new(PluginStore::default_store()?);
    let catalog = PluginCatalog::new(plugins, bundled_index()?);
    let registry = ToolProviderRegistry::with_installed_plugins()?;
    let snapshot = catalog.build_capability_snapshot(&store, &registry)?;
    let provider = snapshot
        .providers
        .iter()
        .find(|provider| {
            provider.plugin_id == plugin_id
                && provider.component_id == component_id
                && provider.provider_id.as_str() == provider_id
        })
        .ok_or_else(|| anyhow!("assigned plugin component is no longer installed"))?;
    if provider.state != PluginState::Enabled {
        return Err(anyhow!(
            "assigned plugin component is not enabled and ready"
        ));
    }
    let tool = provider
        .tools
        .iter()
        .find(|tool| {
            tool.schema_name.as_str() == schema_name
                && tool.provider.tool_name.as_str() == tool_name
                && tool.provider.provider_id.as_str() == provider_id
                && tool.schema_name == schema.name
                && tool.description == schema.description
                && tool.parameters == schema.parameters
        })
        .ok_or_else(|| anyhow!("assigned tool schema is stale or unavailable"))?;
    let call = ToolCall::function(tool_call_id, schema_name, arguments);
    registry.call_tool(&tool.attached_tool(), &call).await
}

fn read_skill(tool_call_id: &str, plugin_id: &str, skill_id: &str) -> Result<ToolExecutionResult> {
    let store = Store::open()?;
    let plugins = PluginStore::default_store()?;
    let catalog = PluginCatalog::new(std::sync::Arc::new(plugins.clone()), bundled_index()?);
    let snapshot = catalog.build_capability_snapshot(&store, &ToolProviderRegistry::new())?;
    if !snapshot.index.installed.iter().any(|plugin| {
        plugin.id == plugin_id && plugin.skills.iter().any(|skill| skill.id == skill_id)
    }) {
        return Err(anyhow!(
            "assigned skill is no longer present in the current capability catalog"
        ));
    }
    let plugin = plugins
        .installed_plugin(plugin_id)?
        .ok_or_else(|| anyhow!("assigned plugin is no longer installed"))?;
    let content = plugin.read_skill(skill_id)?;
    Ok(ToolExecutionResult::success_with_parts(
        crate::conversation::ToolCallId::new(tool_call_id),
        "windie__read_skill",
        content.clone(),
        vec![crate::conversation::UnsavedMessagePart::Text(content)],
    ))
}

fn wire_result(result: ToolExecutionResult) -> DeviceWorkResult {
    DeviceWorkResult {
        success: result.success,
        content: result.content,
        parts: result.parts,
    }
}
