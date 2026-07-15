//! Blender MCP provider definition.

use super::McpProviderDefinition;
use crate::mcp::{McpCommand, McpEnv, McpEnvValue};

/// Returns the code-approved Blender MCP provider definition.
pub(super) fn definition() -> McpProviderDefinition {
    McpProviderDefinition {
        provider_id: "blender-mcp",
        schema_prefix: "blender_mcp",
        display_name: "Blender MCP",
        command: McpCommand {
            program: "uvx",
            args: &["blender-mcp"],
            env: &[
                McpEnv {
                    key: "DISABLE_TELEMETRY",
                    value: McpEnvValue::Literal("true"),
                },
                McpEnv {
                    key: "BLENDER_HOST",
                    value: McpEnvValue::Literal("localhost"),
                },
                McpEnv {
                    key: "BLENDER_PORT",
                    value: McpEnvValue::Literal("9876"),
                },
            ],
        },
        shutdown_command: None,
        setup: None,
    }
}
