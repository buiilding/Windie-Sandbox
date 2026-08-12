//! Installation and cleanup operations for approved MCP servers.
//!
//! The local module owns Windie's data directory and environment files. This
//! module owns MCP-specific installation targets, managed runtime preparation,
//! and upstream CUA Driver installation and cleanup.

use std::env;
use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, anyhow};

use crate::local::ensure_windie_layout;

use super::runtime;

const CUA_DRIVER_INSTALL_URL: &str =
    "https://raw.githubusercontent.com/trycua/cua/main/libs/cua-driver/scripts/install.sh";
const CUA_DRIVER_UNINSTALL_URL: &str =
    "https://raw.githubusercontent.com/trycua/cua/main/libs/cua-driver/scripts/uninstall.sh";

#[derive(Debug, Clone, PartialEq, Eq)]
/// Result of one approved MCP installation request.
pub struct InstallReport {
    pub target: String,
    pub message: String,
    pub status: InstallStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Describes whether an approved MCP dependency was reused or installed.
pub enum InstallStatus {
    Detected,
    Installed,
}

/// Installs or verifies one approved Windie MCP dependency.
pub fn install_target(target: &str) -> Result<InstallReport> {
    ensure_windie_layout()?;

    match target {
        "bifrost" => Ok(InstallReport {
            target: target.to_string(),
            message: "Bifrost is provided by the Windie-owned bundled binary".to_string(),
            status: InstallStatus::Detected,
        }),
        "cua-driver" => install_cua_driver(),
        "desktop-commander" => runtime_report(
            target,
            "Windie-managed Node.js runtime is ready for Desktop Commander",
        ),
        "blender-mcp" => {
            runtime_report(target, "Windie-managed uv runtime is ready for Blender MCP")
        }
        "brightdata" => runtime_report(
            target,
            "Windie-managed Node.js runtime is ready for Bright Data MCP",
        ),
        "basic-memory" => runtime_report(
            target,
            "Windie-managed uv runtime is ready for Basic Memory",
        ),
        _ => Err(anyhow!("unknown install target: {target}")),
    }
}

fn runtime_report(target: &str, message: &str) -> Result<InstallReport> {
    let installed = runtime::ensure_provider_runtime(target)?;
    Ok(InstallReport {
        target: target.to_string(),
        message: message.to_string(),
        status: if installed {
            InstallStatus::Installed
        } else {
            InstallStatus::Detected
        },
    })
}

/// Removes exact provider-owned directories beneath Windie's data root.
pub(crate) fn remove_windie_directories(paths: &[&str]) -> Result<()> {
    let root = absolute_path(&crate::local::windie_home_dir()?)?;
    for relative in paths {
        let relative_path = Path::new(relative);
        if relative_path.is_absolute()
            || relative_path
                .components()
                .any(|component| matches!(component, std::path::Component::ParentDir))
        {
            return Err(anyhow!(
                "provider cleanup path must be relative and cannot contain '..': {relative}"
            ));
        }

        let target = root.join(relative_path);
        if !target.starts_with(&root) {
            return Err(anyhow!(
                "provider cleanup path escapes Windie's data root: {relative}"
            ));
        }

        let metadata = match fs::symlink_metadata(&target) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "failed to inspect provider cleanup path: {}",
                        target.display()
                    )
                });
            }
        };
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(anyhow!(
                "refusing to remove provider cleanup path that is not a directory: {}",
                target.display()
            ));
        }

        fs::remove_dir_all(&target).with_context(|| {
            format!("failed to remove provider directory: {}", target.display())
        })?;
    }
    Ok(())
}

/// Returns an absolute lexical path without requiring the target to exist.
fn absolute_path(path: &Path) -> Result<std::path::PathBuf> {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        env::current_dir()?.join(path)
    };
    let mut normalized = std::path::PathBuf::new();
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

/// Runs CUA Driver's official platform-specific uninstaller with purge mode.
pub(crate) fn uninstall_cua_driver() -> Result<()> {
    #[cfg(target_os = "windows")]
    {
        let script = format!(
            "$ErrorActionPreference = 'Stop'; $env:CUA_DRIVER_RS_UNINSTALL_FORCE = '1'; $env:CUA_DRIVER_RS_UNINSTALL_PURGE = '1'; $path = Join-Path $env:TEMP 'windie-cua-uninstall.ps1'; Invoke-WebRequest -UseBasicParsing -Uri '{CUA_DRIVER_UNINSTALL_URL}' -OutFile $path; $code = 0; try {{ & $path; $code = if ($null -eq $LASTEXITCODE) {{ 0 }} else {{ $LASTEXITCODE }} }} finally {{ Remove-Item -LiteralPath $path -Force -ErrorAction SilentlyContinue }}; exit $code"
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
            .context("failed to start cua-driver uninstaller")?;
        if !status.success() {
            return Err(anyhow!("cua-driver uninstaller failed"));
        }
        return Ok(());
    }

    #[cfg(not(target_os = "windows"))]
    {
        require_command("curl")?;
        require_command("bash")?;
        let status = Command::new("bash")
            .args(["-c", &format!("curl -fsSL --proto '=https' --tlsv1.2 {CUA_DRIVER_UNINSTALL_URL} | bash -s -- --purge")])
            .status()
            .context("failed to start cua-driver uninstaller")?;
        if !status.success() {
            return Err(anyhow!("cua-driver uninstaller failed"));
        }
        Ok(())
    }
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

fn require_command(program: &str) -> Result<()> {
    if command_exists(program) {
        return Ok(());
    }
    Err(anyhow!(
        "required command is not available on PATH: {program}"
    ))
}

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
