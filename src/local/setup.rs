//! User-local Windie setup, environment, and approved dependency installation.
//!
//! This module owns filesystem setup under `~/.windie`, edits Windie's explicit
//! provider-key environment file, and runs install/check commands for
//! code-approved runtime dependencies. It deliberately does not configure
//! arbitrary MCP servers or read project-local `.env` files.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, anyhow};

use super::runtime;

const ENV_FILE_NAME: &str = ".env";
const BIFROST_DIR: &str = "bifrost";
const GATEWAY_LOG_FILE_NAME: &str = "windie-gateway.log";
const GATEWAY_PID_FILE_NAME: &str = "bifrost.pid";
const API_LOG_FILE_NAME: &str = "windie-api.log";
const API_PID_FILE_NAME: &str = "windie-api.pid";
const INSPECTOR_LOG_FILE_NAME: &str = "windie-inspector.log";
const INSPECTOR_PID_FILE_NAME: &str = "windie-inspector.pid";
const TRAY_LOG_FILE_NAME: &str = "windie-tray.log";
const TRAY_PID_FILE_NAME: &str = "windie-tray.pid";
const LLM_ENV_KEYS: &[&str] = &[
    "OPENAI_API_KEY",
    "OPENROUTER_API_KEY",
    "ANTHROPIC_API_KEY",
    "GEMINI_API_KEY",
    "MISTRAL_API_KEY",
    "COHERE_API_KEY",
    "GROQ_API_KEY",
    "DEEPSEEK_API_KEY",
    "CEREBRAS_API_KEY",
    "PERPLEXITY_API_KEY",
    "XAI_API_KEY",
    "FIREWORKS_API_KEY",
    "TOGETHERAI_API_KEY",
    "AZURE_API_KEY",
    "AZURE_OPENAI_API_KEY",
    "AWS_ACCESS_KEY_ID",
    "AWS_SECRET_ACCESS_KEY",
    "GOOGLE_APPLICATION_CREDENTIALS",
];
const CUA_DRIVER_INSTALL_URL: &str =
    "https://raw.githubusercontent.com/trycua/cua/main/libs/cua-driver/scripts/install.sh";

/// Returns the current user's home directory across supported operating
/// systems. Unix environments conventionally expose `HOME`; native Windows
/// exposes `USERPROFILE` instead.
pub fn user_home_dir() -> Result<PathBuf> {
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .ok_or_else(|| anyhow!("HOME or USERPROFILE is not set"))
}

