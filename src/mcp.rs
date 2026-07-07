//! Minimal MCP stdio client.
//!
//! This module owns the protocol boundary for approved MCP providers. It runs a
//! configured command, speaks line-delimited JSON-RPC 2.0 over stdin/stdout,
//! performs the MCP initialize handshake, and exposes the tool operations
//! Windie needs now: `tools/list`, short-lived `tools/call`, and persistent
//! provider sessions for API-owned runtime tools.

use std::collections::HashMap;
use std::env;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use serde::Deserialize;
use serde_json::{Value, json};

const MCP_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const MCP_IDLE_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const MCP_IDLE_REAPER_INTERVAL: Duration = Duration::from_secs(30);
const MCP_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);
const MCP_SHUTDOWN_RETRY_DELAY: Duration = Duration::from_millis(750);
const MCP_SHUTDOWN_RETRIES: usize = 4;
const MCP_STDERR_MAX_BYTES: usize = 16 * 1024;

/// Process command for one approved MCP provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct McpCommand {
    pub program: &'static str,
    pub args: &'static [&'static str],
    pub env: &'static [McpEnv],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Environment variable assigned before Windie starts an MCP provider.
pub struct McpEnv {
    pub key: &'static str,
    pub value: McpEnvValue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Static environment value shape for approved MCP provider commands.
pub enum McpEnvValue {
    WindieDataDir(&'static str),
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
/// Tool entry returned by MCP `tools/list`.
pub struct McpTool {
    pub name: String,
    pub description: String,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
    #[serde(default)]
    pub annotations: Option<McpToolAnnotations>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
/// Optional MCP tool annotations used by Windie policy/UI metadata.
pub struct McpToolAnnotations {
    #[serde(rename = "readOnlyHint")]
    pub read_only_hint: Option<bool>,
}

#[derive(Debug, Deserialize)]
/// Result shape for MCP `tools/list`.
struct McpToolsList {
    tools: Vec<McpTool>,
}

#[derive(Debug, Deserialize)]
/// JSON-RPC response envelope from an MCP server.
struct JsonRpcResponse {
    id: Value,
    #[serde(default)]
    result: Option<Value>,
    #[serde(default)]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Deserialize)]
/// JSON-RPC error body from an MCP server.
struct JsonRpcError {
    code: i64,
    message: String,
}

/// Lists tools from one approved MCP stdio provider.
pub fn list_tools(command: McpCommand) -> Result<Vec<McpTool>> {
    let mut session = McpSession::start(command)?;
    let result = session.call("tools/list", None)?;
    let list = serde_json::from_value::<McpToolsList>(result)
        .context("failed to decode MCP tools/list response")?;

    Ok(list.tools)
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

/// Owns persistent MCP provider sessions for one registry/client.
///
/// The persistent session is keyed by provider ID, not command string, because
/// provider identity is the routing boundary used by attached tool schemas. The
/// session is stopped after a period of inactivity, and stopping the session
/// also runs the provider shutdown hook when one is configured.
#[derive(Clone)]
pub struct McpSessionPool {
    sessions: Arc<Mutex<PersistentMcpSessions>>,
}

impl std::fmt::Debug for McpSessionPool {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("McpSessionPool")
            .finish_non_exhaustive()
    }
}

impl McpSessionPool {
    /// Creates a registry-owned persistent MCP session pool.
    pub fn new() -> Self {
        let sessions = Arc::new(Mutex::new(PersistentMcpSessions::default()));
        spawn_idle_reaper(Arc::clone(&sessions));

        Self { sessions }
    }

    /// Calls one MCP provider tool through this pool's persistent session.
    pub fn call_tool(
        &self,
        provider_id: &str,
        command: McpCommand,
        shutdown_command: Option<McpCommand>,
        name: &str,
        arguments: Value,
    ) -> Result<Value> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| anyhow!("persistent MCP session manager is poisoned"))?;

        sessions.call_tool(provider_id, command, shutdown_command, name, arguments)
    }
}

impl Default for McpSessionPool {
    fn default() -> Self {
        Self::new()
    }
}

/// Starts a small background loop that stops idle persistent MCP sessions.
fn spawn_idle_reaper(sessions: Arc<Mutex<PersistentMcpSessions>>) {
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(MCP_IDLE_REAPER_INTERVAL);
            let Ok(mut sessions) = sessions.lock() else {
                break;
            };
            sessions.stop_idle_sessions(MCP_IDLE_TIMEOUT);
        }
    });
}

#[derive(Default)]
/// Process-wide owner for persistent MCP provider sessions.
struct PersistentMcpSessions {
    sessions: HashMap<String, PersistentMcpSession>,
}

