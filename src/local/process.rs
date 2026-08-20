//! Detached local component process management.
//!
//! This module owns the small amount of operating-system integration needed by
//! the CLI lifecycle commands: persistent PID files, detached stdout/stderr
//! logs, process identity checks, graceful API shutdown requests, and platform
//! process termination. It does not own component-specific runtime behavior.

use std::env;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

use anyhow::{Context, Result, anyhow};

use crate::local;

const STOP_TIMEOUT: Duration = Duration::from_secs(15);
const STOP_POLL_INTERVAL: Duration = Duration::from_millis(100);

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// One independently managed local Windie component.
pub enum ManagedComponent {
    Gateway,
    Api,
    Inspector,
    Tray,
}

impl ManagedComponent {
    /// Returns the stable human-readable component name used in CLI output.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Gateway => "gateway",
            Self::Api => "api",
            Self::Inspector => "inspector",
            Self::Tray => "tray",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Result of one detached component lifecycle action.
pub enum ProcessState {
    Started,
    AlreadyRunning,
    Stopped,
    NotRunning,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Structured result returned to the CLI output boundary.
pub struct ProcessReport {
    pub component: ManagedComponent,
    pub state: ProcessState,
    pub pid: Option<u32>,
    pub log_file: PathBuf,
}

/// Starts the detached Windie API child process.
pub fn start_api() -> Result<ProcessReport> {
    let executable = env::current_exe().context("failed to locate the Windie executable")?;
    start_detached(
        ManagedComponent::Api,
        &executable,
        &["api", "run"],
        &executable,
    )
}

/// Stops the Windie API process through its unauthenticated localhost shutdown
/// route, falling back to the recorded process when the route is unavailable.
pub fn stop_api() -> Result<ProcessReport> {
    let report = existing_report(ManagedComponent::Api)?;
    let Some(pid) = report.pid else {
        if endpoint_is_running(&crate::config::api_address(), "/api/health") {
            return Err(anyhow!(
                "Windie API is running without an owned PID file; refusing to remove it"
            ));
        }
        return Ok(report);
    };

    if request_api_shutdown() {
        wait_for_exit(pid)?;
        remove_pid_file(ManagedComponent::Api)?;
        return Ok(ProcessReport {
            state: ProcessState::Stopped,
            ..report
        });
    }

    stop_recorded_process(ManagedComponent::Api, pid, &report.log_file)
}

/// Starts the standalone Inspector server beside the Windie executable.
pub fn start_inspector() -> Result<ProcessReport> {
    let executable = inspector_executable()?;
    start_detached(ManagedComponent::Inspector, &executable, &[], &executable)
}

/// Starts the detached native tray process without starting any other Windie
/// component.
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub fn start_tray() -> Result<ProcessReport> {
    let executable = tray_executable()?;
    start_detached(
        ManagedComponent::Tray,
        &executable,
        &["tray", "run"],
        &executable,
    )
}

/// Resolves the tray executable for one supported desktop platform.
///
/// Installed macOS releases include the same binary inside `Windie Tray.app`.
/// Launching that copy gives `UNUserNotificationCenter` the application bundle
/// identity required for reliable notification click callbacks. Checkout and
/// development runs intentionally fall back to the current unbundled binary.
#[cfg(target_os = "macos")]
fn tray_executable() -> Result<PathBuf> {
    let current = env::current_exe().context("failed to locate the Windie executable")?;
    let directory = current
        .parent()
        .ok_or_else(|| anyhow!("Windie executable has no parent directory"))?;
    let bundled_tray = directory.join("Windie Tray.app/Contents/MacOS/windie");
    if bundled_tray.is_file() {
        Ok(bundled_tray)
    } else {
        Ok(current)
    }
}

/// Windows does not use the macOS application-bundle notification boundary.
#[cfg(target_os = "windows")]
fn tray_executable() -> Result<PathBuf> {
    env::current_exe().context("failed to locate the Windie executable")
}

/// Reports that the native tray is unavailable on platforms without a tray
/// implementation instead of spawning a child that exits immediately.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn start_tray() -> Result<ProcessReport> {
    Err(anyhow!(
        "Windie tray is currently supported on macOS and Windows only"
    ))
}

/// Stops the standalone Inspector server.
pub fn stop_inspector() -> Result<ProcessReport> {
    let report = existing_report(ManagedComponent::Inspector)?;
    let Some(pid) = report.pid else {
        if endpoint_is_running(&crate::config::inspector_address(), "/") {
            return Err(anyhow!(
                "Windie Inspector is running without an owned PID file; refusing to remove it"
            ));
        }
        return Ok(report);
    };

    if request_inspector_shutdown() {
        wait_for_exit(pid)?;
        remove_pid_file(ManagedComponent::Inspector)?;
        return Ok(ProcessReport {
            state: ProcessState::Stopped,
            ..report
        });
    }

    stop_recorded_process(ManagedComponent::Inspector, pid, &report.log_file)
}

/// Registers the foreground tray process so another Windie command can stop
/// it safely during uninstall.
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub fn register_tray() -> Result<()> {
    let current_pid = std::process::id();
    let pid_file = local::component_pid_file_path(ManagedComponent::Tray)?;
    if read_pid_file(&pid_file)? == Some(current_pid) {
        return Ok(());
    }

    let report = existing_report(ManagedComponent::Tray)?;
    if report.state == ProcessState::AlreadyRunning {
        return Err(anyhow!(
            "Windie tray is already running with PID {}",
            report.pid.unwrap_or_default()
        ));
    }

    write_pid_file(&pid_file, current_pid)
}

/// Removes the current tray PID file after the tray event loop exits.
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub fn unregister_tray() -> Result<()> {
    remove_pid_file(ManagedComponent::Tray)
}

/// Stops the Windie tray process when its PID file still identifies a tray.
pub fn stop_tray() -> Result<ProcessReport> {
    let report = existing_report(ManagedComponent::Tray)?;
    let Some(pid) = report.pid else {
        return Ok(report);
    };

    stop_recorded_process(ManagedComponent::Tray, pid, &report.log_file)
}

/// Stops and verifies every Windie process managed by this process boundary.
///
/// Bifrost remains behind `gateway.rs`, which owns its port and executable
/// identity checks. The uninstall operation invokes both boundaries before
/// removing any filesystem state.
pub fn stop_windie_processes() -> Result<Vec<ProcessReport>> {
    let mut reports = Vec::new();
    let mut failures = Vec::new();
    for stop in [
        stop_tray as fn() -> Result<ProcessReport>,
        stop_api,
        stop_inspector,
    ] {
        match stop() {
            Ok(report) => reports.push(report),
            Err(error) => failures.push(format!("{error:#}")),
        }
    }

    if failures.is_empty() {
        Ok(reports)
    } else {
        Err(anyhow!(
            "one or more Windie processes failed to stop: {}",
            failures.join("; ")
        ))
    }
}

/// Reads the complete persisted stdout/stderr log for one component.
pub fn read_output(component: ManagedComponent) -> Result<String> {
    let path = local::component_log_file_path(component)?;
    if !path.is_file() {
        return Ok(String::new());
    }

    fs::read_to_string(&path)
        .with_context(|| format!("failed to read {} output", component.as_str()))
}

/// Returns whether Windie currently owns a live process for one component.
///
/// This is intentionally narrower than an HTTP health check: it answers
/// whether the local lifecycle boundary can safely start or stop the component.
pub fn is_managed_component_running(component: ManagedComponent) -> Result<bool> {
    let pid_file = local::existing_component_pid_file_path(component)?;
    let Some(pid) = read_pid_file(&pid_file)? else {
        return Ok(false);
    };

    Ok(process_is_alive(pid) && process_matches_component(component, pid))
}

fn start_detached(
    component: ManagedComponent,
    executable: &Path,
    arguments: &[&str],
    expected_executable: &Path,
) -> Result<ProcessReport> {
    if !executable.is_file() {
        return Err(anyhow!(
            "{} executable was not found at {}",
            component.as_str(),
            executable.display()
        ));
    }

    let existing = existing_report(component)?;
    if let Some(pid) = existing.pid {
        return Ok(ProcessReport {
            state: ProcessState::AlreadyRunning,
            pid: Some(pid),
            ..existing
        });
    }

    let log_file = local::component_log_file_path(component)?;
    let pid_file = local::component_pid_file_path(component)?;
    if let Some(parent) = log_file.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let stdout = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_file)
        .with_context(|| format!("failed to open {} output", component.as_str()))?;
    let stderr = stdout
        .try_clone()
        .with_context(|| format!("failed to open {} output", component.as_str()))?;

