//! Blender MCP provider definition.

use anyhow::{Context, Result, anyhow};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

use super::McpProviderDefinition;
use super::provider::McpProviderSetup;
use crate::mcp::{McpCommand, McpEnv, McpEnvValue};
use crate::tool_provider::{
    ProviderAuthentication, ProviderDependency, ProviderManifest, ProviderPermission,
    ProviderPlatform, ProviderScope,
};

/// Returns the code-approved Blender MCP provider definition.
pub(super) fn definition() -> McpProviderDefinition {
    let command = McpCommand {
        program: "uvx",
        args: &["--with", "mcp<2", "blender-mcp"],
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
        setup: Some(McpProviderSetup::BlenderBridge),
    }
}

/// Confirms that Blender's local MCP bridge is accepting connections.
pub(super) fn prepare() -> Result<()> {
    let address = SocketAddr::from(([127, 0, 0, 1], 9876));
    TcpStream::connect_timeout(&address, Duration::from_millis(500))
        .map(|_| ())
        .with_context(|| {
            anyhow!(
                "Blender is not running with its MCP bridge on 127.0.0.1:9876; start Blender and enable the Blender MCP addon"
            )
        })
}
