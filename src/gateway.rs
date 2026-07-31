//! Bifrost gateway availability and lifecycle.
//!
//! This module checks whether the local Bifrost HTTP gateway is healthy and
//! starts or stops a gateway when explicitly requested.
//!
//! Startup uses an owned Bifrost binary so Windie and Bifrost can evolve
//! together without depending on an upstream package release.

use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use reqwest::Client;
use tokio::time::sleep;

use crate::local;

const BIFROST_DIR: &str = "bifrost";
const BIFROST_DATA_DIR: &str = "data";
const BIFROST_LOG_FILE: &str = "windie-gateway.log";
const BIFROST_BINARY_ENV: &str = "WINDIE_BIFROST_BIN";
const BIFROST_PORT: &str = "8080";
const START_TIMEOUT: Duration = Duration::from_secs(60);
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

    /// Returns the configured gateway port, defaulting to Bifrost's standard
    /// port when the URL omits one.
    pub fn port(&self) -> String {
        self.0
            .rsplit_once(':')
            .map(|(_, port)| port.trim_end_matches('/'))
            .filter(|port| port.chars().all(|character| character.is_ascii_digit()))
            .unwrap_or(BIFROST_PORT)
            .to_string()
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
/// Public Bifrost runtime paths owned by Windie.
struct BifrostPaths {
    dir: PathBuf,
    app_dir: PathBuf,
    log_file: PathBuf,
    pid_file: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Concrete way Windie will start Bifrost.
enum BifrostLauncher {
    Binary { path: PathBuf, paths: BifrostPaths },
}

impl BifrostGateway {
    /// Creates a gateway client for a specific local gateway URL.
    pub fn new(url: GatewayUrl) -> Self {
        Self {
            http: Client::builder()
                .timeout(Duration::from_secs(2))
                .build()
                .expect("gateway HTTP client configuration is valid"),
            url,
        }
    }

    /// Starts Bifrost only when the health endpoint is not already available.
    pub async fn start(&self) -> Result<GatewayStart> {
        if self.is_running().await {
            return Ok(GatewayStart::AlreadyRunning);
        }

        let paths = bifrost_paths()?;
        cleanup_stale_owned_process(&paths)?;
        self.start_process()?;
        if let Err(error) = self.wait_until_running().await {
            let _ = stop_owned_process(&paths);
            return Err(error);
        }

        Ok(GatewayStart::Started)
    }

    /// Stops Bifrost processes listening on Windie's configured gateway port.
    pub async fn stop(&self) -> Result<GatewayStop> {
        let paths = bifrost_paths()?;
        let owned_pid = read_pid_file(&paths.pid_file)?;
        if !self.is_running().await && owned_pid.is_none() {
            return Ok(GatewayStop::NotRunning);
        }

        let port = self.url.port();
        let process_ids = if let Some(process_id) = owned_pid {
            vec![process_id]
        } else {
            bifrost_process_ids_on_port(&port)?
        };
        if process_ids.is_empty() {
            remove_pid_file(&paths.pid_file)?;
            return Err(anyhow!(
                "Bifrost appears to be running on port {port}, but Windie could not find a Bifrost process to stop"
            ));
        }

        for process_id in process_ids {
            if let Some(command) = process_command(process_id).ok()
                && !is_bifrost_command(&command)
            {
                return Err(anyhow!(
                    "refusing to stop non-Bifrost process {process_id} listening on port {port}: {command}"
                ));
            }
            let status = stop_process(process_id)?;
            if !status.success() {
                return Err(anyhow!("failed to stop Bifrost process {process_id}"));
            }
        }

        remove_pid_file(&paths.pid_file)?;
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

    /// Spawns the owned Bifrost binary with the first available path.
    ///
    /// Bifrost inherits the same environment as Windie. There is no separate
    /// environment allowlist or `.env` injection at this process boundary.
    fn start_process(&self) -> Result<()> {
        let launcher = find_bifrost_launcher()?;

        let BifrostLauncher::Binary { path, paths } = launcher;
        let port = self.url.port();
        start_binary_process(&path, &paths, &port)
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

        let log_file = bifrost_paths().ok().map(|paths| paths.log_file);
        let log_tail = log_file
            .as_deref()
            .and_then(read_log_tail)
            .filter(|text| !text.is_empty())
            .map(|text| format!("\nBifrost log tail:\n{text}"))
            .unwrap_or_default();
        Err(anyhow!(
            "Bifrost did not become healthy within {} seconds. Check {}{}{}",
            START_TIMEOUT.as_secs(),
            log_file
                .as_deref()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "the Windie gateway log".to_string()),
            log_tail,
            port_owner_diagnostics(&self.url.port())
                .map(|owners| {
                    if owners.is_empty() {
                        String::new()
                    } else {
                        format!("\nPort {} owners:\n{owners}", self.url.port())
                    }
                })
                .unwrap_or_default()
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

/// Starts the Windie-owned Bifrost binary.
fn start_binary_process(binary: &Path, paths: &BifrostPaths, port: &str) -> Result<()> {
    validate_release_manifest(binary)?;
    fs::create_dir_all(&paths.app_dir).with_context(|| {
        format!(
            "failed to create Bifrost app dir {}",
            paths.app_dir.display()
        )
    })?;

    let stdout = gateway_log_file(&paths.log_file)?;
    let stderr = stdout
        .try_clone()
        .with_context(|| format!("failed to open gateway log {}", paths.log_file.display()))?;

    let child = Command::new(binary)
        .arg("-app-dir")
        .arg(&paths.app_dir)
        .arg("-port")
        .arg(port)
        .current_dir(&paths.dir)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .spawn()
        .context("failed to start the Windie-owned Bifrost binary")?;
    write_pid_file(&paths.pid_file, child.id())?;
    drop(child);

    Ok(())
}

/// Validates the compatibility metadata shipped beside published binaries.
/// Development builds without a manifest remain valid and use the source
/// checkout's normal launcher behavior.
fn validate_release_manifest(binary: &Path) -> Result<()> {
    let Some(parent) = binary.parent() else {
        return Ok(());
    };
    let manifest_path = parent.join("release-manifest.txt");
    if !manifest_path.is_file() {
        return Ok(());
    }
    let text = fs::read_to_string(&manifest_path).with_context(|| {
        format!(
            "failed to read release manifest {}",
            manifest_path.display()
        )
    })?;
    let values = text
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(key, value)| (key.trim(), value.trim()))
        .collect::<std::collections::HashMap<_, _>>();
    let windie_version = values
        .get("windie_version")
        .copied()
        .ok_or_else(|| anyhow!("release manifest is missing windie_version"))?;
    let bifrost_version = values
        .get("bifrost_version")
        .copied()
        .ok_or_else(|| anyhow!("release manifest is missing bifrost_version"))?;
    // Release packages currently bundle the Bifrost build labeled `stable`
    // rather than a Windie-matched semantic version. That is an intentional
    // compatibility marker, not a release mismatch.
    if bifrost_version != "stable" && windie_version != bifrost_version {
        return Err(anyhow!(
            "Windie/Bifrost release mismatch: Windie {windie_version}, Bifrost {bifrost_version}"
        ));
    }
    Ok(())
}

/// Finds the first owned Bifrost binary available on this machine.
fn find_bifrost_launcher() -> Result<BifrostLauncher> {
    let paths = bifrost_paths()?;
    select_bifrost_launcher(owned_bifrost_candidates(), paths).ok_or_else(|| {
        anyhow!(
            "Windie-owned Bifrost binary was not found. Set {BIFROST_BINARY_ENV} or build the sibling Bifrost checkout with `go build -o tmp/bifrost-http .` in transports/bifrost-http."
        )
    })
}

/// Selects an owned Bifrost binary from explicit search inputs.
fn select_bifrost_launcher(
    candidates: impl IntoIterator<Item = PathBuf>,
    paths: BifrostPaths,
) -> Option<BifrostLauncher> {
    candidates
        .into_iter()
        .find(|candidate| candidate.is_file())
        .map(|path| BifrostLauncher::Binary { path, paths })
}

/// Returns the owned Bifrost binary candidates used by development and builds.
fn owned_bifrost_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(path) = env::var_os(BIFROST_BINARY_ENV) {
        candidates.push(PathBuf::from(path));
    }

    if let Ok(executable) = env::current_exe()
        && let Some(directory) = executable.parent()
    {
        candidates.push(directory.join("bifrost"));
        candidates.push(directory.join("bifrost-http"));
        #[cfg(windows)]
        {
            candidates.push(directory.join("bifrost.exe"));
            candidates.push(directory.join("bifrost-http.exe"));
        }
    }

    if let Ok(directory) = env::current_dir() {
        candidates.push(directory.join("bifrost").join("tmp").join("bifrost-http"));
        candidates.push(
            directory
                .join("..")
                .join("bifrost")
                .join("tmp")
                .join("bifrost-http"),
        );
        #[cfg(windows)]
        {
            candidates.push(
                directory
                    .join("bifrost")
                    .join("tmp")
                    .join("bifrost-http.exe"),
            );
            candidates.push(
                directory
                    .join("..")
                    .join("bifrost")
                    .join("tmp")
                    .join("bifrost-http.exe"),
            );
        }
    }

    candidates
}

/// Builds the owned Bifrost runtime paths under `~/.windie`.
fn bifrost_paths() -> Result<BifrostPaths> {
    let dir = local::windie_home_dir()?.join(BIFROST_DIR);
    let app_dir = dir.join(BIFROST_DATA_DIR);
    let log_file = dir.join(BIFROST_LOG_FILE);
    let pid_file = dir.join("bifrost.pid");
    fs::create_dir_all(&app_dir)
        .with_context(|| format!("failed to create Bifrost app dir {}", app_dir.display()))?;

    Ok(BifrostPaths {
        dir,
        app_dir,
        log_file,
        pid_file,
    })
}

/// Removes a stale Windie-owned Bifrost process without touching unrelated
/// processes that may have reused the PID file's number.
fn cleanup_stale_owned_process(paths: &BifrostPaths) -> Result<()> {
    let Some(process_id) = read_pid_file(&paths.pid_file)? else {
        return Ok(());
    };

    let command = process_command(process_id).unwrap_or_default();
    if command.is_empty() {
        return remove_pid_file(&paths.pid_file);
    }
    if !is_bifrost_command(&command) {
        return remove_pid_file(&paths.pid_file);
    }

    if stop_process(process_id)?.success() {
        remove_pid_file(&paths.pid_file)?;
    }
    Ok(())
}

/// Stops the process recorded as Windie's owned Bifrost process, if it still
/// identifies itself as Bifrost.
fn stop_owned_process(paths: &BifrostPaths) -> Result<()> {
    let Some(process_id) = read_pid_file(&paths.pid_file)? else {
        return Ok(());
    };
    let command = process_command(process_id).unwrap_or_default();
    if !command.is_empty() && is_bifrost_command(&command) {
        let _ = stop_process(process_id)?;
    }
    remove_pid_file(&paths.pid_file)
}

fn read_pid_file(path: &Path) -> Result<Option<u32>> {
    if !path.is_file() {
        return Ok(None);
    }
    let text = fs::read_to_string(path)
        .with_context(|| format!("failed to read Bifrost PID file {}", path.display()))?;
    let pid = text
        .trim()
        .parse::<u32>()
        .with_context(|| format!("invalid Bifrost PID file {}", path.display()))?;
    Ok(Some(pid))
}

fn write_pid_file(path: &Path, process_id: u32) -> Result<()> {
    let temporary = path.with_extension("pid.tmp");
    fs::write(&temporary, format!("{process_id}\n"))
        .with_context(|| format!("failed to write Bifrost PID file {}", path.display()))?;
    fs::rename(&temporary, path)
        .with_context(|| format!("failed to publish Bifrost PID file {}", path.display()))
}

fn remove_pid_file(path: &Path) -> Result<()> {
    if path.exists() {
        fs::remove_file(path)
            .with_context(|| format!("failed to remove Bifrost PID file {}", path.display()))?;
    }
    Ok(())
}

/// Opens the gateway log file used by detached Bifrost processes.
fn gateway_log_file(path: &Path) -> Result<std::fs::File> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create gateway log dir {}", parent.display()))?;
    }

    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("failed to open gateway log {}", path.display()))
}

/// Reads a bounded tail from the detached Bifrost log for startup diagnostics.
fn read_log_tail(path: &Path) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    let lines = text.lines().collect::<Vec<_>>();
    let start = lines.len().saturating_sub(80);
    Some(lines[start..].join("\n"))
}

