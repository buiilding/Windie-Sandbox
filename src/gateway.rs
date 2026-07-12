//! Bifrost gateway availability and lifecycle.
//!
//! This module checks whether the local Bifrost HTTP gateway is healthy and
//! starts or stops a gateway when explicitly requested.
//!
//! Startup uses public Bifrost launchers so sibling workspace checkouts remain
//! reference material rather than runtime dependencies.

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

const PUBLIC_BIFROST_DIR: &str = "bifrost";
const PUBLIC_BIFROST_DATA_DIR: &str = "data";
const PUBLIC_BIFROST_LOG_FILE: &str = "windie-gateway.log";
const PUBLIC_BIFROST_NPX_PACKAGE: &str = "@maximhq/bifrost";
const PUBLIC_BIFROST_DOCKER_IMAGE: &str = "maximhq/bifrost:latest";
const PUBLIC_BIFROST_DOCKER_NAME: &str = "windie-bifrost";
const BIFROST_PORT: &str = "8080";
const ENV_FILE_NAME: &str = ".env";
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
struct PublicBifrostPaths {
    dir: PathBuf,
    app_dir: PathBuf,
    log_file: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Concrete way Windie will start Bifrost.
enum BifrostLauncher {
    Npx(PublicBifrostPaths),
    Docker(PublicBifrostPaths),
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

