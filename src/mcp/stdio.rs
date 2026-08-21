//! Local stdio MCP transport.
//!
//! This module owns child-process startup, line-delimited JSON-RPC I/O,
//! bounded diagnostics, preparation, and best-effort shutdown.

use std::env;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

use anyhow::{Context, Result, anyhow};
use serde_json::{Value, json};

use super::protocol::{
    JsonRpcResponse, McpArgument, McpCommand, McpEnvValue, McpOwnedCommand, McpRequestTimeout,
    McpTool, McpToolsList, request_timeout_for_method,
};
use crate::local;

const MCP_PACKAGE_PREPARATION_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const MCP_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);
const MCP_SHUTDOWN_RETRY_DELAY: Duration = Duration::from_millis(750);
const MCP_SHUTDOWN_RETRIES: usize = 4;
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;
const MCP_STDERR_MAX_BYTES: usize = 16 * 1024;

impl McpArgument {
    /// Resolves this argument into the value passed to the provider process.
    fn resolve(self) -> String {
        match self {
            Self::Literal(value) => value.to_string(),
            Self::WindieDataDir(relative_path) => windie_data_dir()
                .join(relative_path)
                .to_string_lossy()
                .into_owned(),
        }
    }
}

/// Lists tools from one approved MCP stdio provider.
pub fn list_tools(command: McpCommand) -> Result<Vec<McpTool>> {
    let mut session = McpSession::start(command)?;
    let result = session.call("tools/list", None)?;
    let list = serde_json::from_value::<McpToolsList>(result)
        .context("failed to decode MCP tools/list response")?;

    Ok(list.tools)
}

/// Runs one provider package preparation command before MCP startup.
///
/// Package runners such as `npx` and `uvx` may download a provider package on
/// their first invocation. That download is intentionally handled outside the
/// MCP initialize timeout, so a normal package install does not look like a
/// failed MCP server. The command's stderr is retained when preparation fails
/// because package managers commonly report the useful diagnostic there.
pub fn run_preparation_command(command: McpCommand) -> Result<()> {
    let mut process = configure_process(command)?;
    let mut child = process
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| {
            format!(
                "failed to start provider package preparation command: {}",
                command.program
            )
        })?;
    let stderr = child.stderr.take().map(|mut stream| {
        std::thread::spawn(move || {
            let mut output = String::new();
            let _ = stream.read_to_string(&mut output);
            output
        })
    });
    let started = Instant::now();

    let status = loop {
        if let Some(status) = child
            .try_wait()
            .context("failed to wait for provider package preparation")?
        {
            break status;
        }
        if started.elapsed() >= MCP_PACKAGE_PREPARATION_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            if let Some(stderr) = stderr {
                let _ = stderr.join();
            }
            return Err(anyhow!(
                "provider package preparation timed out after {}s: {}",
                MCP_PACKAGE_PREPARATION_TIMEOUT.as_secs(),
                command.program
            ));
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let stderr = stderr
        .map(|reader| reader.join().unwrap_or_default())
        .unwrap_or_default();

    if !status.success() {
        let diagnostics = stderr.trim();
        if diagnostics.is_empty() {
            return Err(anyhow!(
                "provider package preparation failed with {status}: {}",
                command.program
            ));
        }
        return Err(anyhow!(
            "provider package preparation failed with {status}: {}\nstderr:\n{diagnostics}",
            command.program
        ));
    }

    Ok(())
}

/// Runs a package-owned preparation command before MCP startup.
pub fn run_owned_preparation_command(command: McpOwnedCommand) -> Result<()> {
    let command_name = command.program.clone();
    let mut process = configure_owned_process(command)?;
    let mut child = process
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| {
            format!("failed to start provider package preparation command: {command_name}")
        })?;
    let stderr = child.stderr.take().map(|mut stream| {
        std::thread::spawn(move || {
            let mut output = String::new();
            let _ = stream.read_to_string(&mut output);
            output
        })
    });
    let started = Instant::now();

    let status = loop {
        if let Some(status) = child
            .try_wait()
            .context("failed to wait for provider package preparation")?
        {
            break status;
        }
        if started.elapsed() >= MCP_PACKAGE_PREPARATION_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            if let Some(stderr) = stderr {
                let _ = stderr.join();
            }
            return Err(anyhow!(
                "provider package preparation timed out after {}s: {}",
                MCP_PACKAGE_PREPARATION_TIMEOUT.as_secs(),
                command_name
            ));
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let stderr = stderr
        .map(|reader| reader.join().unwrap_or_default())
        .unwrap_or_default();

    if !status.success() {
        let diagnostics = stderr.trim();
        if diagnostics.is_empty() {
            return Err(anyhow!(
                "provider package preparation failed with {status}: {command_name}"
            ));
        }
        return Err(anyhow!(
            "provider package preparation failed with {status}: {command_name}\nstderr:\n{diagnostics}"
        ));
    }

    Ok(())
}