impl PersistentMcpSessions {
    /// Calls one tool through the persistent session for `provider_id`.
    fn call_tool(
        &mut self,
        provider_id: &str,
        command: McpCommand,
        shutdown_command: Option<McpCommand>,
        name: &str,
        arguments: Value,
    ) -> Result<Value> {
        self.ensure_session(provider_id, command, shutdown_command)?;

        let result = {
            let session = self
                .sessions
                .get_mut(provider_id)
                .ok_or_else(|| anyhow!("persistent MCP session was not started: {provider_id}"))?;
            session.last_used_at = Instant::now();
            session.session.call(
                "tools/call",
                Some(json!({
                    "name": name,
                    "arguments": arguments
                })),
            )
        };

        match result {
            Ok(result) => {
                if let Some(session) = self.sessions.get_mut(provider_id) {
                    session.last_used_at = Instant::now();
                }
                Ok(result)
            }
            Err(error) => {
                self.stop_session(provider_id);
                Err(error)
            }
        }
    }

    /// Ensures a matching persistent MCP session exists for one provider.
    fn ensure_session(
        &mut self,
        provider_id: &str,
        command: McpCommand,
        shutdown_command: Option<McpCommand>,
    ) -> Result<()> {
        let command_changed = self.sessions.get(provider_id).is_some_and(|session| {
            session.command != command || session.shutdown_command != shutdown_command
        });
        if command_changed {
            self.stop_session(provider_id);
        }
        if self.sessions.contains_key(provider_id) {
            return Ok(());
        }

        let session = McpSession::start(command)?;
        self.sessions.insert(
            provider_id.to_string(),
            PersistentMcpSession {
                command,
                shutdown_command,
                session,
                last_used_at: Instant::now(),
            },
        );

        Ok(())
    }

    /// Stops sessions that have not received a call within `idle_timeout`.
    fn stop_idle_sessions(&mut self, idle_timeout: Duration) {
        let now = Instant::now();
        let provider_ids = self
            .sessions
            .iter()
            .filter_map(|(provider_id, session)| {
                if now.duration_since(session.last_used_at) >= idle_timeout {
                    Some(provider_id.clone())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();

        for provider_id in provider_ids {
            self.stop_session(&provider_id);
        }
    }

    /// Stops one persistent session and runs its provider shutdown hook.
    fn stop_session(&mut self, provider_id: &str) {
        let Some(session) = self.sessions.remove(provider_id) else {
            return;
        };
        let shutdown_command = session.shutdown_command;
        drop(session);

        run_shutdown_best_effort(shutdown_command);
    }
}

/// Runtime state for one persistent MCP provider.
struct PersistentMcpSession {
    command: McpCommand,
    shutdown_command: Option<McpCommand>,
    session: McpSession,
    last_used_at: Instant,
}

/// One short-lived stdio MCP session.
struct McpSession {
    command: McpCommand,
    child: Child,
    stdin: ChildStdin,
    stdout_lines: Receiver<Result<String, String>>,
    stderr: Arc<Mutex<Vec<u8>>>,
    next_id: u64,
}

impl McpSession {
    /// Starts the provider process and completes the MCP initialize handshake.
    fn start(command: McpCommand) -> Result<Self> {
        let mut process = Command::new(command.program);
        configure_process(&mut process, command)?;
        let mut child = process
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("failed to start MCP provider: {}", command.program))?;
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
            command,
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
    fn call(&mut self, method: &str, params: Option<Value>) -> Result<Value> {
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
        loop {
            let line = match self.stdout_lines.recv_timeout(MCP_REQUEST_TIMEOUT) {
                Ok(Ok(line)) => line,
                Ok(Err(error)) => {
                    return Err(self.error_with_stderr(format!("{error} for {method}")));
                }
                Err(RecvTimeoutError::Timeout) => {
                    return Err(self.error_with_stderr(format!(
                        "MCP provider timed out during {method}: {}",
                        self.command.program
                    )));
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(self.error_with_stderr(format!(
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
                return Err(self.error_with_stderr(format!(
                    "MCP error {} from {method}: {}",
                    error.code, error.message
                )));
            }

            return response.result.ok_or_else(|| {
                self.error_with_stderr(format!("MCP response for {method} did not include result"))
            });
        }
    }

    /// Adds captured provider stderr to MCP protocol/process errors.
    fn error_with_stderr(&self, message: String) -> anyhow::Error {
        let stderr = captured_stderr(&self.stderr);
        if stderr.trim().is_empty() {
            anyhow!(message)
        } else {
            anyhow!("{message}\nstderr:\n{stderr}")
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

/// Runs one shutdown command with a small timeout.
fn run_shutdown_command(command: McpCommand) -> Result<()> {
    let mut process = Command::new(command.program);
    configure_process(&mut process, command)?;
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
fn configure_process(process: &mut Command, command: McpCommand) -> Result<()> {
    process.args(command.args);
    for variable in command.env {
        process.env(variable.key, resolve_env_value(variable.value)?);
    }

    Ok(())
}

/// Resolves an MCP environment value at process-start time.
fn resolve_env_value(value: McpEnvValue) -> Result<String> {
    match value {
        McpEnvValue::WindieDataDir(relative_path) => Ok(windie_data_dir()
            .join(relative_path)
            .to_string_lossy()
            .into_owned()),
    }
}

/// Returns Windie's per-user data directory.
fn windie_data_dir() -> PathBuf {
    env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".windie")
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
        let value = resolve_env_value(McpEnvValue::WindieDataDir("mcp/desktop-commander")).unwrap();

        assert!(value.ends_with(".windie/mcp/desktop-commander"));
    }
}