        if stop_docker_container()? {
            self.wait_until_stopped().await?;
            return Ok(GatewayStop::Stopped);
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

    /// Spawns Bifrost with the first available launcher.
    ///
    /// The child process environment is intentionally cleared first. Bifrost
    /// receives only variables loaded from Windie's `.env` file so provider keys
    /// are explicit instead of inherited from the user's shell environment.
    fn start_process(&self) -> Result<()> {
        let launcher = find_bifrost_launcher()?;
        let environment = load_bifrost_environment()?;

        match launcher {
            BifrostLauncher::Npx(paths) => {
                start_npx_process(&paths, npx_process_environment(environment))
            }
            BifrostLauncher::Docker(paths) => start_docker_process(&paths, environment),
        }
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

/// Starts public Bifrost through `npx @maximhq/bifrost`.
fn start_npx_process(paths: &PublicBifrostPaths, environment: Vec<(String, String)>) -> Result<()> {
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

    Command::new("npx")
        .arg("-y")
        .arg(PUBLIC_BIFROST_NPX_PACKAGE)
        .arg("-app-dir")
        .arg(&paths.app_dir)
        .arg("-port")
        .arg(BIFROST_PORT)
        .current_dir(&paths.dir)
        .env_clear()
        .envs(environment)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .spawn()
        .context("failed to start public Bifrost with npx")?;

    Ok(())
}

/// Starts public Bifrost through the Docker image.
fn start_docker_process(
    paths: &PublicBifrostPaths,
    environment: Vec<(String, String)>,
) -> Result<()> {
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

    let _ = Command::new("docker")
        .args(["rm", "-f", PUBLIC_BIFROST_DOCKER_NAME])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    let mut command = Command::new("docker");
    command
        .arg("run")
        .arg("-d")
        .arg("--rm")
        .arg("--name")
        .arg(PUBLIC_BIFROST_DOCKER_NAME)
        .arg("-p")
        .arg(format!("{BIFROST_PORT}:{BIFROST_PORT}"))
        .arg("-v")
        .arg(format!("{}:/app/data", paths.app_dir.display()))
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));

    for (key, value) in environment {
        command.arg("-e").arg(format!("{key}={value}"));
    }

    let status = command
        .arg(PUBLIC_BIFROST_DOCKER_IMAGE)
        .status()
        .context("failed to start public Bifrost with Docker")?;
    if !status.success() {
        return Err(anyhow!("failed to start public Bifrost with Docker"));
    }

    Ok(())
}

/// Stops the named Docker container when Windie started Bifrost that way.
fn stop_docker_container() -> Result<bool> {
    if !command_exists("docker") {
        return Ok(false);
    }

    let inspect = Command::new("docker")
        .args([
            "inspect",
            "-f",
            "{{.State.Running}}",
            PUBLIC_BIFROST_DOCKER_NAME,
        ])
        .output()
        .context("failed to inspect Bifrost Docker container")?;
    if !inspect.status.success() {
        return Ok(false);
    }

    let running = String::from_utf8_lossy(&inspect.stdout).trim() == "true";
    if !running {
        return Ok(false);
    }

    let status = Command::new("docker")
        .args(["stop", PUBLIC_BIFROST_DOCKER_NAME])
        .status()
        .context("failed to stop Bifrost Docker container")?;
    if !status.success() {
        return Err(anyhow!("failed to stop Bifrost Docker container"));
    }

    Ok(true)
}

/// Finds the first Bifrost launcher available on this machine.
fn find_bifrost_launcher() -> Result<BifrostLauncher> {
    let public_paths = public_bifrost_paths()?;
    let command_paths = env::var_os("PATH")
        .map(|paths| env::split_paths(&paths).collect::<Vec<_>>())
        .unwrap_or_default();

    select_bifrost_launcher(command_paths, public_paths).ok_or_else(|| {
        anyhow!(
            "Bifrost launcher was not found. Install Node/npm for `npx {PUBLIC_BIFROST_NPX_PACKAGE}` or Docker for `{PUBLIC_BIFROST_DOCKER_IMAGE}`."
        )
    })
}

/// Selects the concrete launcher from explicit search inputs.
fn select_bifrost_launcher(
    command_paths: impl IntoIterator<Item = PathBuf>,
    public_paths: PublicBifrostPaths,
) -> Option<BifrostLauncher> {
    let command_paths = command_paths.into_iter().collect::<Vec<_>>();
    if command_exists_in_paths("npx", command_paths.iter().cloned()) {
        return Some(BifrostLauncher::Npx(public_paths));
    }

    if command_exists_in_paths("docker", command_paths) {
        return Some(BifrostLauncher::Docker(public_paths));
    }

    None
}

/// Builds the public Bifrost runtime paths under `~/.windie`.
fn public_bifrost_paths() -> Result<PublicBifrostPaths> {
    let Some(home) = env::var_os("HOME") else {
        return Err(anyhow!("HOME is not set"));
    };

    let dir = PathBuf::from(home).join(".windie").join(PUBLIC_BIFROST_DIR);
    let app_dir = dir.join(PUBLIC_BIFROST_DATA_DIR);
    let log_file = dir.join(PUBLIC_BIFROST_LOG_FILE);
    fs::create_dir_all(&app_dir)
        .with_context(|| format!("failed to create Bifrost app dir {}", app_dir.display()))?;

    Ok(PublicBifrostPaths {
        dir,
        app_dir,
        log_file,
    })
}

/// Returns whether a command exists on `PATH`.
fn command_exists(program: &str) -> bool {
    let Some(paths) = env::var_os("PATH") else {
        return false;
    };

    command_exists_in_paths(program, env::split_paths(&paths))
}

/// Checks a provided path list for one executable command name.
fn command_exists_in_paths(program: &str, paths: impl IntoIterator<Item = PathBuf>) -> bool {
    paths.into_iter().any(|path| path.join(program).is_file())
}

/// Adds only process-launch variables needed by the public Node/npm launcher.
///
/// Provider keys still come from Windie's explicit `.env` file. `PATH` lets the
/// `npx` shim find `node`, and `HOME` lets npm use its normal cache location.
fn npx_process_environment(mut environment: Vec<(String, String)>) -> Vec<(String, String)> {
    if let Some(path) = env::var_os("PATH").and_then(|value| value.into_string().ok()) {
        environment.push(("PATH".to_string(), path));
    }
    if let Some(home) = env::var_os("HOME").and_then(|value| value.into_string().ok()) {
        environment.push(("HOME".to_string(), home));
    }

    environment
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
    command.contains(PUBLIC_BIFROST_NPX_PACKAGE)
        || command.contains(PUBLIC_BIFROST_DOCKER_IMAGE)
        || command.contains(PUBLIC_BIFROST_DOCKER_NAME)
}

/// Loads the environment variables Windie explicitly gives to Bifrost.
///
/// Missing `.env` is allowed so the gateway can still start for development
/// without provider keys. Provider calls may fail later until keys are added.
fn load_bifrost_environment() -> Result<Vec<(String, String)>> {
    let Some(path) = find_env_file_path() else {
        return Ok(Vec::new());
    };

    let text = fs::read_to_string(&path)
        .with_context(|| format!("failed to read environment file {}", path.display()))?;

    parse_env_file(&text).with_context(|| format!("failed to parse {}", path.display()))
}

/// Finds Windie's provider-key environment file.
fn find_env_file_path() -> Option<PathBuf> {
    env_file_path().filter(|path| path.is_file())
}

/// Returns the only supported provider-key environment file path.
fn env_file_path() -> Option<PathBuf> {
    env::var_os("HOME").map(|home| PathBuf::from(home).join(".windie").join(ENV_FILE_NAME))
}

/// Parses simple KEY=VALUE lines from a `.env` file.
///
/// Empty lines and `#` comments are ignored. `export KEY=VALUE` is accepted.
/// Single or double quotes around the entire value are stripped.
fn parse_env_file(text: &str) -> Result<Vec<(String, String)>> {
    let mut values = Vec::new();

    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let line = line.strip_prefix("export ").unwrap_or(line).trim();
        let Some((key, value)) = line.split_once('=') else {
            return Err(anyhow!("invalid .env line {}", index + 1));
        };
        let key = key.trim();
        if key.is_empty() {
            return Err(anyhow!("empty .env key on line {}", index + 1));
        }

        values.push((key.to_string(), unquote_env_value(value.trim()).to_string()));
    }

