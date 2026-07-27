//! Blender MCP provider definition.

use super::McpProviderDefinition;
use crate::mcp::{McpCommand, McpEnv, McpEnvValue};
use crate::tool_provider::{
    ProviderAuthentication, ProviderDependency, ProviderManifest, ProviderPermission,
    ProviderPlatform, ProviderScope,
};

/// Returns the code-approved Blender MCP provider definition.
pub(super) fn definition() -> McpProviderDefinition {
    let command = McpCommand {
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
    };

    McpProviderDefinition {
        manifest: ProviderManifest::mcp_stdio(
            "blender-mcp",
            "Blender MCP",
            "Inspect and control a local Blender instance through MCP.",
            command.program,
            command.args,
            ProviderPlatform::desktop(),
            vec![ProviderDependency::executable(
                "uvx",
                "uv package runner for Blender MCP",
            )],
            Vec::new(),
            vec![
                ProviderPermission::ExternalProcess,
                ProviderPermission::ComputerControl,
            ],
        )
        .with_metadata(
            ProviderScope::Local,
            ProviderAuthentication::None,
            "creative_tools",
            &["blender", "3d", "local"],
            None,
            &[
                "Install Blender MCP.",
                "Start Blender with its MCP bridge enabled.",
            ],
        ),
        provider_id: "blender-mcp",
        schema_prefix: "blender_mcp",
        display_name: "Blender MCP",
        command,
        shutdown_command: None,
        setup: None,
    }
}
