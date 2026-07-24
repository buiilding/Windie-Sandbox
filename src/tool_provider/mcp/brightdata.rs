//! Bright Data MCP provider definition.

use super::McpProviderDefinition;
use crate::mcp::{McpCommand, McpEnv, McpEnvValue};
use crate::tool_provider::{
    ProviderAuthentication, ProviderDependency, ProviderManifest, ProviderPermission,
    ProviderPlatform, ProviderScope, ProviderSecret,
};

/// Returns the code-approved Bright Data MCP provider definition.
pub(super) fn definition() -> McpProviderDefinition {
    let command = McpCommand {
        program: "npx",
        args: &["-y", "@brightdata/mcp"],
        env: &[McpEnv {
            key: "API_TOKEN",
            value: McpEnvValue::UserEnv("BRIGHTDATA_API_TOKEN"),
        }],
    };

    McpProviderDefinition {
        manifest: ProviderManifest::mcp_stdio(
            "brightdata",
            "Bright Data",
            "Search and access live web data through Bright Data MCP.",
            command.program,
            command.args,
            ProviderPlatform::desktop(),
            vec![ProviderDependency::executable(
                "npx",
                "Node.js package runner for Bright Data MCP",
            )],
            vec![ProviderSecret::required(
                "BRIGHTDATA_API_TOKEN",
                "Bright Data API token",
            )],
            vec![
                ProviderPermission::ExternalProcess,
                ProviderPermission::Network,
            ],
        )
        .with_metadata(
            ProviderScope::Cloud,
            ProviderAuthentication::ApiKey,
            "web_data",
            &["web", "search", "cloud"],
            Some("https://brightdata.com/"),
            &[
                "Create a Bright Data API token.",
                "Enter the token when prompted.",
            ],
        ),
        provider_id: "brightdata",
        schema_prefix: "brightdata",
        display_name: "Bright Data",
        command,
        shutdown_command: None,
        setup: None,
    }
}
