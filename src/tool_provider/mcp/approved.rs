//! Code-approved MCP provider allowlist.
//!
//! This is a code-owned allowlist, not user configuration. Provider
//! availability still does not grant model access; conversations must expose
//! individual tools before their schemas are sent to the model.

use super::McpProviderDefinition;
use super::{blender, brightdata, cua, desktop_commander};

/// Returns the MCP providers Windie is willing to start and execute.
pub(in crate::tool_provider) fn approved_mcp_providers() -> Vec<McpProviderDefinition> {
    vec![
        cua::definition(),
        desktop_commander::definition(),
        blender::definition(),
        brightdata::definition(),
    ]
}

/// Finds one approved MCP provider definition for tests.
#[cfg(test)]
pub(in crate::tool_provider) fn approved_mcp_provider(
    provider_id: &str,
) -> Option<McpProviderDefinition> {
    approved_mcp_providers()
        .into_iter()
        .find(|definition| definition.provider_id == provider_id)
}
