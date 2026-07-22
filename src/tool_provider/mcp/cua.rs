//! CUA Driver MCP provider definition.

use super::McpProviderDefinition;
use crate::mcp::McpCommand;
use crate::tool_provider::{
    ProviderDependency, ProviderManifest, ProviderPermission, ProviderPlatform,
};

/// Returns the code-approved CUA Driver MCP provider definition.
pub(super) fn definition() -> McpProviderDefinition {
    let command = McpCommand {
        program: "cua-driver",
        args: &["mcp"],
        env: &[],
    };

    McpProviderDefinition {
        manifest: ProviderManifest::mcp_stdio(
            "cua-driver",
            "CUA Driver",
            "Operate the local computer through the CUA Driver MCP server.",
            command.program,
            command.args,
            ProviderPlatform::desktop(),
            vec![ProviderDependency::executable(
                "cua-driver",
                "CUA Driver local runtime",
            )],
            Vec::new(),
            vec![
                ProviderPermission::ExternalProcess,
                ProviderPermission::ComputerControl,
            ],
        ),
        provider_id: "cua-driver",
        schema_prefix: "cua_driver",
        display_name: "CUA Driver",
        command,
        shutdown_command: Some(McpCommand {
            program: "cua-driver",
            args: &["stop"],
            env: &[],
        }),
        setup: None,
    }
}
