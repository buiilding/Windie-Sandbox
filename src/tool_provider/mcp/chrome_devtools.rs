//! Chrome DevTools MCP provider definition.
//!
//! Windie launches Chrome DevTools MCP with a dedicated persistent Chrome
//! profile. The profile is intentionally separate from the user's normal
//! Chrome data so browser automation cannot silently take over existing tabs,
//! cookies, or authenticated sessions.

use super::McpProviderDefinition;
use super::provider::McpProviderReadinessProbe;
use crate::mcp::{McpArgument, McpCommand, McpEnv, McpEnvValue, McpTransport};
use crate::tool_provider::{
    ProviderAuthentication, ProviderDependency, ProviderManifest, ProviderPackageManager,
    ProviderPermission, ProviderPlatform, ProviderRuntime, ProviderScope,
};

const CHROME_DEVTOOLS_PACKAGE: &str = "chrome-devtools-mcp@1.6.0";
const CHROME_DEVTOOLS_PROFILE_RELATIVE: &str = "mcp/chrome-devtools/profile";
const CHROME_DEVTOOLS_ENV: &[McpEnv] = &[McpEnv {
    key: "CHROME_DEVTOOLS_MCP_NO_UPDATE_CHECKS",
    value: McpEnvValue::Literal("true"),
}];

/// Returns the code-approved Chrome DevTools MCP provider definition.
pub(super) fn definition() -> McpProviderDefinition {
    let command = McpCommand {
        program: "npx",
        args: &[
            McpArgument::Literal("-y"),
            McpArgument::Literal(CHROME_DEVTOOLS_PACKAGE),
            McpArgument::Literal("--user-data-dir"),
            McpArgument::WindieDataDir(CHROME_DEVTOOLS_PROFILE_RELATIVE),
            McpArgument::Literal("--no-usage-statistics"),
        ],
        env: CHROME_DEVTOOLS_ENV,
    };

    McpProviderDefinition {
        manifest: ProviderManifest::mcp_stdio(
            "chrome-devtools",
            "Chrome DevTools",
            "Inspect, debug, and automate a separate persistent Chrome browser session through Chrome DevTools MCP.",
            command.program,
            command.args,
            ProviderPlatform::desktop(),
            vec![ProviderDependency::executable(
                "npx",
                "Node.js package runner for Chrome DevTools MCP",
            )],
            Vec::new(),
            vec![
                ProviderPermission::ExternalProcess,
                ProviderPermission::ComputerControl,
                ProviderPermission::Network,
            ],
        )
        .with_runtime(ProviderRuntime::Node)
        .with_author("Chrome DevTools team")
        .with_package(ProviderPackageManager::Npm, CHROME_DEVTOOLS_PACKAGE)
        .with_metadata(
            ProviderScope::Local,
            ProviderAuthentication::None,
            "browser_automation",
            &["browser", "chrome", "devtools", "debugging"],
            Some("https://github.com/ChromeDevTools/chrome-devtools-mcp"),
            &[
                "Windie opens a separate Chrome profile for this provider.",
                "Log into websites in that profile once; Windie reuses the session later.",
                "The normal Chrome profile and its open tabs are not used.",
            ],
        )
        .with_readme(include_str!("readmes/chrome-devtools.md")),
        provider_id: "chrome-devtools",
        schema_prefix: "chrome_devtools",
        display_name: "Chrome DevTools",
        transport: McpTransport::stdio(command),
        package_command: Some(McpCommand {
            program: "npx",
            args: &[
                McpArgument::Literal("--yes"),
                McpArgument::Literal("--package"),
                McpArgument::Literal(CHROME_DEVTOOLS_PACKAGE),
                McpArgument::Literal("node"),
                McpArgument::Literal("-e"),
                McpArgument::Literal(""),
            ],
            env: CHROME_DEVTOOLS_ENV,
        }),
        readiness_probe: Some(McpProviderReadinessProbe::Tool("list_pages")),
        setup: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readiness_probe_is_safe_and_read_only() {
        let provider = definition();

        assert!(matches!(
            provider.readiness_probe,
            Some(McpProviderReadinessProbe::Tool("list_pages"))
        ));
    }
}