/// Lists tools with a provider-specific cleanup hook after the MCP process
/// exits.
///
/// Some MCP commands are only a proxy to a separate daemon. CUA is the current
/// example: `cua-driver mcp` exits after `tools/list`, but the CUA daemon may
/// remain alive. This helper keeps catalog reads live while still coupling
/// provider-specific cleanup to the end of the short-lived MCP session.
pub fn list_tools_with_shutdown(
    command: McpCommand,
    shutdown_command: Option<McpCommand>,
) -> Result<Vec<McpTool>> {
    let result = {
        let mut session = McpSession::start(command)?;
        let result = session.call("tools/list", None)?;
        serde_json::from_value::<McpToolsList>(result)
            .context("failed to decode MCP tools/list response")
            .map(|list| list.tools)
    };

    run_shutdown_best_effort(shutdown_command);

    result
}

/// Lists tools from a packaged local MCP and runs its optional cleanup command.
pub(super) fn list_tools_with_owned_shutdown(
    command: McpOwnedCommand,
    shutdown_command: Option<McpOwnedCommand>,
) -> Result<Vec<McpTool>> {
    let result = {
        let mut session = McpSession::start_owned(command)?;
        let result = session.call("tools/list", None)?;
        serde_json::from_value::<McpToolsList>(result)
            .context("failed to decode MCP tools/list response")
            .map(|list| list.tools)
    };

    run_owned_shutdown_best_effort(shutdown_command);
    result
}

/// Calls one MCP provider tool and returns the raw MCP result value.
pub fn call_tool(command: McpCommand, name: &str, arguments: Value) -> Result<Value> {
    let mut session = McpSession::start(command)?;

    session.call(
        "tools/call",
        Some(json!({
            "name": name,
            "arguments": arguments
        })),
    )
}

/// Calls one MCP provider tool and runs a provider-specific cleanup hook when
/// the short-lived MCP process exits.
pub fn call_tool_with_shutdown(
    command: McpCommand,
    shutdown_command: Option<McpCommand>,
    name: &str,
    arguments: Value,
) -> Result<Value> {
    let result = {
        let mut session = McpSession::start(command)?;
        session.call(
            "tools/call",
            Some(json!({
                "name": name,
                "arguments": arguments
            })),
        )
    };

    run_shutdown_best_effort(shutdown_command);

    result
}

/// Calls one packaged local MCP tool and runs its optional cleanup command.
pub(super) fn call_tool_with_owned_shutdown(
    command: McpOwnedCommand,
    shutdown_command: Option<McpOwnedCommand>,
    name: &str,
    arguments: Value,
) -> Result<Value> {
    let result = {
        let mut session = McpSession::start_owned(command)?;
        session.call(
            "tools/call",
            Some(json!({
                "name": name,
                "arguments": arguments
            })),
        )
    };

    run_owned_shutdown_best_effort(shutdown_command);
    result
}

/// One short-lived stdio MCP session.
pub(super) struct McpSession {
    command_name: String,
    child: Child,
    stdin: ChildStdin,
    stdout_lines: Receiver<Result<String, String>>,
    stderr: Arc<Mutex<Vec<u8>>>,
    next_id: u64,
}

impl McpSession {
    /// Starts the provider process and completes the MCP initialize handshake.
    pub(super) fn start(command: McpCommand) -> Result<Self> {
        let command_name = command.program.to_string();
        let process = configure_process(command)?;
        Self::start_with_process(process, command_name)
    }

    /// Starts a packaged provider process and completes the MCP handshake.
    pub(super) fn start_owned(command: McpOwnedCommand) -> Result<Self> {
        let command_name = command.program.clone();
        let process = configure_owned_process(command)?;
        Self::start_with_process(process, command_name)
    }

