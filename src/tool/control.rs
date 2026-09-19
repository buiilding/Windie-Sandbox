//! Pure planning for Windie-owned context controls.
//!
//! Package loading, local skill reads, and attachment writes belong to adapters.
//! These rules operate only on loaded facts and never start a provider.

use std::collections::HashSet;

use anyhow::Result;
use serde_json::Value;

use super::{AttachedTool, ToolDefinition, ToolProviderId, ToolSchemaName};
use crate::{conversation::ToolCall, error};

/// A fully parsed control request, shared by local and hosted adapters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlRequest {
    ReadSkill { plugin_id: String, skill_id: String },
    AttachMcp { plugin_id: String, mcp_id: String },
}

impl ControlRequest {
    /// Parses the existing model-facing contract without persistence access.
    /// Unknown controls are rejected, not treated as provider commands.
    pub fn parse(native_name: &str, call: &ToolCall) -> Result<Self> {
        let arguments: Value = serde_json::from_str(call.arguments())
            .map_err(|err| error::invalid_request(format!("invalid tool arguments: {err}")))?;
        let field = |name: &str| -> Result<String> {
            arguments
                .get(name)
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| error::invalid_request(format!("{name} is required")))
        };
        match native_name {
            super::READ_SKILL_TOOL_NAME => Ok(Self::ReadSkill {
                plugin_id: field("plugin_id")?,
                skill_id: field("skill_id")?,
            }),
            super::ATTACH_MCP_TOOL_NAME => Ok(Self::AttachMcp {
                plugin_id: field("plugin_id")?,
                mcp_id: field("mcp_id")?,
            }),
            _ => Err(error::invalid_request("unknown built-in tool")),
        }
    }
}

/// Validates exact plugin/component membership using either loaded manifests
/// or a validated device snapshot. A globally registered provider is not proof
/// that it belongs to the requested plugin.
pub fn validate_mcp_membership<'a>(
    plugin_id: &str,
    mcp_id: &str,
    component_ids: impl IntoIterator<Item = &'a str>,
) -> Result<()> {
    if component_ids.into_iter().any(|id| id == mcp_id) {
        return Ok(());
    }
    Err(error::invalid_request(format!(
        "MCP does not exist in plugin: {plugin_id}/{mcp_id}"
    )))
}

/// Read-only facts needed to attach a discovered provider. Absence differs
/// from an empty (valid) catalog. Stale discovery remains usable as in the local
/// runtime; an explicitly unavailable catalog does not.
pub struct AttachmentFacts<'a> {
    pub registered: bool,
    pub enabled: bool,
    pub unavailable: bool,
    pub tools: Option<&'a [ToolDefinition]>,
}

/// Returns new attachments only. The caller persists them atomically in its
/// own backend and adds any hosted device/revision binding in that transaction.
pub fn plan_provider_attachment(
    provider_id: &ToolProviderId,
    facts: AttachmentFacts<'_>,
    existing_names: &HashSet<ToolSchemaName>,
) -> Result<Vec<AttachedTool>> {
    if !facts.registered {
        return Err(error::not_found(format!(
            "provider does not exist: {provider_id}"
        )));
    }
    if !facts.enabled {
        return Err(error::invalid_request(format!(
            "provider is not installed, enabled, and healthy: {provider_id}"
        )));
    }
    let tools = facts.tools.ok_or_else(|| {
        error::invalid_request(format!(
            "provider has no discovered tool catalog: {provider_id}"
        ))
    })?;
    if facts.unavailable {
        return Err(error::invalid_request(format!(
            "provider tool catalog is unavailable: {provider_id}"
        )));
    }
    Ok(tools
        .iter()
        .filter(|tool| !existing_names.contains(&tool.schema_name))
        .map(ToolDefinition::attached_tool)
        .collect())
}
