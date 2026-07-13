//! User-local Windie setup, environment, and approved dependency installation.
//!
//! This module owns filesystem setup under `~/.windie`, edits Windie's explicit
//! provider-key environment file, and runs install/check commands for
//! code-approved runtime dependencies. It deliberately does not configure
//! arbitrary MCP servers or read project-local `.env` files.

use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, anyhow};
use uuid::Uuid;

const ENV_FILE_NAME: &str = ".env";
const API_TOKEN_FILE_NAME: &str = "api-token";
const BIFROST_DIR: &str = "bifrost";
const BENCHMARK_DIR: &str = "benchmarks";
const INSPECTOR_LOG_FILE_NAME: &str = "windie-inspector.log";
const CUA_DRIVER_INSTALL_URL: &str =
    "https://raw.githubusercontent.com/trycua/cua/main/libs/cua-driver/scripts/install.sh";

#[derive(Debug, Clone, PartialEq, Eq)]
/// Result of one approved installation request.
pub struct InstallReport {
    pub target: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Paths that make up Windie's user-local runtime layout.
pub struct WindieLayout {
    pub root: PathBuf,
    pub env_file: PathBuf,
    pub api_token_file: PathBuf,
    pub bifrost_dir: PathBuf,
    pub benchmarks_dir: PathBuf,
    pub inspector_log_file: PathBuf,
}

/// Creates Windie's required user-local directories and empty env file.
pub fn ensure_windie_layout() -> Result<WindieLayout> {
    let layout = windie_layout()?;

    fs::create_dir_all(&layout.root)
        .with_context(|| format!("failed to create {}", layout.root.display()))?;
    fs::create_dir_all(&layout.bifrost_dir)
        .with_context(|| format!("failed to create {}", layout.bifrost_dir.display()))?;
    fs::create_dir_all(&layout.benchmarks_dir)
        .with_context(|| format!("failed to create {}", layout.benchmarks_dir.display()))?;
    if !layout.env_file.exists() {
        fs::write(&layout.env_file, "")
            .with_context(|| format!("failed to create {}", layout.env_file.display()))?;
    }

    Ok(layout)
}

/// Returns the only supported Windie provider-key environment file path.
pub fn env_file_path() -> Result<PathBuf> {
    Ok(windie_layout()?.env_file)
}

/// Returns the stable localhost API token shared by `windie api` and UI clients.
pub fn ensure_api_token() -> Result<String> {
    let layout = ensure_windie_layout()?;
    if layout.api_token_file.exists() {
        let token = fs::read_to_string(&layout.api_token_file)
            .with_context(|| format!("failed to read {}", layout.api_token_file.display()))?
            .trim()
            .to_string();
        if !token.is_empty() {
            return Ok(token);
        }
    }

    let token = Uuid::new_v4().to_string();
    write_secret_file(&layout.api_token_file, &format!("{token}\n"))?;

    Ok(token)
}

/// Returns the log file used by the detached local inspector dev server.
pub fn inspector_log_file_path() -> Result<PathBuf> {
    Ok(ensure_windie_layout()?.inspector_log_file)
}

/// Lists keys currently present in Windie's provider-key environment file.
pub fn list_env_keys() -> Result<Vec<String>> {
    let layout = ensure_windie_layout()?;
    let text = fs::read_to_string(&layout.env_file)
        .with_context(|| format!("failed to read {}", layout.env_file.display()))?;
    let mut keys = text
        .lines()
        .filter_map(env_line_key)
        .map(str::to_string)
        .collect::<Vec<_>>();
    keys.sort();
    keys.dedup();

    Ok(keys)
}

/// Sets one or more provider-key environment values in `~/.windie/.env`.
pub fn set_env_values(assignments: &[(String, String)]) -> Result<PathBuf> {
    if assignments.is_empty() {
        return Err(anyhow!("at least one KEY=value assignment is required"));
    }
    for (key, _) in assignments {
        validate_env_key(key)?;
    }

    let layout = ensure_windie_layout()?;
    let text = fs::read_to_string(&layout.env_file).unwrap_or_default();
    let mut lines = text.lines().map(str::to_string).collect::<Vec<_>>();

    for (key, value) in assignments {
        set_env_line(&mut lines, key, value);
    }

    write_env_lines(&layout.env_file, &lines)?;

    Ok(layout.env_file)
}

/// Removes one or more provider-key environment values from `~/.windie/.env`.
pub fn unset_env_values(keys: &[String]) -> Result<PathBuf> {
    if keys.is_empty() {
        return Err(anyhow!("at least one environment key is required"));
    }
    for key in keys {
        validate_env_key(key)?;
    }

    let layout = ensure_windie_layout()?;
    let text = fs::read_to_string(&layout.env_file).unwrap_or_default();
    let lines = text
        .lines()
        .filter(|line| {
            let Some(key) = env_line_key(line) else {
                return true;
            };
            !keys.iter().any(|removed| removed == key)
        })
        .map(str::to_string)
        .collect::<Vec<_>>();

    write_env_lines(&layout.env_file, &lines)?;

    Ok(layout.env_file)
}

/// Installs or verifies one approved Windie runtime dependency.
pub fn install_target(target: &str) -> Result<InstallReport> {
    ensure_windie_layout()?;

    match target {
        "bifrost" => {
            require_command("npx")?;
            Ok(InstallReport {
                target: target.to_string(),
                message: "Bifrost will run through public npx package @maximhq/bifrost".to_string(),
            })
        }
        "cua-driver" => install_cua_driver(),
        "desktop-commander" => {
            require_command("npx")?;
            Ok(InstallReport {
                target: target.to_string(),
                message:
                    "Desktop Commander will run through public npx package @wonderwhy-er/desktop-commander@latest"
                        .to_string(),
            })
        }
        "blender-mcp" => {
            require_command("uvx")?;
            Ok(InstallReport {
                target: target.to_string(),
                message: "Blender MCP will run through public uvx package blender-mcp".to_string(),
            })
        }
        "brightdata" => {
            require_command("npx")?;
            Ok(InstallReport {
                target: target.to_string(),
                message: "Bright Data MCP will run through public npx package @brightdata/mcp"
                    .to_string(),
            })
        }
        _ => Err(anyhow!("unknown install target: {target}")),
    }
}

/// Returns the current user-local Windie layout without creating directories.
fn windie_layout() -> Result<WindieLayout> {
    let Some(home) = env::var_os("HOME") else {
        return Err(anyhow!("HOME is not set"));
    };
    let root = PathBuf::from(home).join(".windie");

    Ok(WindieLayout {
        env_file: root.join(ENV_FILE_NAME),
        api_token_file: root.join(API_TOKEN_FILE_NAME),
        bifrost_dir: root.join(BIFROST_DIR),
        benchmarks_dir: root.join(BENCHMARK_DIR),
        inspector_log_file: root.join(INSPECTOR_LOG_FILE_NAME),
        root,
    })
}

/// Installs CUA Driver using its public upstream installer when needed.
fn install_cua_driver() -> Result<InstallReport> {
    if command_exists("cua-driver") {
        return Ok(InstallReport {
            target: "cua-driver".to_string(),
            message: "cua-driver is already available on PATH".to_string(),
        });
    }

    require_command("curl")?;
    require_command("bash")?;

    let status = Command::new("bash")
        .arg("-c")
        .arg(format!("curl -fsSL {CUA_DRIVER_INSTALL_URL} | bash"))
        .status()
        .context("failed to start cua-driver installer")?;
    if !status.success() {
        return Err(anyhow!("cua-driver installer failed"));
    }

    Ok(InstallReport {
        target: "cua-driver".to_string(),
        message: "installed cua-driver with the public trycua installer".to_string(),
    })
}

/// Requires one executable to be available on PATH.
fn require_command(program: &str) -> Result<()> {
    if command_exists(program) {
        return Ok(());
    }

    Err(anyhow!(
        "required command is not available on PATH: {program}"
    ))
}

/// Returns whether one executable is available on PATH.
fn command_exists(program: &str) -> bool {
    let Some(paths) = env::var_os("PATH") else {
        return false;
    };

    env::split_paths(&paths).any(|path| path.join(program).is_file())
}

/// Validates a `.env` key that Windie is allowed to write.
fn validate_env_key(key: &str) -> Result<()> {
    if key.is_empty() {
        return Err(anyhow!("environment key cannot be empty"));
    }
    if key
        .bytes()
        .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
    {
        return Ok(());
    }

    Err(anyhow!(
        "environment key must contain only uppercase letters, digits, and underscores: {key}"
    ))
}

/// Returns the key assigned by one `.env` line, if the line assigns a value.
fn env_line_key(line: &str) -> Option<&str> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let line = line.strip_prefix("export ").unwrap_or(line).trim();
    let (key, _) = line.split_once('=')?;
    let key = key.trim();
    if key.is_empty() { None } else { Some(key) }
}