    Ok(values)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gateway_url_removes_trailing_slash() {
        let url = GatewayUrl::new("http://localhost:8080/");

        assert_eq!(url.as_str(), "http://localhost:8080");
    }

    #[test]
    fn parses_env_file_values() {
        let values = parse_env_file(
            r#"
            # test launch environment
            WINDIE_TEST_KEY=alpha
            export WINDIE_SECOND_TEST_KEY='beta'
            WINDIE_THIRD_TEST_KEY="gamma"
            EMPTY=
            "#,
        )
        .unwrap();

        assert_eq!(
            values,
            vec![
                ("WINDIE_TEST_KEY".to_string(), "alpha".to_string()),
                ("WINDIE_SECOND_TEST_KEY".to_string(), "beta".to_string()),
                ("WINDIE_THIRD_TEST_KEY".to_string(), "gamma".to_string()),
                ("EMPTY".to_string(), "".to_string()),
            ]
        );
    }

    #[test]
    fn env_file_path_uses_windie_home_only() {
        let Some(home) = env::var_os("HOME") else {
            return;
        };

        assert_eq!(
            env_file_path(),
            Some(PathBuf::from(home).join(".windie").join(ENV_FILE_NAME))
        );
    }

    #[test]
    fn npx_process_environment_preserves_provider_keys() {
        let environment =
            npx_process_environment(vec![("WINDIE_TEST_KEY".to_string(), "alpha".to_string())]);

        assert!(environment.contains(&("WINDIE_TEST_KEY".to_string(), "alpha".to_string())));
    }

    #[test]
    fn npx_process_environment_adds_process_launch_variables() {
        let environment = npx_process_environment(Vec::new());

        if env::var_os("PATH").is_some() {
            assert!(environment.iter().any(|(key, _)| key == "PATH"));
        }
        if env::var_os("HOME").is_some() {
            assert!(environment.iter().any(|(key, _)| key == "HOME"));
        }
    }

    #[test]
    fn rejects_invalid_env_file_line() {
        let error = parse_env_file("OPENAI_API_KEY").unwrap_err();

        assert!(error.to_string().contains("invalid .env line 1"));
    }

    #[test]
    fn detects_command_in_path_list() {
        let root = env::temp_dir().join(format!("windie-command-path-test-{}", std::process::id()));
        let bin_dir = root.join("bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        std::fs::write(bin_dir.join("npx"), "").unwrap();

        assert!(command_exists_in_paths("npx", [bin_dir]));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn launcher_uses_npx_even_when_local_bifrost_exists() {
        let root = env::temp_dir().join(format!("windie-launcher-npx-test-{}", std::process::id()));
        let bifrost_dir = root.join("bifrost");
        let binary = bifrost_dir.join("tmp").join("bifrost-http");
        let command_dir = root.join("bin");
        std::fs::create_dir_all(binary.parent().unwrap()).unwrap();
        std::fs::create_dir_all(&command_dir).unwrap();
        std::fs::write(&binary, "").unwrap();
        std::fs::write(command_dir.join("npx"), "").unwrap();

        let launcher =
            select_bifrost_launcher([command_dir], public_paths_for_test(&root)).unwrap();

        assert!(matches!(launcher, BifrostLauncher::Npx(_)));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn launcher_uses_npx_when_available() {
        let root = env::temp_dir().join(format!(
            "windie-launcher-npx-only-test-{}",
            std::process::id()
        ));
        let command_dir = root.join("bin");
        std::fs::create_dir_all(&command_dir).unwrap();
        std::fs::write(command_dir.join("npx"), "").unwrap();

        let launcher =
            select_bifrost_launcher([command_dir], public_paths_for_test(&root)).unwrap();

        assert!(matches!(launcher, BifrostLauncher::Npx(_)));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn launcher_uses_docker_when_local_and_npx_are_missing() {
        let root = env::temp_dir().join(format!(
            "windie-launcher-docker-test-{}",
            std::process::id()
        ));
        let command_dir = root.join("bin");
        std::fs::create_dir_all(&command_dir).unwrap();
        std::fs::write(command_dir.join("docker"), "").unwrap();

        let launcher =
            select_bifrost_launcher([command_dir], public_paths_for_test(&root)).unwrap();

        assert!(matches!(launcher, BifrostLauncher::Docker(_)));

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
            "npx -y @maximhq/bifrost -app-dir /Users/peterbui/.windie/bifrost/data"
        ));
        assert!(is_bifrost_command(
            "docker run --name windie-bifrost -p 8080:8080 maximhq/bifrost:latest"
        ));
        assert!(!is_bifrost_command("python3 -m http.server 8080"));
    }

    fn public_paths_for_test(root: &Path) -> PublicBifrostPaths {
        PublicBifrostPaths {
            dir: root.join(".windie").join("bifrost"),
            app_dir: root.join(".windie").join("bifrost").join("data"),
            log_file: root
                .join(".windie")
                .join("bifrost")
                .join("windie-gateway.log"),
        }
    }
}
