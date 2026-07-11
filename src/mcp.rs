//! Minimal MCP stdio client.
//!
//! This module owns the protocol boundary for approved MCP providers. It runs a
//! configured command, speaks line-delimited JSON-RPC 2.0 over stdin/stdout,
//! performs the MCP initialize handshake, and exposes the tool operations
//! Windie needs now: `tools/list`, short-lived `tools/call`, and persistent
//! provider sessions for API-owned runtime tools.

use std::collections::HashMap;
use std::env;
use std::fmt;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::run::RunCancellation;
use crate::{paths, provider_env};

const MCP_PROTOCOL_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const MCP_TOOL_CALL_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const MCP_IDLE_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const MCP_IDLE_REAPER_INTERVAL: Duration = Duration::from_secs(30);
const MCP_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);
const MCP_SHUTDOWN_RETRY_DELAY: Duration = Duration::from_millis(750);
const MCP_SHUTDOWN_RETRIES: usize = 4;
const MCP_STDERR_MAX_BYTES: usize = 16 * 1024;
const MCP_STDOUT_CHANNEL_CAPACITY: usize = 32;
const MCP_MAX_FRAME_BYTES: usize = 32 * 1024 * 1024;
const MCP_CANCELLATION_POLL_INTERVAL: Duration = Duration::from_millis(50);
const MCP_RUNTIME_ENVIRONMENT: &[&str] = &["PATH", "HOME", "TMPDIR", "TMP", "TEMP", "SystemRoot"];

#[derive(Debug, Clone, PartialEq, Eq)]
/// Typed timeout error for one MCP JSON-RPC request.
///
/// Tool execution code can detect this error after it crosses the MCP boundary
/// and turn approved `tools/call` timeouts into model-facing tool results. MCP
/// catalog and initialize callers still receive it as a normal operation error.
pub struct McpRequestTimeout {
    pub provider: String,
    pub method: String,
    pub timeout: Duration,
}

impl McpRequestTimeout {
    /// Builds a timeout error for one provider request.
    pub fn new(provider: impl Into<String>, method: impl Into<String>, timeout: Duration) -> Self {
        Self {
            provider: provider.into(),
            method: method.into(),
            timeout,
        }
    }

    /// Returns the timeout duration in milliseconds for structured tool output.
    pub fn timeout_ms(&self) -> u64 {
        self.timeout.as_millis().min(u128::from(u64::MAX)) as u64
    }

    /// Returns the timeout duration in whole seconds for human-facing errors.
    pub fn timeout_seconds(&self) -> u64 {
        self.timeout.as_secs()
    }
}

impl fmt::Display for McpRequestTimeout {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "MCP provider timed out during {} after {}s: {}",
            self.method,
            self.timeout_seconds(),
            self.provider
        )
    }
}

impl std::error::Error for McpRequestTimeout {}