/// Inserts or replaces one key assignment in an in-memory env file.
fn set_env_line(lines: &mut Vec<String>, key: &str, value: &str) {
    let replacement = format!("{key}={value}");
    for line in lines.iter_mut() {
        if env_line_key(line).is_some_and(|line_key| line_key == key) {
            *line = replacement;
            return;
        }
    }

    lines.push(replacement);
}

/// Writes env file lines with a stable trailing newline.
fn write_env_lines(path: &Path, lines: &[String]) -> Result<()> {
    let text = if lines.is_empty() {
        String::new()
    } else {
        format!("{}\n", lines.join("\n"))
    };
    fs::write(path, text).with_context(|| format!("failed to write {}", path.display()))
}

/// Writes a user-local secret file without inheriting permissive default modes.
fn write_secret_file(path: &Path, text: &str) -> Result<()> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }

    let mut file = options
        .open(path)
        .with_context(|| format!("failed to write {}", path.display()))?;
    file.write_all(text.as_bytes())
        .with_context(|| format!("failed to write {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_line_key_reads_plain_and_export_assignments() {
        assert_eq!(env_line_key("OPENAI_API_KEY=value"), Some("OPENAI_API_KEY"));
        assert_eq!(
            env_line_key("export OPENROUTER_API_KEY=value"),
            Some("OPENROUTER_API_KEY")
        );
        assert_eq!(env_line_key("# OPENAI_API_KEY=value"), None);
    }

    #[test]
    fn set_env_line_replaces_existing_key() {
        let mut lines = vec![
            "OPENAI_API_KEY=old".to_string(),
            "OPENROUTER_API_KEY=keep".to_string(),
        ];

        set_env_line(&mut lines, "OPENAI_API_KEY", "new");

        assert_eq!(
            lines,
            vec![
                "OPENAI_API_KEY=new".to_string(),
                "OPENROUTER_API_KEY=keep".to_string()
            ]
        );
    }

    #[test]
    fn rejects_lowercase_env_key() {
        let error = validate_env_key("openai_api_key").unwrap_err();

        assert!(error.to_string().contains("uppercase"));
    }
}