    /// Starts the protocol session from an already configured child process.
    fn start_with_process(mut process: Command, command_name: String) -> Result<Self> {
        let mut child = process
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| anyhow!("failed to start MCP provider {}: {error}", command_name))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("failed to open MCP stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("failed to open MCP stdout"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| anyhow!("failed to open MCP stderr"))?;
        let mut session = Self {
            command_name,
            child,
            stdin,
            stdout_lines: spawn_stdout_reader(stdout),
            stderr: spawn_stderr_reader(stderr),
            next_id: 0,
        };

        session.call(
            "initialize",
            Some(json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {
                    "name": "windie",
                    "version": env!("CARGO_PKG_VERSION")
                }
            })),
        )?;
        session.notify("notifications/initialized", None)?;

        Ok(session)
    }

    /// Sends one JSON-RPC request and waits for the matching response.
    pub(super) fn call(&mut self, method: &str, params: Option<Value>) -> Result<Value> {
        self.next_id += 1;
        let request_id = self.next_id;
        let mut request = json!({
            "jsonrpc": "2.0",
            "id": request_id,
            "method": method,
        });
        if let Some(params) = params {
            request["params"] = params;
        }

        self.write_json(&request)?;
        self.read_response(request_id, method)
    }

    /// Sends one JSON-RPC notification.
    fn notify(&mut self, method: &str, params: Option<Value>) -> Result<()> {
        let mut notification = json!({
            "jsonrpc": "2.0",
            "method": method,
        });
        if let Some(params) = params {
            notification["params"] = params;
        }

        self.write_json(&notification)
    }

    /// Writes one JSON object as a line-delimited MCP message.
    fn write_json(&mut self, value: &Value) -> Result<()> {
        let serialized = serde_json::to_string(value).context("failed to encode MCP request")?;
        self.stdin
            .write_all(serialized.as_bytes())
            .context("failed to write MCP request")?;
        self.stdin
            .write_all(b"\n")
            .context("failed to finish MCP request")?;
        self.stdin.flush().context("failed to flush MCP request")
    }

    /// Reads JSON-RPC lines until the response matching `request_id` arrives.
    fn read_response(&mut self, request_id: u64, method: &str) -> Result<Value> {
        let timeout = request_timeout_for_method(method);
        loop {
            let line = match self.stdout_lines.recv_timeout(timeout) {
                Ok(Ok(line)) => line,
                Ok(Err(error)) => {
                    return Err(self.error_with_stderr(anyhow!("{error} for {method}")));
                }
                Err(RecvTimeoutError::Timeout) => {
                    return Err(self.error_with_stderr(
                        McpRequestTimeout::new(&self.command_name, method, timeout).into(),
                    ));
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(self.error_with_stderr(anyhow!(
                        "MCP provider stdout reader stopped before responding to {method}"
                    )));
                }
            };
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let response = match serde_json::from_str::<JsonRpcResponse>(trimmed) {
                Ok(response) => response,
                Err(_) => continue,
            };
            if response.id != request_id {
                continue;
            }
            if let Some(error) = response.error {
                return Err(self.error_with_stderr(anyhow!(
                    "MCP error {} from {method}: {}",
                    error.code,
                    error.message
                )));
            }

            return response.result.ok_or_else(|| {
                self.error_with_stderr(anyhow!("MCP response for {method} did not include result"))
            });
        }
    }

    /// Adds captured provider stderr to MCP protocol/process errors.
    fn error_with_stderr(&self, error: anyhow::Error) -> anyhow::Error {
        let stderr = captured_stderr(&self.stderr);
        if stderr.trim().is_empty() {
            error
        } else {
            error.context(format!("stderr:\n{stderr}"))
        }
    }
}

impl Drop for McpSession {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Runs a provider-specific shutdown command without failing the user-facing
/// operation that already completed.
fn run_shutdown_best_effort(command: Option<McpCommand>) {
    let Some(command) = command else {
        return;
    };
    for attempt in 0..MCP_SHUTDOWN_RETRIES {
        if attempt > 0 {
            std::thread::sleep(MCP_SHUTDOWN_RETRY_DELAY);
        }
        if run_shutdown_command(command).is_ok() {
            return;
        }
    }
}

/// Runs an owned package shutdown command without failing the completed call.
fn run_owned_shutdown_best_effort(command: Option<McpOwnedCommand>) {
    let Some(command) = command else {
        return;
    };
    for attempt in 0..MCP_SHUTDOWN_RETRIES {
        if attempt > 0 {
            std::thread::sleep(MCP_SHUTDOWN_RETRY_DELAY);
        }
        if run_owned_shutdown_command(command.clone()).is_ok() {
            return;
        }
    }
}

/// Runs one shutdown command with a small timeout.
fn run_shutdown_command(command: McpCommand) -> Result<()> {
    let mut process = configure_process(command)?;
    let mut child = process
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("failed to start MCP shutdown command: {}", command.program))?;
    let started = Instant::now();