    let mut command = Command::new(executable);
    command
        .args(arguments)
        .current_dir(executable.parent().unwrap_or_else(|| Path::new(".")))
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
    let child = command.spawn().with_context(|| {
        format!(
            "failed to start {} with {}",
            component.as_str(),
            expected_executable.display()
        )
    })?;
    write_pid_file(&pid_file, child.id())?;

    Ok(ProcessReport {
        component,
        state: ProcessState::Started,
        pid: Some(child.id()),
        log_file,
    })
}

fn existing_report(component: ManagedComponent) -> Result<ProcessReport> {
    let log_file = local::component_log_file_path(component)?;
    let pid_file = local::component_pid_file_path(component)?;
    let Some(pid) = read_pid_file(&pid_file)? else {
        return Ok(ProcessReport {
            component,
            state: ProcessState::NotRunning,
            pid: None,
            log_file,
        });
    };

    if process_is_alive(pid) && process_matches_component(component, pid) {
        return Ok(ProcessReport {
            component,
            state: ProcessState::AlreadyRunning,
            pid: Some(pid),
            log_file,
        });
    }

    remove_pid_file(component)?;
    Ok(ProcessReport {
        component,
        state: ProcessState::NotRunning,
        pid: None,
        log_file,
    })
}

fn stop_recorded_process(
    component: ManagedComponent,
    pid: u32,
    log_file: &Path,
) -> Result<ProcessReport> {
    let status = stop_process(pid)?;
    if !status.success() && process_is_alive(pid) {
        return Err(anyhow!(
            "failed to stop {} process {pid}",
            component.as_str()
        ));
    }
    wait_for_exit(pid)?;
    remove_pid_file(component)?;

    Ok(ProcessReport {
        component,
        state: ProcessState::Stopped,
        pid: Some(pid),
        log_file: log_file.to_path_buf(),
    })
}

