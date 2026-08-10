//! Chrome DevTools MCP provider definition.
//!
//! Windie launches Chrome DevTools MCP with either a dedicated persistent
//! profile or Chrome's explicit existing-browser approval flow. The managed
//! default is intentionally separate from the user's normal Chrome data.

use super::McpProviderDefinition;
use super::provider::McpProviderReadinessProbe;
use crate::mcp::{McpArgument, McpCommand, McpEnv, McpEnvValue, McpTransport};
use crate::tool_provider::{
    ProviderAuthentication, ProviderCleanup, ProviderDependency, ProviderManifest,
    ProviderPackageManager, ProviderPermission, ProviderPlatform, ProviderRuntime, ProviderScope,
};
use serde::{Deserialize, Serialize};

const CHROME_DEVTOOLS_PACKAGE: &str = "chrome-devtools-mcp@1.6.0";
const CHROME_DEVTOOLS_PROFILE_RELATIVE: &str = "mcp/chrome-devtools/profile";
const CHROME_DEVTOOLS_NPM_CACHE_RELATIVE: &str = "mcp/chrome-devtools/npm-cache";
const CHROME_DEVTOOLS_ENV: &[McpEnv] = &[
    McpEnv {
        key: "CHROME_DEVTOOLS_MCP_NO_UPDATE_CHECKS",
        value: McpEnvValue::Literal("true"),
    },
    McpEnv {
        key: "NPM_CONFIG_CACHE",
        value: McpEnvValue::WindieDataDir(CHROME_DEVTOOLS_NPM_CACHE_RELATIVE),
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Selects which Chrome browser owns a Chrome DevTools MCP session.
pub(crate) enum ChromeDevToolsConnectionMode {
    /// Start a separate persistent browser profile owned by Windie.
    Managed,
    /// Attach to the user's already-running Chrome through Chrome's local
    /// remote-debugging approval flow.
    Existing,
}

impl Default for ChromeDevToolsConnectionMode {
    fn default() -> Self {
        Self::Managed
    }
}

impl ChromeDevToolsConnectionMode {
    /// Returns the stable SQLite representation.
    pub(crate) fn as_storage(self) -> &'static str {
        match self {
            Self::Managed => "managed",
            Self::Existing => "existing",
        }
    }

    /// Decodes the stable SQLite representation.
    pub(crate) fn from_storage(value: &str) -> Option<Self> {
        match value {
            "managed" => Some(Self::Managed),
            "existing" => Some(Self::Existing),
            _ => None,
        }
    }
}

/// Returns the Chrome DevTools MCP command for one connection mode.
pub(super) fn command(mode: ChromeDevToolsConnectionMode) -> McpCommand {
    const MANAGED_ARGS: &[McpArgument] = &[
        McpArgument::Literal("-y"),
        McpArgument::Literal(CHROME_DEVTOOLS_PACKAGE),
        McpArgument::Literal("--user-data-dir"),
        McpArgument::WindieDataDir(CHROME_DEVTOOLS_PROFILE_RELATIVE),
        McpArgument::Literal("--no-usage-statistics"),
    ];
    const EXISTING_ARGS: &[McpArgument] = &[
        McpArgument::Literal("-y"),
        McpArgument::Literal(CHROME_DEVTOOLS_PACKAGE),
        McpArgument::Literal("--auto-connect"),
        McpArgument::Literal("--no-usage-statistics"),
    ];

    McpCommand {
        program: "npx",
        args: match mode {
            ChromeDevToolsConnectionMode::Managed => MANAGED_ARGS,
            ChromeDevToolsConnectionMode::Existing => EXISTING_ARGS,
        },
        env: CHROME_DEVTOOLS_ENV,
    }
}

/// Returns the code-approved Chrome DevTools MCP provider definition.
pub(super) fn definition() -> McpProviderDefinition {
    let command = command(ChromeDevToolsConnectionMode::Managed);

    McpProviderDefinition {
        manifest: ProviderManifest::mcp_stdio(
            "chrome-devtools",
            "Chrome DevTools",
            "Inspect, debug, and automate an explicitly selected Chrome browser session through Chrome DevTools MCP.",
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
                "Windie opens a separate Chrome profile by default, or connects to an explicitly approved existing Chrome.",
                "The selected browser session is reused according to the configured connection mode.",
                "Existing Chrome access requires Chrome's remote-debugging permission.",
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
        cleanup: ProviderCleanup::WindieDirectories(&[CHROME_DEVTOOLS_NPM_CACHE_RELATIVE]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn managed_and_existing_modes_use_distinct_launch_arguments() {
        let managed = command(ChromeDevToolsConnectionMode::Managed);
        let existing = command(ChromeDevToolsConnectionMode::Existing);

        assert!(
            managed
                .args
                .iter()
                .any(|argument| { matches!(argument, McpArgument::Literal("--user-data-dir")) })
        );
        assert!(
            !managed
                .args
                .iter()
                .any(|argument| { matches!(argument, McpArgument::Literal("--auto-connect")) })
        );
        assert!(
            existing
                .args
                .iter()
                .any(|argument| { matches!(argument, McpArgument::Literal("--auto-connect")) })
        );
        assert!(
            !existing
                .args
                .iter()
                .any(|argument| { matches!(argument, McpArgument::Literal("--user-data-dir")) })
        );
    }

    #[test]
    fn connection_modes_round_trip_through_storage_names() {
        assert_eq!(
            ChromeDevToolsConnectionMode::from_storage("managed"),
            Some(ChromeDevToolsConnectionMode::Managed)
        );
        assert_eq!(
            ChromeDevToolsConnectionMode::from_storage("existing"),
            Some(ChromeDevToolsConnectionMode::Existing)
        );
        assert_eq!(ChromeDevToolsConnectionMode::from_storage("other"), None);
    }

    #[test]
    fn readiness_probe_is_safe_and_read_only() {
        let provider = definition();

        assert!(matches!(
            provider.readiness_probe,
            Some(McpProviderReadinessProbe::Tool("list_pages"))
        ));
    }
}
