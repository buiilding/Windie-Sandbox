//! Minimal MCP stdio client.
//!
//! This module owns the protocol boundary for approved MCP providers. It runs a
//! configured command, speaks line-delimited JSON-RPC 2.0 over stdin/stdout,
//! performs the MCP initialize handshake, and exposes the two tool operations
//! Windie needs now: `tools/list` and `tools/call`.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use anyhow::{Context, Result, anyhow};
use serde::Deserialize;
use serde_json::{Value, json};

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
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

impl McpSession {
    /// Starts the provider process and completes the MCP initialize handshake.
    fn start(command: McpCommand) -> Result<Self> {
        let mut child = Command::new(command.program)
            .args(command.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
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
        let mut session = Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
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
        let mut line = String::new();

        loop {
            line.clear();
            let bytes = self
                .stdout
                .read_line(&mut line)
                .with_context(|| format!("failed to read MCP response for {method}"))?;
            if bytes == 0 {
                return Err(anyhow!(
                    "MCP provider closed stdout before responding to {method}"
                ));
            }
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
                return Err(anyhow!(
                    "MCP error {} from {method}: {}",
                    error.code,
                    error.message
                ));
            }

            return response
                .result
                .ok_or_else(|| anyhow!("MCP response for {method} did not include result"));
        }
    }
}

impl Drop for McpSession {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
