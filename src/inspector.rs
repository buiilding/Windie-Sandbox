//! Local browser launcher for the Windie inspector.
//!
//! The inspector is a developer client, not runtime code. This module only
//! finds and starts the React dev server, then opens a browser URL containing
//! the same localhost API token used by `windie api`.

use std::fs::OpenOptions;
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};

use crate::setup;

const INSPECTOR_PORT: u16 = 3000;
const INSPECTOR_START_TIMEOUT: Duration = Duration::from_secs(90);
const INSPECTOR_POLL_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, PartialEq, Eq)]
/// Result of one inspector launcher command.
pub struct InspectorLaunch {
    pub url: String,
    pub started_server: bool,
}

/// Starts the local inspector server when needed and opens it in the browser.
pub fn open(api_token: &str) -> Result<InspectorLaunch> {
    let url = inspector_url(api_token);
    let started_server = ensure_inspector_server()?;
    open_browser(&url)?;

    Ok(InspectorLaunch {
        url,
        started_server,
    })
}

/// Builds the browser URL with the API token already attached.
fn inspector_url(api_token: &str) -> String {
    format!(
        "http://localhost:{INSPECTOR_PORT}?windie_token={}",
        encode_query_value(api_token)
    )
}

/// Starts the React development server if nothing is already listening.
fn ensure_inspector_server() -> Result<bool> {
    if inspector_server_is_running() {
        return Ok(false);
    }

    let inspector_dir = find_inspector_dir()?;
    ensure_node_dependencies(&inspector_dir)?;
    start_inspector_server(&inspector_dir)?;
    wait_for_inspector_server()?;

    Ok(true)
}

/// Finds `dev/windie-inspector` from common local development launch points.
fn find_inspector_dir() -> Result<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(current_dir) = std::env::current_dir() {
        candidates.push(current_dir.join("dev").join("windie-inspector"));
        candidates.push(
            current_dir
                .join("windie")
                .join("dev")
                .join("windie-inspector"),
        );
    }
    if let Ok(current_exe) = std::env::current_exe() {
        for ancestor in current_exe.ancestors() {
            candidates.push(ancestor.join("dev").join("windie-inspector"));
        }
    }

    candidates
        .into_iter()
        .find(|path| path.join("package.json").is_file())
        .ok_or_else(|| {
            anyhow!(
                "failed to find dev/windie-inspector; run this command from the Windie repository or with the repository-built binary"
            )
        })
}

/// Installs frontend dependencies on first use so `windie inspector` is enough
/// to bring up the local browser client after a fresh checkout.
fn ensure_node_dependencies(inspector_dir: &Path) -> Result<()> {
    if inspector_dir.join("node_modules").is_dir() {
        return Ok(());
    }

    let status = Command::new("npm")
        .arg("install")
        .arg("--legacy-peer-deps")
        .current_dir(inspector_dir)
        .status()
        .context("failed to start npm install for Windie inspector")?;
    if status.success() {
        return Ok(());
    }

    Err(anyhow!("npm install failed for Windie inspector"))
}

/// Starts `npm run start` detached, with output redirected to `~/.windie`.
fn start_inspector_server(inspector_dir: &Path) -> Result<()> {
    let log_file = setup::inspector_log_file_path()?;
    let stdout = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_file)
        .with_context(|| format!("failed to open {}", log_file.display()))?;
    let stderr = stdout
        .try_clone()
        .with_context(|| format!("failed to open {}", log_file.display()))?;

    Command::new("npm")
        .arg("run")
        .arg("start")
        .current_dir(inspector_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .spawn()
        .context("failed to start Windie inspector dev server")?;

    Ok(())
}

/// Waits until the local inspector port accepts connections.
fn wait_for_inspector_server() -> Result<()> {
    let started = Instant::now();
    while started.elapsed() < INSPECTOR_START_TIMEOUT {
        if inspector_server_is_running() {
            return Ok(());
        }
        std::thread::sleep(INSPECTOR_POLL_INTERVAL);
    }

    Err(anyhow!(
        "Windie inspector did not start within {} seconds",
        INSPECTOR_START_TIMEOUT.as_secs()
    ))
}

/// Returns whether the local inspector dev server is accepting TCP connections.
fn inspector_server_is_running() -> bool {
    let address = SocketAddr::from(([127, 0, 0, 1], INSPECTOR_PORT));
    TcpStream::connect_timeout(&address, Duration::from_millis(200)).is_ok()
}

/// Opens the system browser at one URL.
fn open_browser(url: &str) -> Result<()> {
    let mut command = browser_command(url);
    let status = command.status().context("failed to open browser")?;
    if status.success() {
        return Ok(());
    }

    Err(anyhow!("failed to open browser"))
}

/// Builds the platform browser opener command.
#[cfg(target_os = "macos")]
fn browser_command(url: &str) -> Command {
    let mut command = Command::new("open");
    command.arg(url);
    command
}

/// Builds the platform browser opener command.
#[cfg(target_os = "windows")]
fn browser_command(url: &str) -> Command {
    let mut command = Command::new("cmd");
    command.args(["/C", "start", "", url]);
    command
}

/// Builds the platform browser opener command.
#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
fn browser_command(url: &str) -> Command {
    let mut command = Command::new("xdg-open");
    command.arg(url);
    command
}

/// Percent-encodes one URL query value without adding another dependency.
fn encode_query_value(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                encoded.push(byte as char);
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }

    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inspector_url_includes_encoded_token() {
        assert_eq!(
            inspector_url("abc 123"),
            "http://localhost:3000?windie_token=abc%20123"
        );
    }
}
