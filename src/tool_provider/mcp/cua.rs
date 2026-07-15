//! CUA Driver MCP provider definition.

use super::McpProviderDefinition;
use crate::mcp::McpCommand;

/// Returns the code-approved CUA Driver MCP provider definition.
pub(super) fn definition() -> McpProviderDefinition {
    McpProviderDefinition {
        provider_id: "cua-driver",
        schema_prefix: "cua_driver",
        display_name: "CUA Driver",
        command: McpCommand {
            program: "cua-driver",
            args: &["mcp"],
            env: &[],
        },
        shutdown_command: Some(McpCommand {
            program: "cua-driver",
            args: &["stop"],
            env: &[],
        }),
        setup: None,
    }
}
