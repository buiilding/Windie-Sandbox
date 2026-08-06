//! Bright Data MCP provider definition.

use super::McpProviderDefinition;
use crate::mcp::{McpCommand, McpEnv, McpEnvValue};
use crate::tool_provider::{
    ProviderAuthentication, ProviderDependency, ProviderManifest, ProviderPackageManager,
    ProviderPermission, ProviderPlatform, ProviderRuntime, ProviderScope, ProviderSecret,
};

const BRIGHTDATA_NPM_CACHE_RELATIVE: &str = "mcp/brightdata/npm-cache";
const BRIGHTDATA_ENV: &[McpEnv] = &[
    McpEnv {
        key: "API_TOKEN",
        value: McpEnvValue::UserEnv("BRIGHTDATA_API_TOKEN"),
    },
    McpEnv {
        key: "NPM_CONFIG_CACHE",
        value: McpEnvValue::WindieDataDir(BRIGHTDATA_NPM_CACHE_RELATIVE),
    },
];

/// Returns the code-approved Bright Data MCP provider definition.
pub(super) fn definition() -> McpProviderDefinition {
    let command = McpCommand {
        program: "npx",
        args: &["-y", "@brightdata/mcp"],
        env: BRIGHTDATA_ENV,
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
        .with_runtime(ProviderRuntime::Node)
        .with_author("Bright Data")
        .with_package(ProviderPackageManager::Npm, "@brightdata/mcp")
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
        package_command: Some(McpCommand {
            program: "npx",
            args: &["--yes", "--package", "@brightdata/mcp", "node", "-e", ""],
            env: BRIGHTDATA_ENV,
        }),
        shutdown_command: None,
        setup: None,
    }
}