/// Returns Windie's user-local data directory.
///
/// `WINDIE_HOME` is an explicit escape hatch for isolated installations and
/// tests. Without it, Windie stores state under the platform user's home.
pub fn windie_home_dir() -> Result<PathBuf> {
    if let Some(path) = env::var_os("WINDIE_HOME")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
    {
        return Ok(path);
    }

    Ok(user_home_dir()?.join(".windie"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Result of one approved installation request.
pub struct InstallReport {
    pub target: String,
    pub message: String,
    pub status: InstallStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Describes whether an approved dependency was reused or installed.
pub enum InstallStatus {
    Detected,
    Installed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Paths that make up Windie's user-local runtime layout.
pub struct WindieLayout {
    pub root: PathBuf,
    pub env_file: PathBuf,
    pub bifrost_dir: PathBuf,
    pub gateway_log_file: PathBuf,
    pub gateway_pid_file: PathBuf,
    pub api_log_file: PathBuf,
    pub api_pid_file: PathBuf,
    pub inspector_log_file: PathBuf,
    pub inspector_pid_file: PathBuf,
    pub tray_log_file: PathBuf,
    pub tray_pid_file: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Exact paths that the uninstall operation is allowed to remove.
pub struct UninstallPlan {
    pub windie_home: PathBuf,
    pub install_dir: PathBuf,
    pub binaries: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Filesystem result from one Windie uninstall attempt.
pub struct UninstallCleanup {
    pub removed_data: bool,
    pub removed_binaries: Vec<PathBuf>,
    pub deferred_binaries: Vec<PathBuf>,
    pub cleanup_scheduled: bool,
}

/// Creates Windie's required user-local directories and empty env file.
pub fn ensure_windie_layout() -> Result<WindieLayout> {
    let layout = windie_layout()?;

    fs::create_dir_all(&layout.root)
        .with_context(|| format!("failed to create {}", layout.root.display()))?;
    fs::create_dir_all(&layout.bifrost_dir)
        .with_context(|| format!("failed to create {}", layout.bifrost_dir.display()))?;
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

/// Returns the persistent log file for one managed component.
pub fn component_log_file_path(component: crate::process::ManagedComponent) -> Result<PathBuf> {
    let layout = ensure_windie_layout()?;
    Ok(match component {
        crate::process::ManagedComponent::Gateway => layout.gateway_log_file,
        crate::process::ManagedComponent::Api => layout.api_log_file,
        crate::process::ManagedComponent::Inspector => layout.inspector_log_file,
        crate::process::ManagedComponent::Tray => layout.tray_log_file,
    })
}

/// Returns the persistent PID file for one managed component.
pub fn component_pid_file_path(component: crate::process::ManagedComponent) -> Result<PathBuf> {
    let layout = ensure_windie_layout()?;
    Ok(match component {
        crate::process::ManagedComponent::Gateway => layout.gateway_pid_file,
        crate::process::ManagedComponent::Api => layout.api_pid_file,
        crate::process::ManagedComponent::Inspector => layout.inspector_pid_file,
        crate::process::ManagedComponent::Tray => layout.tray_pid_file,
    })
}

/// Returns the exact Windie-owned paths that uninstall may remove.
///
/// The data root is the only recursive target. Installed binaries are always
/// individual files inside the configured install directory; the directory
/// itself is never removed because it may contain unrelated user programs.
pub fn uninstall_plan() -> Result<UninstallPlan> {
    let user_home = absolute_path(&user_home_dir()?)?;
    let windie_home = absolute_path(&windie_home_dir()?)?;
    let install_dir = absolute_path(&windie_install_dir(&user_home)?)?;
    validate_uninstall_paths(&user_home, &windie_home, &install_dir)?;

    Ok(UninstallPlan {
        windie_home,
        install_dir: install_dir.clone(),
        binaries: ["windie", "bifrost", "windie-inspector"]
            .into_iter()
            .map(|name| install_dir.join(executable_name(name)))
            .collect(),
    })
}

/// Removes the exact paths in an uninstall plan.
///
/// On Windows, a running `windie.exe` cannot remove itself. When the current
/// executable is one of the planned binaries, this function schedules a
/// short-lived PowerShell cleanup process and returns before that process
/// removes the files after Windie exits.
pub fn remove_uninstall_plan(plan: &UninstallPlan) -> Result<UninstallCleanup> {
    validate_uninstall_plan(plan, &absolute_path(&user_home_dir()?)?)?;

    let current_executable = absolute_path(&std::env::current_exe()?).ok();
    let deferred_binaries = if cfg!(windows)
        && current_executable
            .as_ref()
            .is_some_and(|path| plan.binaries.iter().any(|binary| binary == path))
    {
        plan.binaries.clone()
    } else {
        Vec::new()
    };

    let mut removed_binaries = Vec::new();
    for path in &plan.binaries {
        if deferred_binaries.contains(path) {
            continue;
        }
        if let Some(removed) = remove_owned_file(path)? {
            removed_binaries.push(removed);
        }
    }

    let cleanup_scheduled = !deferred_binaries.is_empty();
    if cleanup_scheduled {
        schedule_windows_cleanup(plan)?;
    }

    let removed_data = match fs::symlink_metadata(&plan.windie_home) {
        Ok(_) => {
            ensure_owned_directory(&plan.windie_home)?;
            if cleanup_scheduled {
                false
            } else {
                fs::remove_dir_all(&plan.windie_home).with_context(|| {
                    format!(
                        "failed to remove Windie data {}",
                        plan.windie_home.display()
                    )
                })?;
                true
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "failed to inspect Windie data {}",
                    plan.windie_home.display()
                )
            });
        }
    };

    Ok(UninstallCleanup {
        removed_data,
        removed_binaries,
        deferred_binaries,
        cleanup_scheduled,
    })
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

/// Reads one provider-key value from Windie's `~/.windie/.env` file.
pub fn env_value(key: &str) -> Result<Option<String>> {
    let path = env_file_path()?;
    if !path.exists() {
        return Ok(None);
    }

    let text =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;

    Ok(text.lines().find_map(|line| env_line_value(line, key)))
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
        "bifrost" => Ok(InstallReport {
            target: target.to_string(),
            message: "Bifrost is provided by the Windie-owned bundled binary".to_string(),
            status: InstallStatus::Detected,
        }),
        "cua-driver" => install_cua_driver(),
        "desktop-commander" => {
            let status = runtime::ensure_provider_runtime(target)?;
            Ok(InstallReport {
                target: target.to_string(),
                message: "Windie-managed Node.js runtime is ready for Desktop Commander"
                    .to_string(),
                status: if status {
                    InstallStatus::Installed
                } else {
                    InstallStatus::Detected
                },
            })
        }
        "blender-mcp" => {
            let status = runtime::ensure_provider_runtime(target)?;
            Ok(InstallReport {
                target: target.to_string(),
                message: "Windie-managed uv runtime is ready for Blender MCP".to_string(),
                status: if status {
                    InstallStatus::Installed
                } else {
                    InstallStatus::Detected
                },
            })
        }
        "brightdata" => {
            let status = runtime::ensure_provider_runtime(target)?;
            Ok(InstallReport {
                target: target.to_string(),
                message: "Windie-managed Node.js runtime is ready for Bright Data MCP".to_string(),
                status: if status {
                    InstallStatus::Installed
                } else {
                    InstallStatus::Detected
                },
            })
        }
        "basic-memory" => {
            let status = runtime::ensure_provider_runtime(target)?;
            Ok(InstallReport {
                target: target.to_string(),
                message: "Windie-managed uv runtime is ready for Basic Memory".to_string(),
                status: if status {
                    InstallStatus::Installed
                } else {
                    InstallStatus::Detected
                },
            })
        }
        _ => Err(anyhow!("unknown install target: {target}")),
    }
}

/// Returns the current user-local Windie layout without creating directories.
fn windie_layout() -> Result<WindieLayout> {
    let root = windie_home_dir()?;

    Ok(WindieLayout {
        env_file: root.join(ENV_FILE_NAME),
        bifrost_dir: root.join(BIFROST_DIR),
        gateway_log_file: root.join(BIFROST_DIR).join(GATEWAY_LOG_FILE_NAME),
        gateway_pid_file: root.join(BIFROST_DIR).join(GATEWAY_PID_FILE_NAME),
        api_log_file: root.join(API_LOG_FILE_NAME),
        api_pid_file: root.join(API_PID_FILE_NAME),
        inspector_log_file: root.join(INSPECTOR_LOG_FILE_NAME),
        inspector_pid_file: root.join(INSPECTOR_PID_FILE_NAME),
        tray_log_file: root.join(TRAY_LOG_FILE_NAME),
        tray_pid_file: root.join(TRAY_PID_FILE_NAME),
        root,
    })
}

/// Returns the user-local directory containing the Windie binaries.
///
/// A packaged executable knows its own install directory. Prefer that path so
/// uninstall works even when the installer used a custom directory and did not
/// persist `WINDIE_INSTALL_DIR` into future shells. The explicit environment
/// variable remains first for isolated local-release tests and administrators
/// that intentionally manage a custom layout.
fn windie_install_dir(user_home: &Path) -> Result<PathBuf> {
    if let Some(path) = env::var_os("WINDIE_INSTALL_DIR") {
        return Ok(PathBuf::from(path));
    }

    if let Some(path) = packaged_executable_directory()? {
        return Ok(path);
    }

    #[cfg(windows)]
    {
        if let Some(local_app_data) = env::var_os("LOCALAPPDATA") {
            return Ok(PathBuf::from(local_app_data).join("Windie").join("bin"));
        }
    }

    Ok(user_home.join(".local").join("bin"))
}

/// Returns the directory of a packaged Windie executable when recognizable.
///
/// The release manifest is the ownership marker placed beside all published
/// binaries. Requiring it prevents a developer binary under `target\debug` or
/// `target\release` from accidentally turning its build directory into an
/// uninstall target.
fn packaged_executable_directory() -> Result<Option<PathBuf>> {
    let executable = env::current_exe().ok();
    Ok(executable.and_then(|executable| packaged_executable_directory_for(&executable)))
}

/// Recognizes one packaged executable path using the adjacent release marker.
fn packaged_executable_directory_for(executable: &Path) -> Option<PathBuf> {
    let directory = executable.parent()?;
    let name = executable.file_name().and_then(|name| name.to_str())?;

    if !name.eq_ignore_ascii_case(&executable_name("windie"))
        || !directory.join("release-manifest.txt").is_file()
    {
        return None;
    }

    Some(directory.to_path_buf())
}

/// Returns an absolute, lexical path without requiring the target to exist.
fn absolute_path(path: &Path) -> Result<PathBuf> {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        env::current_dir()?.join(path)
    };
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            _ => normalized.push(component.as_os_str()),
        }
    }
    Ok(normalized)
}

/// Rejects paths that could turn uninstall into a broad recursive deletion.
fn validate_uninstall_paths(
    user_home: &Path,
    windie_home: &Path,
    install_dir: &Path,
) -> Result<()> {
    let root = Path::new(std::path::MAIN_SEPARATOR_STR);
    if windie_home == root || windie_home == user_home || windie_home.parent().is_none() {
        return Err(anyhow!(
            "refusing to uninstall: Windie data path is unsafe: {}",
            windie_home.display()
        ));
    }
    if !matches!(
        windie_home.file_name().and_then(|name| name.to_str()),
        Some(".windie") | Some("windie")
    ) {
        return Err(anyhow!(
            "refusing to uninstall: data path must be a Windie directory: {}",
            windie_home.display()
        ));
    }
    if install_dir == root
        || install_dir.parent().is_none()
        || install_dir == user_home
        || install_dir == windie_home
        || install_dir.starts_with(windie_home)
        || windie_home.starts_with(install_dir)
    {
        return Err(anyhow!(
            "refusing to uninstall: install and data paths overlap unsafely"
        ));
    }
    Ok(())
}

/// Validates both the safe roots and the exact binary list selected by Windie.
fn validate_uninstall_plan(plan: &UninstallPlan, user_home: &Path) -> Result<()> {
    validate_uninstall_paths(user_home, &plan.windie_home, &plan.install_dir)?;
    let expected = ["windie", "bifrost", "windie-inspector"]
        .into_iter()
        .map(|name| plan.install_dir.join(executable_name(name)))
        .collect::<Vec<_>>();
    if plan.binaries != expected {
        return Err(anyhow!(
            "refusing to uninstall: plan contains paths outside Windie's owned binaries"
        ));
    }
    Ok(())
}

/// Refuses to follow or remove a symlink at an owned file target.
fn remove_owned_file(path: &Path) -> Result<Option<PathBuf>> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error).with_context(|| format!("failed to inspect {}", path.display()));
        }
    };
    if metadata.file_type().is_symlink() {
        return Err(anyhow!(
            "refusing to remove symlink at Windie-owned path {}",
            path.display()
        ));
    }
    if !metadata.is_file() {
        return Err(anyhow!(
            "refusing to remove non-file at Windie-owned path {}",
            path.display()
        ));
    }
    fs::remove_file(path).with_context(|| format!("failed to remove {}", path.display()))?;
    Ok(Some(path.to_path_buf()))
}