/// Finds Bifrost process IDs listening on a port and filters out unrelated
/// processes that may also be reported by `lsof`.
fn bifrost_process_ids_on_port(port: &str) -> Result<Vec<u32>> {
    #[cfg(windows)]
    return bifrost_process_ids_on_port_windows(port);

    #[cfg(not(windows))]
    bifrost_process_ids_on_port_unix(port)
}

/// Returns a human-readable process listing for a contested gateway port.
fn port_owner_diagnostics(port: &str) -> Result<String> {
    let mut owners = Vec::new();
    for process_id in all_process_ids_on_port(port)? {
        let command = process_command(process_id).unwrap_or_default();
        owners.push(format!("pid={process_id} command={command}"));
    }
    Ok(owners.join("\n"))
}

fn all_process_ids_on_port(port: &str) -> Result<Vec<u32>> {
    #[cfg(windows)]
    return all_process_ids_on_port_windows(port);

    #[cfg(not(windows))]
    all_process_ids_on_port_unix(port)
}

#[cfg(not(windows))]
fn all_process_ids_on_port_unix(port: &str) -> Result<Vec<u32>> {
    let output = Command::new("lsof")
        .args(["-nP", &format!("-iTCP:{port}"), "-sTCP:LISTEN", "-t"])
        .output()
        .context("failed to inspect local gateway port")?;
    Ok(parse_process_ids(&String::from_utf8_lossy(&output.stdout)))
}

