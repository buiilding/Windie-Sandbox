//! Windie-owned control tools.
//!
//! Builtin tools are the runtime's control-plane API to the model. They are
//! represented using the same `tool` contracts as MCP tools, but their schemas
//! and execution remain owned by Windie rather than an external server.

mod definitions;

pub(crate) use definitions::{
    ATTACH_EXTENSION_TOOL_NAME, ATTACH_PROVIDER_TOOL_NAME, BUILTIN_PROVIDER_ID,
    LIST_PROVIDERS_TOOL_NAME, READ_SKILL_TOOL_NAME, definitions, model_definitions,
};

use anyhow::Result;
use serde_json::Value;

use crate::conversation::ConversationId;
use crate::error;
use crate::mcp::{McpRegistry, ProviderManifest};
use crate::plugins::{ExtensionTarget, PluginId, PluginRegistry};
use crate::skills::{SkillId, SkillPath};
use crate::store::{ProviderCatalogStatus, Store};
use crate::tool::{
    AttachedTool, ToolDefinition, ToolExecutionResult, ToolProviderId, ToolProviderKind,
    ToolSchemaName,
};

/// Returns the builtin definition matching a model-facing schema name.
pub(crate) fn find(schema_name: &ToolSchemaName) -> Option<ToolDefinition> {
    definitions()
        .into_iter()
        .find(|definition| definition.schema_name == *schema_name)
}

/// Returns whether Windie owns and can execute one builtin tool.
pub(crate) fn can_execute(attached_tool: &AttachedTool) -> bool {
    attached_tool.provider.kind == ToolProviderKind::Builtin
        && attached_tool.provider.provider_id.as_str() == BUILTIN_PROVIDER_ID
        && find(&attached_tool.schema_name).is_some()
}