/// Refuses to recursively remove a symlink in place of Windie's data root.
fn ensure_owned_directory(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("failed to inspect Windie data {}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(anyhow!(
            "refusing to recursively remove non-directory Windie data path {}",
            path.display()
        ));
    }
    Ok(())
}

/// Returns the platform-specific installed executable filename.
fn executable_name(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

#[cfg(windows)]
fn schedule_windows_cleanup(plan: &UninstallPlan) -> Result<()> {
    let mut paths = plan.binaries.clone();
    paths.push(plan.windie_home.clone());
    let path_literals = paths
        .iter()
        .map(|path| powershell_single_quoted(path.to_string_lossy().as_ref()))
        .collect::<Vec<_>>()
        .join(", ");
    let script = format!(
        "Start-Sleep -Milliseconds 500; $paths = @({path_literals}); foreach ($path in $paths) {{ if (Test-Path -LiteralPath $path) {{ Remove-Item -LiteralPath $path -Recurse -Force -ErrorAction SilentlyContinue }} }}"
    );
    let mut command = Command::new("powershell.exe");
    command.args([
        "-NoProfile",
        "-NonInteractive",
        "-WindowStyle",
        "Hidden",
        "-Command",
        &script,
    ]);
    command
        .spawn()
        .context("failed to schedule Windows Windie cleanup")?;
    Ok(())
}

#[cfg(windows)]
fn powershell_single_quoted(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

#[cfg(not(windows))]
fn schedule_windows_cleanup(_plan: &UninstallPlan) -> Result<()> {
    Ok(())
}

/// Installs CUA Driver using its public upstream installer when needed.
fn install_cua_driver() -> Result<InstallReport> {
    #[cfg(target_os = "windows")]
    {
        if runtime::resolve_command("cua-driver").is_err() {
            install_cua_driver_windows()?;
        }
        runtime::resolve_command("cua-driver")?;
        return Ok(InstallReport {
            target: "cua-driver".to_string(),
            message: "installed or verified cua-driver with its official Windows installer"
                .to_string(),
            status: InstallStatus::Detected,
        });
    }

    #[cfg(target_os = "macos")]
    if runtime::resolve_command("cua-driver").is_ok() && runtime::cua_driver_app_available() {
        return Ok(InstallReport {
            target: "cua-driver".to_string(),
            message: "cua-driver and its macOS application are already available".to_string(),
            status: InstallStatus::Detected,
        });
    }

    #[cfg(not(target_os = "macos"))]
    if command_exists("cua-driver") {
        return Ok(InstallReport {
            target: "cua-driver".to_string(),
            message: "cua-driver is already available on PATH".to_string(),
            status: InstallStatus::Detected,
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

    runtime::resolve_command("cua-driver")
        .context("cua-driver installer completed but cua-driver is not resolvable")?;

    #[cfg(target_os = "macos")]
    if !runtime::cua_driver_app_available() {
        return Err(anyhow!(
            "cua-driver installer completed but /Applications/CuaDriver.app is not installed"
        ));
    }

    Ok(InstallReport {
        target: "cua-driver".to_string(),
        message: "installed cua-driver with the public trycua installer".to_string(),
        status: InstallStatus::Installed,
    })
}

#[cfg(target_os = "windows")]
fn install_cua_driver_windows() -> Result<()> {
    const INSTALL_URL: &str =
        "https://raw.githubusercontent.com/trycua/cua/main/libs/cua-driver/scripts/install.ps1";
    let script = format!(
        "$ErrorActionPreference = 'Stop'; $path = Join-Path $env:TEMP 'windie-cua-install.ps1'; Invoke-WebRequest -UseBasicParsing -Uri '{INSTALL_URL}' -OutFile $path; & $path; $code = $LASTEXITCODE; Remove-Item -LiteralPath $path -Force -ErrorAction SilentlyContinue; exit $code"
    );
    let status = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            &script,
        ])
        .status()
        .context("failed to start the CUA Driver Windows installer")?;
    if !status.success() {
        return Err(anyhow!(
            "CUA Driver Windows installer failed with status {status}"
        ));
    }
    runtime::resolve_command("cua-driver")
        .context("CUA Driver installer completed but cua-driver is not resolvable")?;
    Ok(())
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

    env::split_paths(&paths).any(|path| {
        if path.join(program).is_file() {
            return true;
        }
        #[cfg(target_os = "windows")]
        {
            return [".exe", ".cmd", ".bat"]
                .iter()
                .any(|suffix| path.join(format!("{program}{suffix}")).is_file());
        }
        #[cfg(not(target_os = "windows"))]
        false
    })
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
        if LLM_ENV_KEYS.contains(&key) {
            return Err(anyhow!(
                "LLM provider keys are managed by Bifrost; use `windie onboard`: {key}"
            ));
        }
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

/// Returns the value assigned to a target key by one `.env` line.
fn env_line_value(line: &str, target_key: &str) -> Option<String> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let line = line.strip_prefix("export ").unwrap_or(line).trim();
    let (key, value) = line.split_once('=')?;
    if key.trim() != target_key {
        return None;
    }

    Some(unquote_env_value(value.trim()).to_string())
}

/// Removes matching quote characters around a full `.env` value.
fn unquote_env_value(value: &str) -> &str {
    if value.len() >= 2 {
        let bytes = value.as_bytes();
        if (bytes[0] == b'"' && bytes[value.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[value.len() - 1] == b'\'')
        {
            return &value[1..value.len() - 1];
        }
    }

    value
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uninstall_rejects_home_root_and_overlapping_paths() {
        let home = Path::new("/Users/example");
        let install = home.join(".local/bin");

        assert!(validate_uninstall_paths(home, home, &install).is_err());
        assert!(validate_uninstall_paths(home, Path::new("/"), &install).is_err());
        assert!(validate_uninstall_paths(home, &home.join(".windie"), home).is_err());
        assert!(
            validate_uninstall_paths(home, &home.join(".windie"), &home.join(".windie/bin"))
                .is_err()
        );
    }

    #[test]
    fn uninstall_removes_only_exact_windie_paths() {
        let root =
            std::env::temp_dir().join(format!("windie-uninstall-test-{}", std::process::id()));
        let home = root.join("home");
        let windie_home = home.join(".windie");
        let install_dir = home.join(".local/bin");
        let windie_binary = install_dir.join(executable_name("windie"));
        let unrelated_file = install_dir.join("unrelated");
        fs::create_dir_all(&windie_home).unwrap();
        fs::create_dir_all(&install_dir).unwrap();
        fs::write(&windie_binary, "owned").unwrap();
        fs::write(&unrelated_file, "preserve").unwrap();

        let binaries = ["windie", "bifrost", "windie-inspector"]
            .into_iter()
            .map(|name| install_dir.join(executable_name(name)))
            .collect();
        let plan = UninstallPlan {
            windie_home: windie_home.clone(),
            install_dir: install_dir.clone(),
            binaries,
        };
        let cleanup = remove_uninstall_plan(&plan).unwrap();

        assert!(cleanup.removed_data);
        assert!(!windie_home.exists());
        assert!(!windie_binary.exists());
        assert!(unrelated_file.exists());
        assert!(install_dir.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn packaged_executable_requires_release_manifest() {
        let root = std::env::temp_dir().join(format!(
            "windie-packaged-install-test-{}",
            std::process::id()
        ));
        let executable = root.join(executable_name("windie"));
        fs::create_dir_all(&root).unwrap();
        fs::write(&executable, "owned").unwrap();

        assert_eq!(packaged_executable_directory_for(&executable), None);

        fs::write(root.join("release-manifest.txt"), "version=local\n").unwrap();
        assert_eq!(
            packaged_executable_directory_for(&executable),
            Some(root.clone())
        );

        fs::remove_dir_all(root).unwrap();
    }

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
    fn env_line_value_reads_plain_export_and_quoted_assignments() {
        assert_eq!(
            env_line_value("OPENAI_API_KEY=value", "OPENAI_API_KEY"),
            Some("value".to_string())
        );
        assert_eq!(
            env_line_value("export OPENROUTER_API_KEY='quoted'", "OPENROUTER_API_KEY"),
            Some("quoted".to_string())
        );
        assert_eq!(
            env_line_value("BRIGHTDATA_API_TOKEN=\"bright\"", "BRIGHTDATA_API_TOKEN"),
            Some("bright".to_string())
        );
        assert_eq!(
            env_line_value("# OPENAI_API_KEY=value", "OPENAI_API_KEY"),
            None
        );
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

    #[test]
    fn rejects_llm_provider_env_key() {
        let error = validate_env_key("OPENAI_API_KEY").unwrap_err();

        assert!(error.to_string().contains("managed by Bifrost"));
    }
}
