//! Desktop Commander MCP provider definition and setup.
//!
//! Desktop Commander reads config from `$HOME/.claude-server-commander`, so
//! Windie starts the process with a provider-specific HOME and keeps this
//! config separate from any user-level Desktop Commander install.

use std::env;
use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};
use serde_json::json;

use super::provider::{McpProviderDefinition, McpProviderSetup};
use crate::mcp::{McpCommand, McpEnv, McpEnvValue};

const DESKTOP_COMMANDER_HOME_RELATIVE: &str = "mcp/desktop-commander";

/// Returns the code-approved Desktop Commander MCP provider definition.
pub(super) fn definition() -> McpProviderDefinition {
    McpProviderDefinition {
        provider_id: "desktop-commander",
        schema_prefix: "desktop_commander",
        display_name: "Desktop Commander",
        command: McpCommand {
            program: "npx",
            args: &["-y", "@wonderwhy-er/desktop-commander@latest"],
            env: &[McpEnv {
                key: "HOME",
                value: McpEnvValue::WindieDataDir(DESKTOP_COMMANDER_HOME_RELATIVE),
            }],
        },
        shutdown_command: None,
        setup: Some(McpProviderSetup::DesktopCommanderConfig),
    }
}

/// Writes Windie's isolated Desktop Commander config.
pub(super) fn prepare() -> Result<()> {
    let config_path = desktop_commander_home()
        .join(".claude-server-commander")
        .join("config.json");
    let config_dir = config_path
        .parent()
        .ok_or_else(|| anyhow!("Desktop Commander config path has no parent"))?;
    fs::create_dir_all(config_dir).with_context(|| {
        format!(
            "failed to create Desktop Commander config directory: {}",
            config_dir.display()
        )
    })?;

    let config = json!({
        "blockedCommands": blocked_commands(),
        "allowedDirectories": [],
        "telemetryEnabled": false,
        "fileWriteLineLimit": 50,
        "fileReadLineLimit": 1000,
        "pendingWelcomeOnboarding": false
    });
    fs::write(&config_path, serde_json::to_vec_pretty(&config)?).with_context(|| {
        format!(
            "failed to write Desktop Commander config: {}",
            config_path.display()
        )
    })
}

/// Returns the HOME directory Windie assigns to Desktop Commander.
fn desktop_commander_home() -> PathBuf {
    windie_data_dir().join(DESKTOP_COMMANDER_HOME_RELATIVE)
}

/// Returns Windie's per-user data directory.
fn windie_data_dir() -> PathBuf {
    env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".windie")
}

/// Keeps Desktop Commander's default high-risk shell command blocklist.
pub(super) fn blocked_commands() -> Vec<&'static str> {
    vec![
        "mkfs", "format", "mount", "umount", "fdisk", "dd", "parted", "diskpart", "sudo", "su",
        "passwd", "adduser", "useradd", "usermod", "groupadd", "chsh", "visudo", "shutdown",
        "reboot", "halt", "poweroff", "init", "iptables", "firewall", "netsh", "sfc", "bcdedit",
        "reg", "net", "sc", "runas", "cipher", "takeown",
    ]
}
