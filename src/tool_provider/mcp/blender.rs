//! Blender MCP provider definition.

use super::McpProviderDefinition;
use crate::mcp::{McpArgument, McpCommand, McpEnv, McpEnvValue};
use crate::tool_provider::{
    ProviderAuthentication, ProviderDependency, ProviderManifest, ProviderPackageManager,
    ProviderPermission, ProviderPlatform, ProviderRuntime, ProviderScope,
};

const BLENDER_UV_CACHE_RELATIVE: &str = "mcp/blender-mcp/uv-cache";
const BLENDER_ENV: &[McpEnv] = &[
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
    McpEnv {
        key: "UV_CACHE_DIR",
        value: McpEnvValue::WindieDataDir(BLENDER_UV_CACHE_RELATIVE),
    },
];

/// Returns the code-approved Blender MCP provider definition.
pub(super) fn definition() -> McpProviderDefinition {
    let command = McpCommand {
        program: "uvx",
        args: &[
            McpArgument::Literal("--with"),
            McpArgument::Literal("mcp<2"),
            McpArgument::Literal("blender-mcp"),
        ],
        env: BLENDER_ENV,
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
        .with_runtime(ProviderRuntime::Uv)
        .with_author("ahujasid")
        .with_package(ProviderPackageManager::Uv, "blender-mcp")
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
        package_command: Some(McpCommand {
            program: "uvx",
            args: &[
                McpArgument::Literal("--with"),
                McpArgument::Literal("mcp<2"),
                McpArgument::Literal("--from"),
                McpArgument::Literal("blender-mcp"),
                McpArgument::Literal("python"),
                McpArgument::Literal("-c"),
                McpArgument::Literal("pass"),
            ],
            env: BLENDER_ENV,
        }),
        shutdown_command: None,
        readiness_probe: None,
        setup: None,
    }
}