fn request_api_shutdown() -> bool {
    request_loopback_shutdown(&crate::config::api_address(), "/api/shutdown")
}

/// Requests the Inspector's own shutdown route before any owned-PID fallback.
fn request_inspector_shutdown() -> bool {
    request_loopback_shutdown(&crate::config::inspector_address(), "/shutdown")
}

/// Sends one minimal local HTTP shutdown request without creating a blocking
/// async runtime inside the async CLI process.
fn request_loopback_shutdown(address: &str, path: &str) -> bool {
    loopback_request_succeeded(address, "POST", path)
}

/// Returns whether one local component endpoint accepts a health request.
fn endpoint_is_running(address: &str, path: &str) -> bool {
    loopback_request_succeeded(address, "GET", path)
}

/// Sends one bounded local HTTP request without constructing a nested Tokio
/// runtime inside the asynchronous CLI process.
fn loopback_request_succeeded(address: &str, method: &str, path: &str) -> bool {
    let Ok(socket) = address.to_socket_addrs().and_then(|mut addresses| {
        addresses
            .next()
            .ok_or_else(|| std::io::Error::other("shutdown address has no socket"))
    }) else {
        return false;
    };
    let Ok(mut stream) = TcpStream::connect_timeout(&socket, Duration::from_secs(2)) else {
        return false;
    };
    if stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .is_err()
        || stream
            .set_write_timeout(Some(Duration::from_secs(2)))
            .is_err()
    {
        return false;
    }

    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\nContent-Length: 0\r\n\r\n"
    );
    if stream.write_all(request.as_bytes()).is_err() {
        return false;
    }

    let mut response = [0_u8; 128];
    let Ok(length) = stream.read(&mut response) else {
        return false;
    };
    std::str::from_utf8(&response[..length])
        .ok()
        .is_some_and(|status| status.starts_with("HTTP/1.1 2") || status.starts_with("HTTP/1.0 2"))
}

