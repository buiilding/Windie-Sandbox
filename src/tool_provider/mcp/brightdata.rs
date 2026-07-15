//! Bright Data MCP provider definition.

use super::McpProviderDefinition;
use crate::mcp::{McpCommand, McpEnv, McpEnvValue};

/// Returns the code-approved Bright Data MCP provider definition.
pub(super) fn definition() -> McpProviderDefinition {
    McpProviderDefinition {
        provider_id: "brightdata",
        schema_prefix: "brightdata",
        display_name: "Bright Data",
        command: McpCommand {
            program: "npx",
            args: &["-y", "@brightdata/mcp"],
            env: &[McpEnv {
                key: "API_TOKEN",
                value: McpEnvValue::UserEnv("BRIGHTDATA_API_TOKEN"),
            }],
        },
        shutdown_command: None,
        setup: None,
    }
}
