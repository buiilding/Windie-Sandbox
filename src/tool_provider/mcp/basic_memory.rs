//! Basic Memory MCP provider definition and local project setup.
//!
//! Windie uses Basic Memory's normal user-wide configuration, but gives its
//! MCP process a dedicated `windie-memory` project rooted at `~/.windie/memory`.
//! The project argument is the provider boundary: Basic Memory can remain
//! globally installed and useful to other clients without letting Windie
//! access their other memory projects.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, anyhow};
use serde_json::Value;

use super::McpProviderDefinition;
use super::provider::McpProviderSetup;
use crate::mcp::McpCommand;
use crate::tool_provider::{
    ProviderAuthentication, ProviderDependency, ProviderManifest, ProviderPermission,
    ProviderPlatform, ProviderScope,
};

const BASIC_MEMORY_PROJECT_NAME: &str = "windie-memory";
const BASIC_MEMORY_MEMORY_RELATIVE: &str = "memory";

/// Returns the code-approved Basic Memory MCP provider definition.
pub(super) fn definition() -> McpProviderDefinition {
    let command = McpCommand {
        program: "uvx",
        args: &[
            "basic-memory",
            "mcp",
            "--project",
            BASIC_MEMORY_PROJECT_NAME,
        ],
        env: &[],
    };

    McpProviderDefinition {
        manifest: ProviderManifest::mcp_stdio(
            "basic-memory",
            "Basic Memory",
            "Store and search Windie's local user memory through Basic Memory.",
            command.program,
            command.args,
            ProviderPlatform::desktop(),
            vec![ProviderDependency::executable(
                "uvx",
                "uv package runner for Basic Memory",
            )],
            Vec::new(),
            vec![
                ProviderPermission::ExternalProcess,
                ProviderPermission::Filesystem,
            ],
        )
        .with_metadata(
            ProviderScope::Local,
            ProviderAuthentication::None,
            "memory",
            &["memory", "notes", "local"],
            None,
            &[
                "Install Basic Memory.",
                "Create Windie's isolated memory project.",
            ],
        ),
        provider_id: "basic-memory",
        schema_prefix: "basic_memory",
        display_name: "Basic Memory",
        command,
        shutdown_command: None,
        setup: Some(McpProviderSetup::BasicMemoryProject),
    }
}

/// Ensures Basic Memory has the Windie-owned project before catalog discovery.
pub(super) fn prepare() -> Result<()> {
    let memory_dir = windie_data_dir().join(BASIC_MEMORY_MEMORY_RELATIVE);
    fs::create_dir_all(&memory_dir).with_context(|| {
        format!(
            "failed to create Basic Memory directory: {}",
            memory_dir.display()
        )
    })?;

    let projects = Command::new("uvx")
        .args(["basic-memory", "project", "list", "--json"])
        .output()
        .context("failed to list Basic Memory projects")?;
    if !projects.status.success() {
        return Err(anyhow!(
            "Basic Memory project listing failed: {}",
            command_error(&projects.stderr)
        ));
    }

    let project_list: Value = serde_json::from_slice(&projects.stdout)
        .context("failed to decode Basic Memory project list")?;
    if let Some(configured_path) = project_path(&project_list, BASIC_MEMORY_PROJECT_NAME) {
        let expected_path = canonical_path(&memory_dir)?;
        let actual_path = canonical_path(Path::new(configured_path))?;
        if actual_path != expected_path {
            return Err(anyhow!(
                "Basic Memory project {BASIC_MEMORY_PROJECT_NAME} already points to {}; expected {}",
                actual_path.display(),
                expected_path.display()
            ));
        }
        return Ok(());
    }

    let created = Command::new("uvx")
        .args([
            "basic-memory",
            "project",
            "add",
            BASIC_MEMORY_PROJECT_NAME,
            memory_dir.to_string_lossy().as_ref(),
            "--default",
        ])
        .output()
        .context("failed to create Basic Memory Windie project")?;
    if !created.status.success() {
        return Err(anyhow!(
            "failed to create Basic Memory project {BASIC_MEMORY_PROJECT_NAME}: {}",
            command_error(&created.stderr)
        ));
    }

    Ok(())
}

/// Returns the configured path for one project from Basic Memory JSON output.
fn project_path<'a>(project_list: &'a Value, project_name: &str) -> Option<&'a str> {
    let projects = project_list.get("projects")?;

    if let Some(projects) = projects.as_array() {
        return projects.iter().find_map(|project| {
            (project.get("name")?.as_str()? == project_name)
                .then(|| project.get("path")?.as_str())
                .flatten()
        });
    }

    projects
        .as_object()?
        .get(project_name)?
        .get("path")?
        .as_str()
}

/// Returns an absolute path when possible, preserving a useful error path.
fn canonical_path(path: &Path) -> Result<PathBuf> {
    fs::canonicalize(path).with_context(|| format!("failed to resolve path: {}", path.display()))
}

/// Returns a concise provider stderr message without exposing empty output.
fn command_error(stderr: &[u8]) -> String {
    let message = String::from_utf8_lossy(stderr).trim().to_string();
    if message.is_empty() {
        "no error details were returned".to_string()
    } else {
        message
    }
}

/// Returns Windie's per-user data directory.
fn windie_data_dir() -> PathBuf {
    env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".windie")
}
