//! Bifrost gateway availability and lifecycle.
//!
//! This module checks whether the local Bifrost HTTP gateway is healthy and
//! starts or stops the current dev/workspace gateway when explicitly requested.

use std::collections::BTreeSet;
use std::env;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use reqwest::Client;
use tokio::time::sleep;

// Current workspace gateway mode. Future production builds should prefer a
// minimal/headless Bifrost gateway when available.
const DEV_BIFROST_DIR: &str = "bifrost";
const DEV_BIFROST_BINARY: &str = "tmp/bifrost-http";
const DEV_BIFROST_APP_DIR: &str = "data";
const BIFROST_PORT: &str = "8080";
const START_TIMEOUT: Duration = Duration::from_secs(10);
const HEALTH_CHECK_INTERVAL: Duration = Duration::from_millis(200);

#[derive(Debug, Clone, PartialEq, Eq)]
/// Base URL for the local Bifrost gateway health endpoint.
pub struct GatewayUrl(String);

impl GatewayUrl {
    /// Stores the URL without a trailing slash so endpoint joining is stable.
    pub fn new(url: impl Into<String>) -> Self {
        Self(url.into().trim_end_matches('/').to_string())
    }

    /// Returns the normalized gateway URL text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for GatewayUrl {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Local Bifrost gateway lifecycle and readiness client.
pub struct BifrostGateway {
    http: Client,
    url: GatewayUrl,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Result of an explicit gateway start request.
pub enum GatewayStart {
    AlreadyRunning,
    Started,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Result of an explicit gateway stop request.
pub enum GatewayStop {
    NotRunning,
    Stopped,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Filesystem paths needed to start the development Bifrost gateway.
struct BifrostPaths {
    binary: PathBuf,
    app_dir: PathBuf,
}

impl BifrostGateway {
    /// Creates a gateway client for a specific local gateway URL.
    pub fn new(url: GatewayUrl) -> Self {
        Self {
            http: Client::new(),
            url,
        }
    }

    /// Starts Bifrost only when the health endpoint is not already available.
    pub async fn start(&self) -> Result<GatewayStart> {
        if self.is_running().await {
            return Ok(GatewayStart::AlreadyRunning);
        }

        self.start_process()?;
        self.wait_until_running().await?;

        Ok(GatewayStart::Started)
    }

    /// Stops Bifrost processes listening on Windie's configured gateway port.
    pub async fn stop(&self) -> Result<GatewayStop> {
        if !self.is_running().await {
            return Ok(GatewayStop::NotRunning);
        }

        let process_ids = bifrost_process_ids_on_port(BIFROST_PORT)?;
        if process_ids.is_empty() {
            return Err(anyhow!(
                "Bifrost appears to be running on port {BIFROST_PORT}, but Windie could not find a Bifrost process to stop"
            ));
        }

        for process_id in process_ids {
            let status = Command::new("kill")
                .arg(process_id.to_string())
                .status()
                .with_context(|| format!("failed to stop Bifrost process {process_id}"))?;
            if !status.success() {
                return Err(anyhow!("failed to stop Bifrost process {process_id}"));
            }
        }

        self.wait_until_stopped().await?;

        Ok(GatewayStop::Stopped)
    }

    /// Requires Bifrost to already be running for commands that should not
    /// cause hidden gateway startup.
    pub async fn require_running(&self) -> Result<()> {
        if self.is_running().await {
            return Ok(());
        }

        Err(anyhow!(
            "Bifrost is not running. Start it with: windie gateway start"
        ))
    }

    /// Returns whether the gateway health endpoint currently responds
    /// successfully.
    pub async fn is_running(&self) -> bool {
        self.health_check().await.is_ok()
    }

    /// Calls the gateway health endpoint and treats non-2xx responses as not
    /// healthy.
    async fn health_check(&self) -> Result<()> {
        let health_url = format!("{}/health", self.url);
        let response = self
            .http
            .get(&health_url)
            .send()
            .await
            .context("failed to reach Bifrost health endpoint")?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow!("Bifrost health check failed with {status}: {body}"));
        }

        Ok(())
    }

    /// Spawns the local development Bifrost binary with the known app dir and
    /// port.
    fn start_process(&self) -> Result<()> {
        let paths = find_dev_bifrost_paths()?;

        Command::new(&paths.binary)
            .arg("-app-dir")
            .arg(&paths.app_dir)
            .arg("-port")
            .arg(BIFROST_PORT)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("failed to start Bifrost")?;

        Ok(())
    }

    /// Polls the health endpoint until startup succeeds or times out.
    async fn wait_until_running(&self) -> Result<()> {
        let mut waited = Duration::ZERO;

        while waited < START_TIMEOUT {
            if self.is_running().await {
                return Ok(());
            }

            sleep(HEALTH_CHECK_INTERVAL).await;
            waited += HEALTH_CHECK_INTERVAL;
        }

        Err(anyhow!(
            "Bifrost did not become healthy within {} seconds",
            START_TIMEOUT.as_secs()
        ))
    }