    loop {
        if child
            .try_wait()
            .context("failed to wait for MCP shutdown command")?
            .is_some()
        {
            return Ok(());
        }
        if started.elapsed() >= MCP_SHUTDOWN_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            return Err(anyhow!(
                "MCP shutdown command timed out: {}",
                command.program
            ));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Runs one owned package shutdown command with a bounded wait.
fn run_owned_shutdown_command(command: McpOwnedCommand) -> Result<()> {
    let mut process = configure_owned_process(command.clone())?;
    let mut child = process
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("failed to start MCP shutdown command: {}", command.program))?;
    let started = Instant::now();

    loop {
        if child
            .try_wait()
            .context("failed to wait for MCP shutdown command")?
            .is_some()
        {
            return Ok(());
        }
        if started.elapsed() >= MCP_SHUTDOWN_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            return Err(anyhow!(
                "MCP shutdown command timed out: {}",
                command.program
            ));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Applies the static command definition to a spawned provider process.
fn configure_process(command: McpCommand) -> Result<Command> {
    let program = crate::managed_runtime::resolve_command(command.program)?;
    let command_path = crate::managed_runtime::path_with_command_parent(&program);
    let args = command
        .args
        .iter()
        .copied()
        .map(McpArgument::resolve)
        .collect::<Vec<_>>();
    let mut process = windows_command(program, &args);
    if let Some(path) = command_path {
        process.env("PATH", path);
    }
    for variable in command.env {
        process.env(variable.key, resolve_env_value(variable.value)?);
    }

    Ok(process)
}

/// Applies an installed package command without invoking a shell.
fn configure_owned_process(command: McpOwnedCommand) -> Result<Command> {
    let program = if PathBuf::from(&command.program).is_absolute()
        || command.program.contains(std::path::MAIN_SEPARATOR)
    {
        PathBuf::from(&command.program)
    } else {
        crate::managed_runtime::resolve_command(&command.program)?
    };
    let command_path = crate::managed_runtime::path_with_command_parent(&program);
    let mut process = windows_command(program, &command.args);
    if let Some(path) = command_path {
        process.env("PATH", path);
    }
    for (key, value) in command.env {
        process.env(key, value);
    }
    for (key, name, required) in command.secret_env {
        let value = local::env_value(&name)?.or_else(|| env::var(&name).ok());
        match value {
            Some(value) if !value.is_empty() => {
                process.env(key, value);
            }
            Some(_) | None if required => {
                return Err(anyhow::anyhow!(
                    "missing required environment variable: {name}"
                ));
            }
            Some(_) | None => {
                process.env_remove(key);
            }
        }
    }
    Ok(process)
}

/// Builds a process command that can launch both native executables and
/// Windows command shims such as npm's `npx.cmd`.
fn windows_command(program: PathBuf, args: &[String]) -> Command {
    #[cfg(target_os = "windows")]
    if program
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("cmd"))
    {
        let mut command_line = String::from("call ");
        command_line.push_str(&quote_windows_argument(&program.to_string_lossy()));
        for argument in args {
            command_line.push(' ');
            command_line.push_str(&quote_windows_argument(argument));
        }
        let command_processor = env::var_os("COMSPEC")
            .map(PathBuf::from)
            .filter(|path| path.is_file())
            .unwrap_or_else(|| PathBuf::from(r"C:\Windows\System32\cmd.exe"));
        let mut process = Command::new(command_processor);
        process.raw_arg("/D /S /C ");
        process.raw_arg(command_line);
        #[cfg(windows)]
        process.creation_flags(CREATE_NO_WINDOW);
        return process;
    }

    let mut process = Command::new(program);
    process.args(args);
    #[cfg(windows)]
    process.creation_flags(CREATE_NO_WINDOW);
    process
}

/// Quotes one static argument for the `cmd.exe /C` command line.
#[cfg(target_os = "windows")]
fn quote_windows_argument(argument: &str) -> String {
    format!("\"{}\"", argument.replace('"', "\\\""))
}

/// Resolves an MCP environment value at process-start time.
fn resolve_env_value(value: McpEnvValue) -> Result<String> {
    match value {
        McpEnvValue::WindieDataDir(relative_path) => Ok(windie_data_dir()
            .join(relative_path)
            .to_string_lossy()
            .into_owned()),
        McpEnvValue::Literal(value) => Ok(value.to_string()),
        McpEnvValue::UserEnv(name) => local::env_value(name)?
            .or_else(|| env::var(name).ok())
            .with_context(|| format!("missing required environment variable: {name}")),
    }
}

/// Returns Windie's per-user data directory.
fn windie_data_dir() -> PathBuf {
    local::windie_home_dir().unwrap_or_else(|_| PathBuf::from("."))
}

/// Reads provider stdout on a dedicated thread so protocol waits can time out.
fn spawn_stdout_reader(stdout: ChildStdout) -> Receiver<Result<String, String>> {
    let (sender, receiver) = mpsc::channel();

    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();

        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => {
                    let _ = sender.send(Err("MCP provider closed stdout".to_string()));
                    break;
                }
                Ok(_) => {
                    if sender.send(Ok(line.clone())).is_err() {
                        break;
                    }
                }
                Err(error) => {
                    let _ = sender.send(Err(format!("failed to read MCP response: {error}")));
                    break;
                }
            }
        }
    });

    receiver
}