fn inspector_executable() -> Result<PathBuf> {
    if let Some(path) = env::var_os("WINDIE_INSPECTOR_BIN") {
        return Ok(PathBuf::from(path));
    }

    let current = env::current_exe().context("failed to locate the Windie executable")?;
    let directory = current
        .parent()
        .ok_or_else(|| anyhow!("Windie executable has no parent directory"))?;
    Ok(directory.join(executable_name("windie-inspector")))
}

fn executable_name(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

fn process_matches_component(component: ManagedComponent, pid: u32) -> bool {
    let Ok(command) = process_command(pid) else {
        return false;
    };
    let expected = match component {
        ManagedComponent::Gateway => "bifrost",
        ManagedComponent::Api => "windie",
        ManagedComponent::Inspector => "windie-inspector",
        ManagedComponent::Tray => "windie",
    };
    let executable = command
        .trim_matches('"')
        .split_whitespace()
        .next()
        .and_then(|value| Path::new(value).file_stem())
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    if !executable.eq_ignore_ascii_case(expected) {
        return false;
    }

    if component == ManagedComponent::Tray {
        return process_command_line(pid)
            .map(|command| {
                command
                    .split_whitespace()
                    .any(|argument| argument.trim_matches('"') == "tray")
            })
            .unwrap_or(false);
    }

    true
}

fn process_is_alive(pid: u32) -> bool {
    process_command(pid).is_ok_and(|command| !command.trim().is_empty())
}

fn wait_for_exit(pid: u32) -> Result<()> {
    let started = Instant::now();
    while started.elapsed() < STOP_TIMEOUT {
        if !process_is_alive(pid) {
            return Ok(());
        }
        thread::sleep(STOP_POLL_INTERVAL);
    }

    Err(anyhow!(
        "process {pid} did not stop within {} seconds",
        STOP_TIMEOUT.as_secs()
    ))
}

fn read_pid_file(path: &Path) -> Result<Option<u32>> {
    if !path.is_file() {
        return Ok(None);
    }
    let text = fs::read_to_string(path)
        .with_context(|| format!("failed to read PID file {}", path.display()))?;
    text.trim()
        .parse::<u32>()
        .map(Some)
        .with_context(|| format!("invalid PID file {}", path.display()))
}

fn write_pid_file(path: &Path, pid: u32) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let temporary = path.with_extension("pid.tmp");
    fs::write(&temporary, format!("{pid}\n"))
        .with_context(|| format!("failed to write {}", path.display()))?;
    fs::rename(&temporary, path).with_context(|| format!("failed to publish {}", path.display()))
}

fn remove_pid_file(component: ManagedComponent) -> Result<()> {
    let path = local::component_pid_file_path(component)?;
    if path.exists() {
        fs::remove_file(&path).with_context(|| format!("failed to remove {}", path.display()))?;
    }
    Ok(())
}

fn stop_process(pid: u32) -> Result<std::process::ExitStatus> {
    #[cfg(windows)]
    {
        Command::new("taskkill.exe")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .status()
            .with_context(|| format!("failed to stop process {pid}"))
    }

    #[cfg(not(windows))]
    {
        Command::new("kill")
            .arg(pid.to_string())
            .status()
            .with_context(|| format!("failed to stop process {pid}"))
    }
}

fn process_command(pid: u32) -> Result<String> {
    #[cfg(windows)]
    {
        let script =
            format!("(Get-CimInstance Win32_Process -Filter 'ProcessId = {pid}').ExecutablePath");
        let output = Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .output()
            .with_context(|| format!("failed to inspect process {pid}"))?;
        return Ok(String::from_utf8_lossy(&output.stdout).trim().to_string());
    }

    #[cfg(not(windows))]
    {
        let output = Command::new("ps")
            .args(["-p", &pid.to_string(), "-o", "command="])
            .output()
            .with_context(|| format!("failed to inspect process {pid}"))?;
        if !output.status.success() {
            return Ok(String::new());
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }
}

fn process_command_line(pid: u32) -> Result<String> {
    #[cfg(windows)]
    {
        let script =
            format!("(Get-CimInstance Win32_Process -Filter 'ProcessId = {pid}').CommandLine");
        let output = Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .output()
            .with_context(|| format!("failed to inspect process {pid}"))?;
        return Ok(String::from_utf8_lossy(&output.stdout).trim().to_string());
    }

    #[cfg(not(windows))]
    {
        process_command(pid)
    }
}