/// Executes a Windie-owned control tool.
pub(crate) fn execute(
    store: &mut Store,
    conversation_id: &ConversationId,
    tool_call: &crate::conversation::ToolCall,
    attached_tool: &AttachedTool,
    registry: &McpRegistry,
) -> Result<ToolExecutionResult> {
    if !can_execute(attached_tool) {
        return Ok(ToolExecutionResult::failure(
            tool_call.id.clone(),
            tool_call.name(),
            "unknown built-in tool",
        ));
    }

    match attached_tool.provider.tool_name.as_str() {
        LIST_PROVIDERS_TOOL_NAME => Ok(ToolExecutionResult::success(
            tool_call.id.clone(),
            tool_call.name(),
            list_attachable_providers(store, enabled_provider_manifests(store, registry)?)?,
        )),
        ATTACH_PROVIDER_TOOL_NAME => {
            let arguments = parse_arguments(tool_call)?;
            let Some(provider_id) = arguments.get("provider_id").and_then(Value::as_str) else {
                return Ok(ToolExecutionResult::failure(
                    tool_call.id.clone(),
                    tool_call.name(),
                    "provider_id is required",
                ));
            };
            match registry.attach_provider_tools(
                store,
                conversation_id,
                &ToolProviderId::new(provider_id),
            ) {
                Ok(_) => Ok(ToolExecutionResult::success(
                    tool_call.id.clone(),
                    tool_call.name(),
                    "provider attached",
                )),
                Err(error) => Ok(ToolExecutionResult::failure(
                    tool_call.id.clone(),
                    tool_call.name(),
                    error.to_string(),
                )),
            }
        }
        READ_SKILL_TOOL_NAME => {
            let arguments = parse_arguments(tool_call)?;
            let Some(plugin_id) = arguments.get("plugin_id").and_then(Value::as_str) else {
                return Ok(ToolExecutionResult::failure(
                    tool_call.id.clone(),
                    tool_call.name(),
                    "plugin_id is required",
                ));
            };
            let Some(skill_id) = arguments.get("skill_id").and_then(Value::as_str) else {
                return Ok(ToolExecutionResult::failure(
                    tool_call.id.clone(),
                    tool_call.name(),
                    "skill_id is required",
                ));
            };
            let path = match arguments.get("path").and_then(Value::as_str) {
                Some(path) => match SkillPath::new(path) {
                    Ok(path) => Some(path),
                    Err(error) => {
                        return Ok(ToolExecutionResult::failure(
                            tool_call.id.clone(),
                            tool_call.name(),
                            error,
                        ));
                    }
                },
                None => None,
            };
            match PluginRegistry::default().read_skill(
                &PluginId::new(plugin_id),
                &SkillId::new(skill_id),
                path.as_ref(),
            ) {
                Ok(document) => Ok(ToolExecutionResult::success(
                    tool_call.id.clone(),
                    tool_call.name(),
                    document.as_model_text(),
                )),
                Err(error) => Ok(ToolExecutionResult::failure(
                    tool_call.id.clone(),
                    tool_call.name(),
                    error.to_string(),
                )),
            }
        }
        ATTACH_EXTENSION_TOOL_NAME => {
            let arguments = parse_arguments(tool_call)?;
            let Some(target) = arguments.get("target").and_then(Value::as_str) else {
                return Ok(ToolExecutionResult::failure(
                    tool_call.id.clone(),
                    tool_call.name(),
                    "target is required",
                ));
            };
            let target = match target.strip_prefix("plugin:") {
                Some(plugin_id) if !plugin_id.is_empty() => {
                    ExtensionTarget::Plugin(PluginId::new(plugin_id))
                }
                None => match target.strip_prefix("mcp:") {
                    Some(provider_id) if !provider_id.is_empty() => {
                        ExtensionTarget::Mcp(ToolProviderId::new(provider_id))
                    }
                    _ => {
                        return Ok(ToolExecutionResult::failure(
                            tool_call.id.clone(),
                            tool_call.name(),
                            "target must use plugin:<plugin_id> or mcp:<server_id>",
                        ));
                    }
                },
                _ => {
                    return Ok(ToolExecutionResult::failure(
                        tool_call.id.clone(),
                        tool_call.name(),
                        "target must use plugin:<plugin_id> or mcp:<server_id>",
                    ));
                }
            };
            match PluginRegistry::default().attach_extension(
                store,
                conversation_id,
                &target,
                registry,
            ) {
                Ok(names) => Ok(ToolExecutionResult::success(
                    tool_call.id.clone(),
                    tool_call.name(),
                    serde_json::json!({
                        "target": target_label(&target),
                        "attached_tools": names.iter().map(ToString::to_string).collect::<Vec<_>>()
                    })
                    .to_string(),
                )),
                Err(error) => Ok(ToolExecutionResult::failure(
                    tool_call.id.clone(),
                    tool_call.name(),
                    error.to_string(),
                )),
            }
        }
        _ => Ok(ToolExecutionResult::failure(
            tool_call.id.clone(),
            tool_call.name(),
            "unknown built-in tool",
        )),
    }
}

fn target_label(target: &ExtensionTarget) -> String {
    match target {
        ExtensionTarget::Plugin(plugin_id) => format!("plugin:{plugin_id}"),
        ExtensionTarget::Mcp(provider_id) => format!("mcp:{provider_id}"),
    }
}

fn parse_arguments(tool_call: &crate::conversation::ToolCall) -> Result<Value> {
    serde_json::from_str(tool_call.arguments())
        .map_err(|error| error::invalid_request(format!("invalid tool arguments: {error}")))
}

fn list_attachable_providers(store: &Store, manifests: Vec<ProviderManifest>) -> Result<String> {
    let mut lines = vec!["provider_id, description".to_string()];
    for manifest in manifests {
        let Some(catalog) = store.load_provider_tool_catalog(&manifest.provider_id)? else {
            continue;
        };
        if catalog.status != ProviderCatalogStatus::Unavailable {
            lines.push(format!(
                "{}, {}",
                manifest.provider_id.as_str(),
                manifest.description
            ));
        }
    }
    Ok(lines.join("\n"))
}

fn enabled_provider_manifests(
    store: &Store,
    registry: &McpRegistry,
) -> Result<Vec<ProviderManifest>> {
    Ok(registry
        .provider_manifests()
        .into_iter()
        .filter(|manifest| {
            store
                .provider_is_enabled(&manifest.provider_id)
                .unwrap_or(false)
        })
        .collect())
}