/// Captures bounded provider stderr for later operation errors.
fn spawn_stderr_reader(stderr: impl Read + Send + 'static) -> Arc<Mutex<Vec<u8>>> {
    let captured = Arc::new(Mutex::new(Vec::new()));
    let captured_for_thread = Arc::clone(&captured);

    std::thread::spawn(move || {
        let mut reader = BufReader::new(stderr);
        let mut buffer = [0_u8; 1024];

        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(bytes_read) => {
                    let mut captured = match captured_for_thread.lock() {
                        Ok(captured) => captured,
                        Err(_) => break,
                    };
                    let remaining = MCP_STDERR_MAX_BYTES.saturating_sub(captured.len());
                    if remaining == 0 {
                        break;
                    }
                    captured.extend_from_slice(&buffer[..bytes_read.min(remaining)]);
                }
                Err(_) => break,
            }
        }
    });

    captured
}

/// Returns captured provider stderr as UTF-8 text, with a truncation marker.
fn captured_stderr(captured: &Arc<Mutex<Vec<u8>>>) -> String {
    let Ok(captured) = captured.lock() else {
        return String::new();
    };
    if captured.is_empty() {
        return String::new();
    }

    let mut text = String::from_utf8_lossy(&captured).to_string();
    if captured.len() >= MCP_STDERR_MAX_BYTES {
        text.push_str("\n[truncated]");
    }

    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captured_stderr_returns_provider_text() {
        let captured = Arc::new(Mutex::new(b"missing permission".to_vec()));
        assert_eq!(captured_stderr(&captured), "missing permission");
    }

    #[test]
    fn captured_stderr_marks_truncated_text() {
        let captured = Arc::new(Mutex::new(vec![b'x'; MCP_STDERR_MAX_BYTES]));
        assert!(captured_stderr(&captured).ends_with("\n[truncated]"));
    }

    #[test]
    fn windie_data_dir_env_value_resolves_under_user_home() {
        let _lock = crate::local::ENVIRONMENT_LOCK.lock().unwrap();
        let value = resolve_env_value(McpEnvValue::WindieDataDir("mcp/desktop-commander")).unwrap();
        assert!(
            std::path::Path::new(&value).ends_with(
                std::path::Path::new(".windie")
                    .join("mcp")
                    .join("desktop-commander")
            )
        );
    }

    #[test]
    fn literal_mcp_argument_resolves_without_changes() {
        assert_eq!(McpArgument::Literal("--no-slim").resolve(), "--no-slim");
    }

    #[test]
    fn windie_data_dir_mcp_argument_resolves_under_user_home() {
        let _lock = crate::local::ENVIRONMENT_LOCK.lock().unwrap();
        let value = McpArgument::WindieDataDir("mcp/chrome-devtools/profile").resolve();
        assert!(
            std::path::Path::new(&value).ends_with(
                std::path::Path::new(".windie")
                    .join("mcp")
                    .join("chrome-devtools")
                    .join("profile")
            )
        );
    }

    #[test]
    fn literal_env_value_resolves_directly() {
        assert_eq!(
            resolve_env_value(McpEnvValue::Literal("true")).unwrap(),
            "true"
        );
    }

    #[test]
    fn user_env_value_resolves_from_process_environment() {
        let (key, expected) = if cfg!(windows) {
            ("USERPROFILE", env::var("USERPROFILE").unwrap())
        } else {
            ("HOME", env::var("HOME").unwrap())
        };
        assert_eq!(
            resolve_env_value(McpEnvValue::UserEnv(key)).unwrap(),
            expected
        );
    }

    #[test]
    fn missing_user_env_value_returns_clear_error() {
        let error =
            resolve_env_value(McpEnvValue::UserEnv("WINDIE_TEST_MISSING_MCP_ENV")).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("missing required environment variable: WINDIE_TEST_MISSING_MCP_ENV")
        );
    }
}
