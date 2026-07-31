//! Basic Memory MCP provider definition and local project setup.
//!
//! Windie uses Basic Memory's normal user-wide configuration, but gives its
//! MCP process a dedicated `windie-memory` project rooted at `~/.windie/memory`.
//! The project argument is the provider boundary: Basic Memory can remain
//! globally installed and useful to other clients without letting Windie
//! access their other memory projects.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, anyhow};
use serde_json::Value;

use super::McpProviderDefinition;
use super::provider::McpProviderSetup;
use crate::local;
use crate::mcp::McpCommand;
use crate::tool_provider::{
    ProviderAuthentication, ProviderDependency, ProviderManifest, ProviderPackageManager,
    ProviderPermission, ProviderPlatform, ProviderRuntime, ProviderScope,
};

const BASIC_MEMORY_PROJECT_NAME: &str = "windie-memory";
const BASIC_MEMORY_MEMORY_RELATIVE: &str = "memory";
const BASIC_MEMORY_UV_CACHE_RELATIVE: &str = "mcp/basic-memory/uv-cache";
const BASIC_MEMORY_ENV: &[crate::mcp::McpEnv] = &[crate::mcp::McpEnv {
    key: "UV_CACHE_DIR",
    value: crate::mcp::McpEnvValue::WindieDataDir(BASIC_MEMORY_UV_CACHE_RELATIVE),
}];

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
        env: BASIC_MEMORY_ENV,
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
        .with_runtime(ProviderRuntime::Uv)
        .with_package(ProviderPackageManager::Uv, "basic-memory")
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
        package_command: Some(McpCommand {
            program: "uvx",
            args: &["--from", "basic-memory", "python", "-c", "pass"],
            env: BASIC_MEMORY_ENV,
        }),
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

    let uvx = local::resolve_command("uvx")?;
    let mut projects_command = Command::new(&uvx);
    if let Some(path) = local::path_with_command_parent(&uvx) {
        projects_command.env("PATH", path);
    }
    projects_command.env(
        "UV_CACHE_DIR",
        windie_data_dir().join(BASIC_MEMORY_UV_CACHE_RELATIVE),
    );
    let projects = projects_command
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

    let mut created_command = Command::new(&uvx);
    if let Some(path) = local::path_with_command_parent(&uvx) {
        created_command.env("PATH", path);
    }
    created_command.env(
        "UV_CACHE_DIR",
        windie_data_dir().join(BASIC_MEMORY_UV_CACHE_RELATIVE),
    );
    let created = created_command
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
                .then(|| {
                    project
                        .get("path")
                        .or_else(|| project.get("local_path"))?
                        .as_str()
                })
                .flatten()
        });
    }

    let project = projects.as_object()?.get(project_name)?;
    project
        .get("path")
        .or_else(|| project.get("local_path"))?
        .as_str()
}

/// Returns an absolute path when possible, preserving a useful error path.
fn canonical_path(path: &Path) -> Result<PathBuf> {
    let expanded = expand_home_path(path)?;
    fs::canonicalize(&expanded)
        .with_context(|| format!("failed to resolve path: {}", path.display()))
}

/// Expands a leading home-directory shorthand before filesystem access.
fn expand_home_path(path: &Path) -> Result<PathBuf> {
    let text = path.to_string_lossy();
    if text == "~" || text.starts_with("~/") || text.starts_with("~\\") {
        let suffix = text
            .strip_prefix('~')
            .expect("home path was checked for a leading tilde");
        return Ok(local::user_home_dir()?.join(suffix.trim_start_matches(['/', '\\'])));
    }

    Ok(path.to_path_buf())
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
    local::windie_home_dir().unwrap_or_else(|_| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use super::{expand_home_path, project_path};
    use crate::local;
    use serde_json::json;
    use std::path::Path;

    #[test]
    fn reads_basic_memory_local_path_project_field() {
        let projects = json!({
            "projects": [{
                "name": "windie-memory",
                "local_path": "C:/Users/test/.windie/memory"
            }]
        });

        assert_eq!(
            project_path(&projects, "windie-memory"),
            Some("C:/Users/test/.windie/memory")
        );
    }

    #[test]
    fn expands_home_shorthand_before_resolving_a_project_path() {
        let expanded = expand_home_path(Path::new("~/.windie/memory")).unwrap();

        assert_eq!(
            expanded,
            local::user_home_dir().unwrap().join(".windie/memory")
        );
    }
}