#[cfg(windows)]
fn all_process_ids_on_port_windows(port: &str) -> Result<Vec<u32>> {
    let script = format!(
        "Get-NetTCPConnection -LocalPort {port} -State Listen -ErrorAction SilentlyContinue | Select-Object -ExpandProperty OwningProcess"
    );
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output()
        .context("failed to inspect local gateway port")?;
    Ok(parse_process_ids(&String::from_utf8_lossy(&output.stdout)))
}

#[cfg(not(windows))]
fn bifrost_process_ids_on_port_unix(port: &str) -> Result<Vec<u32>> {
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

#[cfg(windows)]
fn bifrost_process_ids_on_port_windows(port: &str) -> Result<Vec<u32>> {
    let script = format!(
        "Get-NetTCPConnection -LocalPort {port} -State Listen -ErrorAction SilentlyContinue | Select-Object -ExpandProperty OwningProcess"
    );
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output()
        .context("failed to inspect local gateway process")?;

    let mut process_ids = BTreeSet::new();
    for process_id in parse_process_ids(&String::from_utf8_lossy(&output.stdout)) {
        let command = process_command(process_id)?;
        if is_bifrost_command(&command) {
            process_ids.insert(process_id);
        }
    }

    Ok(process_ids.into_iter().collect())
}

/// Stops one owned Bifrost process using the platform's process command.
fn stop_process(process_id: u32) -> Result<std::process::ExitStatus> {
    #[cfg(windows)]
    {
        return Command::new("taskkill.exe")
            .args(["/PID", &process_id.to_string(), "/T", "/F"])
            .status()
            .with_context(|| format!("failed to stop Bifrost process {process_id}"));
    }

    #[cfg(not(windows))]
    {
        Command::new("kill")
            .arg(process_id.to_string())
            .status()
            .with_context(|| format!("failed to stop Bifrost process {process_id}"))
    }
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
    #[cfg(windows)]
    {
        let script = format!(
            "(Get-CimInstance Win32_Process -Filter 'ProcessId = {process_id}').ExecutablePath"
        );
        let output = Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .output()
            .with_context(|| format!("failed to inspect process {process_id}"))?;

        return Ok(String::from_utf8_lossy(&output.stdout).trim().to_string());
    }

    #[cfg(not(windows))]
    {
        let output = Command::new("ps")
            .args(["-p", &process_id.to_string(), "-o", "command="])
            .output()
            .with_context(|| format!("failed to inspect process {process_id}"))?;

        if !output.status.success() {
            return Ok(String::new());
        }

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }
}

/// Identifies whether a process command line belongs to Bifrost.
fn is_bifrost_command(command: &str) -> bool {
    let command = command.trim();
    let executable = if let Some(quoted) = command.strip_prefix('"') {
        quoted.split('"').next().unwrap_or_default()
    } else {
        command.split_whitespace().next().unwrap_or_default()
    };

    Path::new(executable)
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.to_ascii_lowercase().contains("bifrost"))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gateway_url_removes_trailing_slash() {
        let url = GatewayUrl::new("http://localhost:8080/");

        assert_eq!(url.as_str(), "http://localhost:8080");
        assert_eq!(url.port(), "8080");
    }

    #[test]
    fn gateway_url_defaults_port_when_omitted() {
        assert_eq!(GatewayUrl::new("http://localhost").port(), "8080");
        assert_eq!(GatewayUrl::new("http://localhost:8081").port(), "8081");
    }

    #[test]
    fn selects_existing_owned_bifrost_binary() {
        let root = env::temp_dir().join(format!("windie-command-path-test-{}", std::process::id()));
        let binary = root.join("bifrost-http");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(&binary, "").unwrap();

        let launcher = select_bifrost_launcher([binary.clone()], public_paths_for_test(&root));
        assert!(matches!(launcher, Some(BifrostLauncher::Binary { path, .. }) if path == binary));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn launcher_requires_an_owned_binary() {
        let root = env::temp_dir().join(format!(
            "windie-launcher-missing-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();

        let launcher = select_bifrost_launcher(
            [root.join("missing-bifrost-http")],
            public_paths_for_test(&root),
        );

        assert!(launcher.is_none());

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn parses_process_ids() {
        let process_ids = parse_process_ids("123\nnot-a-pid\n456\n");

        assert_eq!(process_ids, vec![123, 456]);
    }

    #[test]
    fn recognizes_bifrost_process_command() {
        assert!(is_bifrost_command(
            "/Users/peterbui/Library/Caches/bifrost/v2.0.0-prerelease1/bin/bifrost-http-0 -app-dir /Users/peterbui/.local/share/windie/bifrost/data -port 8080"
        ));
        assert!(is_bifrost_command(
            "/Users/peterbui/.windie/bifrost/bifrost-http -app-dir /Users/peterbui/.windie/bifrost/data -port 8080"
        ));
        assert!(!is_bifrost_command("python3 -m http.server 8080"));
    }

    #[test]
    fn accepts_stable_bifrost_release_manifest() {
        let root = env::temp_dir().join(format!(
            "windie-stable-manifest-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("release-manifest.txt"),
            "windie_version=v0.2.4\nbifrost_version=stable\n",
        )
        .unwrap();
        let binary = root.join("bifrost");
        std::fs::write(&binary, "").unwrap();

        assert!(validate_release_manifest(&binary).is_ok());

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_non_matching_versioned_bifrost_release_manifest() {
        let root = env::temp_dir().join(format!(
            "windie-mismatched-manifest-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("release-manifest.txt"),
            "windie_version=v0.2.4\nbifrost_version=v0.2.3\n",
        )
        .unwrap();
        let binary = root.join("bifrost");
        std::fs::write(&binary, "").unwrap();

        let error = validate_release_manifest(&binary).unwrap_err();
        assert!(error.to_string().contains("release mismatch"));

        let _ = std::fs::remove_dir_all(root);
    }

    fn public_paths_for_test(root: &Path) -> BifrostPaths {
        BifrostPaths {
            dir: root.join(".windie").join("bifrost"),
            app_dir: root.join(".windie").join("bifrost").join("data"),
            log_file: root
                .join(".windie")
                .join("bifrost")
                .join("windie-gateway.log"),
            pid_file: root.join(".windie").join("bifrost").join("bifrost.pid"),
        }
    }
}
