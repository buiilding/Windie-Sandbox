//! Basic Memory MCP provider definition and local project setup.
//!
//! Windie uses Basic Memory's normal user-wide configuration, but gives each
//! Windie home a dedicated project rooted at that home's `memory` directory.
//! The project constraint is the provider boundary: Basic Memory can remain
//! globally configured and useful to other clients without letting Windie
//! access their other memory projects.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, anyhow};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::McpProviderDefinition;
use super::provider::McpProviderSetup;
use crate::local;
use crate::mcp::{McpArgument, McpCommand, McpTransport};
use crate::mcp::{
    ProviderAuthentication, ProviderCleanup, ProviderDependency, ProviderManifest,
    ProviderPackageManager, ProviderPermission, ProviderPlatform, ProviderRuntime, ProviderScope,
};

const BASIC_MEMORY_PROJECT_NAME: &str = "windie-memory";
const BASIC_MEMORY_PROJECT_ENV: &str = "BASIC_MEMORY_MCP_PROJECT";
const BASIC_MEMORY_MEMORY_RELATIVE: &str = "memory";
const BASIC_MEMORY_UV_CACHE_RELATIVE: &str = "mcp/basic-memory/uv-cache";
// Basic Memory 0.22.1 allows LiteLLM versions below 2.0, so uv can select
// LiteLLM 1.95.0, which can require a local Rust/maturin build on macOS.
// Keep the provider on the last known broadly installable dependency range
// until Basic Memory publishes its corrected dependency metadata.
const BASIC_MEMORY_LITELLM_CONSTRAINT: &str = "litellm<1.92";
const BASIC_MEMORY_PACKAGE_ENV: &[crate::mcp::McpEnv] = &[crate::mcp::McpEnv {
    key: "UV_CACHE_DIR",
    value: crate::mcp::McpEnvValue::WindieDataDir(BASIC_MEMORY_UV_CACHE_RELATIVE),
}];
const BASIC_MEMORY_MCP_ENV: &[crate::mcp::McpEnv] = &[
    crate::mcp::McpEnv {
        key: "UV_CACHE_DIR",
        value: crate::mcp::McpEnvValue::WindieDataDir(BASIC_MEMORY_UV_CACHE_RELATIVE),
    },
    crate::mcp::McpEnv {
        key: BASIC_MEMORY_PROJECT_ENV,
        value: crate::mcp::McpEnvValue::UserEnv(BASIC_MEMORY_PROJECT_ENV),
    },
];

