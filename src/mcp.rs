//! Minimal MCP stdio client.
//!
//! This module owns the protocol boundary for approved MCP providers. It runs a
//! configured command, speaks line-delimited JSON-RPC 2.0 over stdin/stdout,
//! performs the MCP initialize handshake, and exposes the two tool operations
//! Windie needs now: `tools/list` and `tools/call`.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use serde::Deserialize;
use serde_json::{Value, json};

const MCP_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const MCP_STDERR_MAX_BYTES: usize = 16 * 1024;

/// Process command for one approved MCP provider.
#[derive(Debug, Clone, Copy)]
pub struct McpCommand {
    pub program: &'static str,
    pub args: &'static [&'static str],
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
        let mut child = Command::new(command.program)
            .args(command.args)
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
            if response.id != Value::from(request_id) {
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
}
