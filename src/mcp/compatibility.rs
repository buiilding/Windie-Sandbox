//! Code-approved MCP provider compatibility definitions.
//!
//! This module is temporary migration compatibility for providers that have
//! not yet moved into Windie's installed plugin store. Provider availability
//! still does not grant model access; conversations must expose individual
//! tools before their schemas are sent to the model.

use super::McpProviderDefinition;
use super::legacy_parallel;

/// Returns the MCP providers Windie is willing to start and execute.
pub(crate) fn approved_mcp_providers() -> Vec<McpProviderDefinition> {
    vec![legacy_parallel::definition()]
}

/// Finds one approved MCP provider definition for tests.
#[cfg(test)]
pub(crate) fn approved_mcp_provider(provider_id: &str) -> Option<McpProviderDefinition> {
    approved_mcp_providers()
        .into_iter()
        .find(|definition| definition.provider_id == provider_id)
}