/// Finds an MCP timeout in an anyhow error chain.
pub fn request_timeout_from_error(error: &anyhow::Error) -> Option<&McpRequestTimeout> {
    error.downcast_ref::<McpRequestTimeout>()
}

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
    /// Build the value from Windie's per-user data directory plus this suffix.
    WindieDataDir(&'static str),
    /// Use a fixed value owned by Windie's approved provider definition.
    Literal(&'static str),
    /// Copy a value from Windie's process environment into the provider child.
    ///
    /// This keeps provider secret names explicit. For example, Windie can read
    /// `BRIGHTDATA_API_TOKEN` from the user environment and pass it to a child
    /// process as that provider's expected `API_TOKEN`.
    UserEnv(&'static str),
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

#[derive(Debug, Clone, PartialEq)]
/// Provider-neutral representation of an MCP `tools/call` result.
///
/// MCP wire field names and content-block decoding stay in this module. Tool
/// providers receive this type and only own Windie's storage normalization.
pub struct McpToolResult {
    pub content: Vec<McpContentBlock>,
    pub structured_content: Option<Value>,
    pub is_error: bool,
    pub raw_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// One decoded MCP tool-result content block.
pub enum McpContentBlock {
    Text(String),
    Image { data: String, mime_type: String },
    Unsupported { kind: String },
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

/// Calls one MCP provider tool and returns its decoded result.
pub fn call_tool(command: McpCommand, name: &str, arguments: Value) -> Result<McpToolResult> {
    let mut session = McpSession::start(command)?;

    let result = session.call(
        "tools/call",
        Some(json!({
            "name": name,
            "arguments": arguments
        })),
    )?;
    decode_tool_result(result)
}

pub fn call_tool_with_shutdown_cancellable(
    command: McpCommand,
    shutdown_command: Option<McpCommand>,
    name: &str,
    arguments: Value,
    cancellation: &RunCancellation,
) -> Result<McpToolResult> {
    let result = {
        let mut session = McpSession::start(command)?;
        session.call_cancellable(
            "tools/call",
            Some(json!({
                "name": name,
                "arguments": arguments
            })),
            cancellation,
        )
    };

    run_shutdown_best_effort(shutdown_command);
    result.and_then(decode_tool_result)
}

/// Owns persistent MCP provider sessions for one registry/client.
///
/// The persistent session is keyed by provider ID, not command string, because
/// provider identity is the routing boundary used by attached tool schemas. The
/// session is stopped after a period of inactivity, and stopping the session
/// also runs the provider shutdown hook when one is configured.
#[derive(Clone)]
pub struct McpSessionPool {
    inner: Arc<McpSessionPoolInner>,
}

type PersistentMcpSessionSlot = Arc<Mutex<Option<PersistentMcpSession>>>;
type PersistentMcpSessionMap = HashMap<String, PersistentMcpSessionSlot>;

struct McpSessionPoolInner {
    sessions: Arc<Mutex<PersistentMcpSessionMap>>,
    reaper_shutdown: Sender<()>,
    reaper: Mutex<Option<JoinHandle<()>>>,
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
        let sessions = Arc::new(Mutex::new(HashMap::new()));
        let (reaper_shutdown, shutdown_receiver) = mpsc::channel();
        let reaper = spawn_idle_reaper(Arc::clone(&sessions), shutdown_receiver);

        Self {
            inner: Arc::new(McpSessionPoolInner {
                sessions,
                reaper_shutdown,
                reaper: Mutex::new(Some(reaper)),
            }),
        }
    }

    /// Calls one MCP provider tool through this pool's persistent session.
    #[cfg(test)]
    pub fn call_tool(
        &self,
        provider_id: &str,
        command: McpCommand,
        shutdown_command: Option<McpCommand>,
        name: &str,
        arguments: Value,
    ) -> Result<McpToolResult> {
        self.call_tool_cancellable(
            provider_id,
            command,
            shutdown_command,
            name,
            arguments,
            &RunCancellation::default(),
        )
    }

    pub fn call_tool_cancellable(
        &self,
        provider_id: &str,
        command: McpCommand,
        shutdown_command: Option<McpCommand>,
        name: &str,
        arguments: Value,
        cancellation: &RunCancellation,
    ) -> Result<McpToolResult> {
        let slot = self
            .inner
            .sessions
            .lock()
            .map_err(|_| anyhow!("persistent MCP session manager is poisoned"))?
            .entry(provider_id.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(None)))
            .clone();
        let result = call_persistent_tool(
            slot,
            command,
            shutdown_command,
            name,
            arguments,
            cancellation,
        )?;
        decode_tool_result(result)
    }
}

/// Decodes MCP-owned wire fields before results cross into tool-provider code.
pub(crate) fn decode_tool_result(result: Value) -> Result<McpToolResult> {
    let mut content = Vec::new();
    if let Some(blocks) = result.get("content").and_then(Value::as_array) {
        for block in blocks {
            match block.get("type").and_then(Value::as_str) {
                Some("text") => {
                    if let Some(text) = block.get("text").and_then(Value::as_str) {
                        content.push(McpContentBlock::Text(text.to_string()));
                    }
                }
                Some("image") => {
                    let data = block
                        .get("data")
                        .and_then(Value::as_str)
                        .ok_or_else(|| anyhow!("MCP image result did not include data"))?;
                    let mime_type = block
                        .get("mimeType")
                        .or_else(|| block.get("mime_type"))
                        .and_then(Value::as_str)
                        .unwrap_or("image/png");
                    content.push(McpContentBlock::Image {
                        data: data.to_string(),
                        mime_type: mime_type.to_string(),
                    });
                }
                Some(kind) => content.push(McpContentBlock::Unsupported {
                    kind: kind.to_string(),
                }),
                None => {}
            }
        }
    }

    Ok(McpToolResult {
        content,
        structured_content: result
            .get("structuredContent")
            .filter(|value| !value.is_null())
            .cloned(),
        is_error: result
            .get("isError")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        raw_json: result.to_string(),
    })
}

impl Drop for McpSessionPoolInner {
    fn drop(&mut self) {
        let _ = self.reaper_shutdown.send(());
        if let Ok(reaper) = self.reaper.get_mut()
            && let Some(reaper) = reaper.take()
        {
            let _ = reaper.join();
        }
    }
}

impl Default for McpSessionPool {
    fn default() -> Self {
        Self::new()
    }
}

/// Starts a small background loop that stops idle persistent MCP sessions.
fn spawn_idle_reaper(
    sessions: Arc<Mutex<PersistentMcpSessionMap>>,
    shutdown: Receiver<()>,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        while matches!(
            shutdown.recv_timeout(MCP_IDLE_REAPER_INTERVAL),
            Err(RecvTimeoutError::Timeout)
        ) {
            let Ok(session_slots) = sessions.lock().map(|sessions| {
                sessions
                    .iter()
                    .map(|(provider_id, slot)| (provider_id.clone(), Arc::clone(slot)))
                    .collect::<Vec<_>>()
            }) else {
                break;
            };
            for (provider_id, slot) in session_slots {
                let Ok(mut session) = slot.try_lock() else {
                    continue;
                };
                let is_idle = session
                    .as_ref()
                    .is_some_and(|session| session.last_used_at.elapsed() >= MCP_IDLE_TIMEOUT);
                if !is_idle {
                    continue;
                }
                let stopped = session.take();
                drop(session);
                if let Some(stopped) = stopped {
                    let shutdown_command = stopped.shutdown_command;
                    drop(stopped);
                    run_shutdown_best_effort(shutdown_command);
                }
                if let Ok(mut sessions) = sessions.lock()
                    && sessions
                        .get(&provider_id)
                        .is_some_and(|current| Arc::ptr_eq(current, &slot))
                {
                    sessions.remove(&provider_id);
                }
            }
        }
        stop_all_persistent_sessions(&sessions);
    })
}

fn stop_all_persistent_sessions(sessions: &Arc<Mutex<PersistentMcpSessionMap>>) {
    let Ok(session_slots) = sessions
        .lock()
        .map(|mut sessions| sessions.drain().map(|(_, slot)| slot).collect::<Vec<_>>())
    else {
        return;
    };
    for slot in session_slots {
        let stopped = slot.lock().ok().and_then(|mut session| session.take());
        if let Some(stopped) = stopped {
            let shutdown_command = stopped.shutdown_command;
            drop(stopped);
            run_shutdown_best_effort(shutdown_command);
        }
    }
}

fn call_persistent_tool(
    slot: PersistentMcpSessionSlot,
    command: McpCommand,
    shutdown_command: Option<McpCommand>,
    name: &str,
    arguments: Value,
    cancellation: &RunCancellation,
) -> Result<Value> {
    let mut session_slot = slot
        .lock()
        .map_err(|_| anyhow!("persistent MCP provider session is poisoned"))?;
    let command_changed = session_slot.as_ref().is_some_and(|session| {
        session.command != command || session.shutdown_command != shutdown_command
    });
    if command_changed {
        let stopped = session_slot.take();
        if let Some(stopped) = stopped {
            let old_shutdown = stopped.shutdown_command;
            drop(stopped);
            run_shutdown_best_effort(old_shutdown);
        }
    }
    if session_slot.is_none() {
        *session_slot = Some(PersistentMcpSession {
            command,
            shutdown_command,
            session: McpSession::start(command)?,
            last_used_at: Instant::now(),
        });
    }

    let session = session_slot
        .as_mut()
        .ok_or_else(|| anyhow!("persistent MCP session was not started"))?;
    session.last_used_at = Instant::now();
    let result = session.session.call_cancellable(
        "tools/call",
        Some(json!({
            "name": name,
            "arguments": arguments
        })),
        cancellation,
    );
    match result {
        Ok(result) => {
            session.last_used_at = Instant::now();
            Ok(result)
        }
        Err(error) => {
            let stopped = session_slot.take();
            if let Some(stopped) = stopped {
                let shutdown = stopped.shutdown_command;
                drop(stopped);
                run_shutdown_best_effort(shutdown);
            }
            Err(error)
        }
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
        self.read_response(request_id, method, None)
    }

    fn call_cancellable(
        &mut self,
        method: &str,
        params: Option<Value>,
        cancellation: &RunCancellation,
    ) -> Result<Value> {
        cancellation.check()?;
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
        self.read_response(request_id, method, Some(cancellation))
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
    fn read_response(
        &mut self,
        request_id: u64,
        method: &str,
        cancellation: Option<&RunCancellation>,
    ) -> Result<Value> {
        let timeout = request_timeout_for_method(method);
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(cancellation) = cancellation {
                cancellation.check()?;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(self.error_with_stderr(
                    McpRequestTimeout::new(self.command.program, method, timeout).into(),
                ));
            }
            let wait = if cancellation.is_some() {
                remaining.min(MCP_CANCELLATION_POLL_INTERVAL)
            } else {
                remaining
            };
            let line = match self.stdout_lines.recv_timeout(wait) {
                Ok(Ok(line)) => line,
                Ok(Err(error)) => {
                    return Err(self.error_with_stderr(anyhow!("{error} for {method}")));
                }
                Err(RecvTimeoutError::Timeout) => {
                    if cancellation.is_some() && wait < remaining {
                        continue;
                    }
                    return Err(self.error_with_stderr(
                        McpRequestTimeout::new(self.command.program, method, timeout).into(),
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

/// Returns the request timeout for one MCP method.
fn request_timeout_for_method(method: &str) -> Duration {
    if method == "tools/call" {
        MCP_TOOL_CALL_TIMEOUT
    } else {
        MCP_PROTOCOL_REQUEST_TIMEOUT
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
    let runtime_environment = MCP_RUNTIME_ENVIRONMENT
        .iter()
        .filter_map(|name| env::var_os(name).map(|value| (*name, value)))
        .collect::<Vec<_>>();
    process.env_clear();
    process.args(command.args);
    process.envs(runtime_environment);
    for variable in command.env {
        process.env(variable.key, resolve_env_value(variable.value)?);
    }

    Ok(())
}

/// Resolves an MCP environment value at process-start time.
fn resolve_env_value(value: McpEnvValue) -> Result<String> {
    match value {
        McpEnvValue::WindieDataDir(relative_path) => Ok(paths::data_dir()
            .join(relative_path)
            .to_string_lossy()
            .into_owned()),
        McpEnvValue::Literal(value) => Ok(value.to_string()),
        McpEnvValue::UserEnv(name) => provider_env::required(name),
    }
}

/// Reads provider stdout on a dedicated thread so protocol waits can time out.
fn spawn_stdout_reader(stdout: ChildStdout) -> Receiver<Result<String, String>> {
    let (sender, receiver) = mpsc::sync_channel(MCP_STDOUT_CHANNEL_CAPACITY);

    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);

        loop {
            match read_bounded_stdout_line(&mut reader, MCP_MAX_FRAME_BYTES) {
                Ok(None) => {
                    let _ = sender.send(Err("MCP provider closed stdout".to_string()));
                    break;
                }
                Ok(Some(line)) => {
                    if sender.send(Ok(line)).is_err() {
                        break;
                    }
                }
                Err(error) => {
                    let _ = sender.send(Err(error));
                    break;
                }
            }
        }
    });

    receiver
}

/// Reads one UTF-8 line without allowing an MCP process to allocate an
/// unbounded protocol frame before JSON decoding.
fn read_bounded_stdout_line(
    reader: &mut impl BufRead,
    max_bytes: usize,
) -> std::result::Result<Option<String>, String> {
    let mut bytes = Vec::new();
    let read = reader
        .take(max_bytes.saturating_add(1) as u64)
        .read_until(b'\n', &mut bytes)
        .map_err(|error| format!("failed to read MCP response: {error}"))?;
    if read == 0 {
        return Ok(None);
    }
    if read > max_bytes {
        return Err(format!("MCP response frame exceeds {max_bytes} bytes"));
    }

    String::from_utf8(bytes)
        .map(Some)
        .map_err(|error| format!("MCP response was not valid UTF-8: {error}"))
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

    const ENVIRONMENT_TEST_MCP: McpCommand = McpCommand {
        program: "/bin/echo",
        args: &[],
        env: &[McpEnv {
            key: "WINDIE_DECLARED_VALUE",
            value: McpEnvValue::Literal("declared"),
        }],
    };

    const SLOW_TEST_MCP: McpCommand = McpCommand {
        program: "/bin/sh",
        args: &[
            "-c",
            concat!(
                "while IFS= read -r line; do\n",
                "case \"$line\" in\n",
                "*'\"method\":\"initialize\"'*) printf '%s\\n' '{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}' ;;\n",
                "*'\"method\":\"tools/call\"'*) sleep 1; printf '%s\\n' '{\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{\"content\":[]}}'; exit 0 ;;\n",
                "esac\n",
                "done",
            ),
        ],
        env: &[],
    };
    const FAST_TEST_MCP: McpCommand = McpCommand {
        program: "/bin/sh",
        args: &[
            "-c",
            concat!(
                "while IFS= read -r line; do\n",
                "case \"$line\" in\n",
                "*'\"method\":\"initialize\"'*) printf '%s\\n' '{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}' ;;\n",
                "*'\"method\":\"tools/call\"'*) printf '%s\\n' '{\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{\"content\":[]}}'; exit 0 ;;\n",
                "esac\n",
                "done",
            ),
        ],
        env: &[],
    };
    const SEQUENTIAL_TEST_MCP: McpCommand = McpCommand {
        program: "/bin/sh",
        args: &[
            "-c",
            concat!(
                "calls=0\n",
                "while IFS= read -r line; do\n",
                "case \"$line\" in\n",
                "*'\"method\":\"initialize\"'*) printf '%s\\n' '{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}' ;;\n",
                "*'\"method\":\"tools/call\"'*) calls=$((calls + 1)); sleep 0.25; printf '{\"jsonrpc\":\"2.0\",\"id\":%s,\"result\":{\"content\":[]}}\\n' \"$((calls + 1))\" ;;\n",
                "esac\n",
                "done",
            ),
        ],
        env: &[],
    };

    #[test]
    fn captured_stderr_returns_provider_text() {
        let captured = Arc::new(Mutex::new(b"missing permission".to_vec()));

        assert_eq!(captured_stderr(&captured), "missing permission");
    }

    #[test]
    fn stdout_reader_rejects_oversized_protocol_frame() {
        let mut reader = std::io::Cursor::new(b"12345\n");

        let error = read_bounded_stdout_line(&mut reader, 4).unwrap_err();

        assert_eq!(error, "MCP response frame exceeds 4 bytes");
    }

    #[test]
    fn stdout_reader_accepts_bounded_utf8_line() {
        let mut reader = std::io::Cursor::new("ok\n".as_bytes());

        let line = read_bounded_stdout_line(&mut reader, 3).unwrap();

        assert_eq!(line.as_deref(), Some("ok\n"));
    }

    #[test]
    fn captured_stderr_marks_truncated_text() {
        let captured = Arc::new(Mutex::new(vec![b'x'; MCP_STDERR_MAX_BYTES]));

        assert!(captured_stderr(&captured).ends_with("\n[truncated]"));
    }

    #[test]
    fn tool_calls_use_longer_timeout_than_protocol_requests() {
        assert_eq!(
            request_timeout_for_method("initialize"),
            MCP_PROTOCOL_REQUEST_TIMEOUT
        );
        assert_eq!(
            request_timeout_for_method("tools/list"),
            MCP_PROTOCOL_REQUEST_TIMEOUT
        );
        assert_eq!(
            request_timeout_for_method("tools/call"),
            MCP_TOOL_CALL_TIMEOUT
        );
    }

    #[test]
    fn mcp_timeout_errors_report_elapsed_limit() {
        let timeout =
            McpRequestTimeout::new("desktop-commander", "tools/call", MCP_TOOL_CALL_TIMEOUT);
        let error: anyhow::Error = timeout.into();
        let found = request_timeout_from_error(&error).unwrap();

        assert_eq!(found.provider, "desktop-commander");
        assert_eq!(found.method, "tools/call");
        assert_eq!(found.timeout_ms(), 300_000);
        assert_eq!(
            error.to_string(),
            "MCP provider timed out during tools/call after 300s: desktop-commander"
        );
    }

    #[test]
    fn windie_data_dir_env_value_resolves_under_user_home() {
        let value = resolve_env_value(McpEnvValue::WindieDataDir("mcp/desktop-commander")).unwrap();

        assert!(value.ends_with(".local/share/windie/mcp/desktop-commander"));
    }

    #[test]
    fn literal_env_value_resolves_directly() {
        let value = resolve_env_value(McpEnvValue::Literal("true")).unwrap();

        assert_eq!(value, "true");
    }

    #[test]
    fn user_env_value_resolves_from_process_environment() {
        let expected = env::var("HOME").unwrap();
        let value = resolve_env_value(McpEnvValue::UserEnv("HOME")).unwrap();

        assert_eq!(value, expected);
    }

    #[test]
    fn missing_user_env_value_returns_clear_error() {
        let error =
            resolve_env_value(McpEnvValue::UserEnv("WINDIE_TEST_MISSING_MCP_ENV")).unwrap_err();

        assert!(error.to_string().contains(
            "missing required provider environment variable: WINDIE_TEST_MISSING_MCP_ENV"
        ));
    }

    #[test]
    fn provider_environment_removes_unrelated_values() {
        let mut process = Command::new(ENVIRONMENT_TEST_MCP.program);
        process.env("WINDIE_UNRELATED_SECRET", "must-not-leak");

        configure_process(&mut process, ENVIRONMENT_TEST_MCP).unwrap();
        let environment = process
            .get_envs()
            .map(|(key, value)| {
                (
                    key.to_string_lossy().into_owned(),
                    value.map(|value| value.to_string_lossy().into_owned()),
                )
            })
            .collect::<HashMap<_, _>>();

        assert!(!environment.contains_key("WINDIE_UNRELATED_SECRET"));
        assert_eq!(
            environment.get("WINDIE_DECLARED_VALUE"),
            Some(&Some("declared".to_string()))
        );
    }

    #[test]
    fn dropping_session_pool_stops_owned_reaper() {
        let pool = McpSessionPool::new();
        let inner = Arc::downgrade(&pool.inner);

        drop(pool);

        assert!(inner.upgrade().is_none());
    }

    #[test]
    fn slow_provider_does_not_block_another_provider() {
        let pool = McpSessionPool::new();
        let slow_pool = pool.clone();
        let slow = std::thread::spawn(move || {
            slow_pool.call_tool("slow", SLOW_TEST_MCP, None, "slow", json!({}))
        });
        std::thread::sleep(Duration::from_millis(100));

        let started = Instant::now();
        pool.call_tool("fast", FAST_TEST_MCP, None, "fast", json!({}))
            .unwrap();
        let fast_elapsed = started.elapsed();

        slow.join().unwrap().unwrap();
        assert!(
            fast_elapsed < Duration::from_millis(500),
            "fast provider waited {fast_elapsed:?}"
        );
    }

    #[test]
    fn cancelling_tool_call_stops_session_before_retry() {
        let pool = McpSessionPool::new();
        let cancellation = RunCancellation::default();
        let worker_pool = pool.clone();
        let worker_cancellation = cancellation.clone();
        let started = Instant::now();
        let call = std::thread::spawn(move || {
            worker_pool.call_tool_cancellable(
                "cancelled",
                SLOW_TEST_MCP,
                None,
                "slow",
                json!({}),
                &worker_cancellation,
            )
        });
        std::thread::sleep(Duration::from_millis(100));
        cancellation.cancel();

        let error = call.join().unwrap().unwrap_err();
        assert!(crate::run::is_runtime_cancelled(&error));
        assert!(started.elapsed() < Duration::from_millis(500));

        pool.call_tool("cancelled", FAST_TEST_MCP, None, "fast", json!({}))
            .unwrap();
    }

    #[test]
    fn calls_to_one_provider_remain_sequential() {
        let pool = McpSessionPool::new();
        let first_pool = pool.clone();
        let started = Instant::now();
        let first = std::thread::spawn(move || {
            first_pool.call_tool("sequential", SEQUENTIAL_TEST_MCP, None, "first", json!({}))
        });
        std::thread::sleep(Duration::from_millis(50));
        let second_pool = pool.clone();
        let second = std::thread::spawn(move || {
            second_pool.call_tool("sequential", SEQUENTIAL_TEST_MCP, None, "second", json!({}))
        });

        first.join().unwrap().unwrap();
        second.join().unwrap().unwrap();
        let elapsed = started.elapsed();
        assert!(
            elapsed >= Duration::from_millis(450),
            "same-provider calls overlapped: {elapsed:?}"
        );
    }
}