/// Returns the code-approved Basic Memory MCP provider definition.
pub(super) fn definition() -> McpProviderDefinition {
    let command = McpCommand {
        program: "uvx",
        args: &[
            McpArgument::Literal("--with"),
            McpArgument::Literal(BASIC_MEMORY_LITELLM_CONSTRAINT),
            McpArgument::Literal("basic-memory"),
            McpArgument::Literal("mcp"),
        ],
        env: BASIC_MEMORY_MCP_ENV,
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
        .with_author("Basic Machines")
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
        )
        .with_readme(include_str!("readmes/basic-memory.md")),
        provider_id: "basic-memory",
        schema_prefix: "basic_memory",
        display_name: "Basic Memory",
        transport: McpTransport::stdio(command),
        package_command: Some(McpCommand {
            program: "uvx",
            args: &[
                McpArgument::Literal("--with"),
                McpArgument::Literal(BASIC_MEMORY_LITELLM_CONSTRAINT),
                McpArgument::Literal("--from"),
                McpArgument::Literal("basic-memory"),
                McpArgument::Literal("python"),
                McpArgument::Literal("-c"),
                McpArgument::Literal("pass"),
            ],
            env: BASIC_MEMORY_PACKAGE_ENV,
        }),
        readiness_probe: None,
        setup: Some(McpProviderSetup::BasicMemoryProject),
        cleanup: ProviderCleanup::BasicMemory,
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

    let project_name = project_name()?;
    local::set_env_values(&[(BASIC_MEMORY_PROJECT_ENV.to_string(), project_name.clone())])?;

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
            command_error(&projects.stdout, &projects.stderr)
        ));
    }

    let project_list: Value = serde_json::from_slice(&projects.stdout)
        .context("failed to decode Basic Memory project list")?;
    if let Some(configured_path) = project_path(&project_list, &project_name) {
        let expected_path = canonical_path(&memory_dir)?;
        let actual_path = canonical_path(Path::new(configured_path))?;
        if actual_path != expected_path {
            return Err(anyhow!(
                "Basic Memory project {project_name} already points to {}; expected {}",
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
            project_name.as_str(),
            memory_dir.to_string_lossy().as_ref(),
        ])
        .output()
        .context("failed to create Basic Memory Windie project")?;
    if !created.status.success() {
        return Err(anyhow!(
            "failed to create Basic Memory project {project_name}: {}",
            command_error(&created.stdout, &created.stderr)
        ));
    }

    Ok(())
}

/// Removes Windie's Basic Memory project registration and package cache.
///
/// The project registration is global to Basic Memory, so it must be removed
/// through Basic Memory's CLI. The `memory` directory itself is user data and
/// remains intact. A project pointing anywhere else is rejected rather than
/// allowing an uninstall to mutate another user's project.
pub(crate) fn uninstall() -> Result<()> {
    let project_name = project_name()?;
    let memory_dir = windie_data_dir().join(BASIC_MEMORY_MEMORY_RELATIVE);
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
        .context("failed to list Basic Memory projects during uninstall")?;
    if !projects.status.success() {
        return Err(anyhow!(
            "Basic Memory project listing failed during uninstall: {}",
            command_error(&projects.stdout, &projects.stderr)
        ));
    }

    let project_list: Value = serde_json::from_slice(&projects.stdout)
        .context("failed to decode Basic Memory project list during uninstall")?;
    if let Some(configured_path) = project_path(&project_list, &project_name) {
        let expected_path = canonical_path(&memory_dir)?;
        let actual_path = canonical_path(Path::new(configured_path))?;
        if actual_path != expected_path {
            return Err(anyhow!(
                "refusing to remove Basic Memory project {project_name}: it points to {} instead of {}",
                actual_path.display(),
                expected_path.display()
            ));
        }

        let default_replacement = if project_is_default(&project_list, &project_name) {
            Some(replacement_project_name(&project_list, &project_name).ok_or_else(|| {
                anyhow::anyhow!(
                    "cannot remove Basic Memory project {project_name}: it is the default project and no other local project is available to become default"
                )
            })?)
        } else {
            None
        };

        if let Some(replacement) = &default_replacement {
            set_default_project(&uvx, replacement, &windie_data_dir())?;
        }

        let mut remove_command = Command::new(&uvx);
        if let Some(path) = local::path_with_command_parent(&uvx) {
            remove_command.env("PATH", path);
        }
        remove_command.env(
            "UV_CACHE_DIR",
            windie_data_dir().join(BASIC_MEMORY_UV_CACHE_RELATIVE),
        );
        let removed = remove_command
            .args([
                "basic-memory",
                "project",
                "remove",
                project_name.as_str(),
                "--local",
            ])
            .output()
            .context("failed to remove Basic Memory Windie project")?;
        if !removed.status.success() {
            if default_replacement.is_some() {
                let _ = set_default_project(&uvx, &project_name, &windie_data_dir());
            }
            return Err(anyhow!(
                "failed to remove Basic Memory project {project_name}: {}",
                command_error(&removed.stdout, &removed.stderr)
            ));
        }
    }

    local::remove_windie_directories(&["mcp/basic-memory"])?;
    local::unset_env_values(&[BASIC_MEMORY_PROJECT_ENV.to_string()])?;
    Ok(())
}

/// Returns the global Basic Memory project name assigned to this Windie home.
///
/// The normal user installation keeps the historical stable name so existing
/// users keep their project. Isolated homes, such as local release tests, get
/// deterministic names so they can coexist in Basic Memory's global registry.
fn project_name() -> Result<String> {
    let current_home = canonical_path(&windie_data_dir())?;
    let default_home = local::user_home_dir()?.join(".windie");
    let default_home = fs::canonicalize(&default_home).unwrap_or(default_home);

    Ok(project_name_for_paths(&current_home, &default_home))
}

/// Builds a stable project name without exposing the full local path.
fn project_name_for_paths(current_home: &Path, default_home: &Path) -> String {
    if paths_equal(current_home, default_home) {
        return BASIC_MEMORY_PROJECT_NAME.to_string();
    }

    let digest = Sha256::digest(normalized_path_text(current_home).as_bytes());
    let suffix = format!("{digest:x}");
    format!("{BASIC_MEMORY_PROJECT_NAME}-{}", &suffix[..12])
}

/// Compares paths using Windows' case-insensitive filesystem semantics.
fn paths_equal(left: &Path, right: &Path) -> bool {
    #[cfg(windows)]
    {
        left.to_string_lossy()
            .eq_ignore_ascii_case(&right.to_string_lossy())
    }
    #[cfg(not(windows))]
    {
        left == right
    }
}

/// Returns a stable path representation for project-name hashing.
fn normalized_path_text(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    if cfg!(windows) {
        text.to_ascii_lowercase()
    } else {
        text
    }
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

/// Returns whether Basic Memory marks one project as the CLI default.
fn project_is_default(project_list: &Value, project_name: &str) -> bool {
    let Some(projects) = project_list.get("projects") else {
        return false;
    };

    if let Some(projects) = projects.as_array() {
        return projects.iter().any(|project| {
            project.get("name").and_then(Value::as_str) == Some(project_name)
                && project
                    .get("is_default")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
        });
    }

    projects
        .as_object()
        .and_then(|projects| projects.get(project_name))
        .and_then(|project| project.get("is_default"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// Chooses another local project that can temporarily become the CLI default.
fn replacement_project_name(project_list: &Value, removed_project_name: &str) -> Option<String> {
    let projects = project_list.get("projects")?;

    if let Some(projects) = projects.as_array() {
        return projects.iter().find_map(|project| {
            let name = project.get("name")?.as_str()?;
            if name == removed_project_name {
                return None;
            }

            let local_path = project
                .get("local_path")
                .or_else(|| project.get("path"))
                .and_then(Value::as_str)
                .filter(|path| !path.trim().is_empty());
            local_path.map(|_| name.to_string())
        });
    }

    projects.as_object()?.iter().find_map(|(name, project)| {
        if name == removed_project_name {
            return None;
        }

        let local_path = project
            .get("local_path")
            .or_else(|| project.get("path"))
            .and_then(Value::as_str)
            .filter(|path| !path.trim().is_empty());
        local_path.map(|_| name.clone())
    })
}

/// Makes another local project the Basic Memory CLI default.
fn set_default_project(uvx: &Path, project_name: &str, windie_home: &Path) -> Result<()> {
    let mut command = Command::new(uvx);
    if let Some(path) = local::path_with_command_parent(uvx) {
        command.env("PATH", path);
    }
    command.env(
        "UV_CACHE_DIR",
        windie_home.join(BASIC_MEMORY_UV_CACHE_RELATIVE),
    );
    let output = command
        .args([
            "basic-memory",
            "project",
            "default",
            project_name,
            "--local",
        ])
        .output()
        .with_context(|| format!("failed to set Basic Memory default project to {project_name}"))?;
    if !output.status.success() {
        return Err(anyhow!(
            "failed to set Basic Memory default project to {project_name}: {}",
            command_error(&output.stdout, &output.stderr)
        ));
    }

    Ok(())
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

/// Returns a concise provider command diagnostic from stdout or stderr.
fn command_error(stdout: &[u8], stderr: &[u8]) -> String {
    let stdout = String::from_utf8_lossy(stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(stderr).trim().to_string();
    let message = match (stderr.is_empty(), stdout.is_empty()) {
        (false, false) => format!("{stderr}; {stdout}"),
        (false, true) => stderr,
        (true, false) => stdout,
        (true, true) => String::new(),
    };
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
    use super::{
        command_error, expand_home_path, project_is_default, project_name_for_paths, project_path,
        replacement_project_name,
    };
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

    #[test]
    fn keeps_the_default_project_name_for_the_normal_windie_home() {
        assert_eq!(
            project_name_for_paths(
                Path::new("C:/Users/test/.windie"),
                Path::new("C:/Users/test/.windie")
            ),
            "windie-memory"
        );
    }

    #[test]
    fn gives_isolated_homes_distinct_stable_project_names() {
        let first = project_name_for_paths(
            Path::new("C:/repo/target/local-installer/windows-x86_64/.windie"),
            Path::new("C:/Users/test/.windie"),
        );
        let second = project_name_for_paths(
            Path::new("C:/repo/target/local-installer/windows-x86_64/.windie"),
            Path::new("C:/Users/test/.windie"),
        );

        assert_eq!(first, second);
        assert!(first.starts_with("windie-memory-"));
    }

    #[test]
    fn reads_default_project_from_basic_memory_catalog() {
        let projects = json!({
            "projects": [
                {"name": "main", "local_path": "~/basic-memory", "is_default": false},
                {"name": "windie-memory", "local_path": "~/.windie/memory", "is_default": true}
            ]
        });

        assert!(project_is_default(&projects, "windie-memory"));
        assert!(!project_is_default(&projects, "main"));
    }

    #[test]
    fn chooses_another_local_project_as_default_replacement() {
        let projects = json!({
            "projects": [
                {"name": "cloud-only", "local_path": "", "is_default": false},
                {"name": "windie-memory", "local_path": "~/.windie/memory", "is_default": true},
                {"name": "main", "local_path": "~/basic-memory", "is_default": false}
            ]
        });

        assert_eq!(
            replacement_project_name(&projects, "windie-memory").as_deref(),
            Some("main")
        );
    }

    #[test]
    fn reports_stdout_when_provider_command_writes_no_stderr() {
        assert_eq!(
            command_error(b"Error removing project: cannot remove default", b""),
            "Error removing project: cannot remove default"
        );
    }
}