    /// Polls the health endpoint until shutdown succeeds or times out.
    async fn wait_until_stopped(&self) -> Result<()> {
        let mut waited = Duration::ZERO;

        while waited < START_TIMEOUT {
            if !self.is_running().await {
                return Ok(());
            }

            sleep(HEALTH_CHECK_INTERVAL).await;
            waited += HEALTH_CHECK_INTERVAL;
        }

        Err(anyhow!(
            "Bifrost did not stop within {} seconds",
            START_TIMEOUT.as_secs()
        ))
    }
}

/// Finds Bifrost process IDs listening on a port and filters out unrelated
/// processes that may also be reported by `lsof`.
fn bifrost_process_ids_on_port(port: &str) -> Result<Vec<u32>> {
    let output = Command::new("lsof")
        .args(["-nP", &format!("-iTCP:{port}"), "-sTCP:LISTEN", "-t"])
        .output()
        .context("failed to inspect local gateway process")?;

    if !output.status.success() && output.stdout.is_empty() {
        return Ok(Vec::new());
    }

    let mut process_ids = BTreeSet::new();
    for process_id in parse_process_ids(&String::from_utf8_lossy(&output.stdout)) {
        let command = process_command(process_id)?;
        if is_bifrost_command(&command) {
            process_ids.insert(process_id);
        }
    }

    Ok(process_ids.into_iter().collect())
}

/// Parses numeric process IDs from `lsof -t` output.
fn parse_process_ids(output: &str) -> Vec<u32> {
    output
        .lines()
        .filter_map(|line| line.trim().parse::<u32>().ok())
        .collect()
}

/// Reads the full command line for one process ID.
fn process_command(process_id: u32) -> Result<String> {
    let output = Command::new("ps")
        .args(["-p", &process_id.to_string(), "-o", "command="])
        .output()
        .with_context(|| format!("failed to inspect process {process_id}"))?;

    if !output.status.success() {
        return Ok(String::new());
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Identifies whether a process command line belongs to Bifrost.
fn is_bifrost_command(command: &str) -> bool {
    command.contains("bifrost-http")
}

/// Finds the development Bifrost binary relative to common workspace roots.
fn find_dev_bifrost_paths() -> Result<BifrostPaths> {
    let roots = dev_bifrost_search_roots();
    find_dev_bifrost_paths_in(roots).ok_or_else(|| {
        anyhow!("Bifrost binary was not found. Build Bifrost before running Windie.")
    })
}

/// Searches candidate roots for the development Bifrost layout.
fn find_dev_bifrost_paths_in(roots: impl IntoIterator<Item = PathBuf>) -> Option<BifrostPaths> {
    for root in roots {
        for bifrost_dir in dev_bifrost_dirs(&root) {
            let paths = BifrostPaths {
                binary: bifrost_dir.join(DEV_BIFROST_BINARY),
                app_dir: bifrost_dir.join(DEV_BIFROST_APP_DIR),
            };

            if paths.binary.exists() {
                return Some(paths);
            }
        }
    }

    None
}

/// Builds the list of roots used to discover the sibling/local Bifrost checkout.
fn dev_bifrost_search_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();

    if let Ok(current_dir) = env::current_dir() {
        roots.push(current_dir);
    }

    if let Ok(exe_path) = env::current_exe()
        && let Some(exe_dir) = exe_path.parent()
    {
        roots.push(exe_dir.to_path_buf());
    }

    roots.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")));
    roots
}

/// Returns both supported Bifrost locations for a root: inside the root and next
/// to it.
fn dev_bifrost_dirs(root: &Path) -> [PathBuf; 2] {
    [
        root.join(DEV_BIFROST_DIR),
        root.parent()
            .map(|parent| parent.join(DEV_BIFROST_DIR))
            .unwrap_or_else(|| root.join(DEV_BIFROST_DIR)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gateway_url_removes_trailing_slash() {
        let url = GatewayUrl::new("http://localhost:8080/");

        assert_eq!(url.as_str(), "http://localhost:8080");
    }

    #[test]
    fn finds_bifrost_under_root() {
        let root = env::temp_dir().join(format!("windie-gateway-test-{}", std::process::id()));
        let bifrost_dir = root.join("bifrost");
        let binary = bifrost_dir.join("tmp").join("bifrost-http");
        std::fs::create_dir_all(binary.parent().unwrap()).unwrap();
        std::fs::create_dir_all(bifrost_dir.join("data")).unwrap();
        std::fs::write(&binary, "").unwrap();

        let paths = find_dev_bifrost_paths_in([root.clone()]).unwrap();

        assert_eq!(paths.binary, binary);
        assert_eq!(paths.app_dir, bifrost_dir.join("data"));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn finds_bifrost_next_to_root() {
        let workspace =
            env::temp_dir().join(format!("windie-gateway-parent-test-{}", std::process::id()));
        let windie_dir = workspace.join("windie");
        let bifrost_dir = workspace.join("bifrost");
        let binary = bifrost_dir.join("tmp").join("bifrost-http");
        std::fs::create_dir_all(&windie_dir).unwrap();
        std::fs::create_dir_all(binary.parent().unwrap()).unwrap();
        std::fs::create_dir_all(bifrost_dir.join("data")).unwrap();
        std::fs::write(&binary, "").unwrap();

        let paths = find_dev_bifrost_paths_in([windie_dir]).unwrap();

        assert_eq!(paths.binary, binary);
        assert_eq!(paths.app_dir, bifrost_dir.join("data"));

        let _ = std::fs::remove_dir_all(workspace);
    }

    #[test]
    fn parses_process_ids() {
        let process_ids = parse_process_ids("123\nnot-a-pid\n456\n");

        assert_eq!(process_ids, vec![123, 456]);
    }

    #[test]
    fn recognizes_bifrost_process_command() {
        assert!(is_bifrost_command(
            "/Users/peterbui/Documents/WindieOS/bifrost/tmp/bifrost-http -port 8080"
        ));
        assert!(!is_bifrost_command("python3 -m http.server 8080"));
    }
}
