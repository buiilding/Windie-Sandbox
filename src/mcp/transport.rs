//! MCP transport routing.
//!
//! This module presents the stable list/call operations and routes them to
//! the explicit stdio or Streamable HTTP implementation.

use std::future::Future;

use anyhow::{Context, Result, anyhow};
use serde_json::{Value, json};

use super::protocol::{McpCommand, McpOwnedCommand, McpTool, McpToolsList, McpTransport};
use super::{session, stdio};

/// Lists tools from one approved MCP stdio provider.
pub fn list_tools(command: McpCommand) -> Result<Vec<McpTool>> {
    stdio::list_tools(command)
}

/// Runs one provider package preparation command before MCP startup.
pub fn run_preparation_command(command: McpCommand) -> Result<()> {
    stdio::run_preparation_command(command)
}

/// Runs a package-owned preparation command before MCP startup.
pub fn run_owned_preparation_command(command: McpOwnedCommand) -> Result<()> {
    stdio::run_owned_preparation_command(command)
}

/// Calls one MCP provider tool and returns the raw MCP result value.
pub fn call_tool(command: McpCommand, name: &str, arguments: Value) -> Result<Value> {
    stdio::call_tool(command, name, arguments)
}

/// Lists tools from either a local stdio or hosted Streamable HTTP provider.
pub fn list_tools_with_transport(transport: McpTransport) -> Result<Vec<McpTool>> {
    match transport {
        McpTransport::Stdio {
            command,
            shutdown_command,
        } => stdio::list_tools_with_shutdown(command, shutdown_command),
        McpTransport::PackagedStdio {
            command,
            shutdown_command,
        } => stdio::list_tools_with_owned_shutdown(command, shutdown_command),
        McpTransport::StreamableHttp { .. } => {
            run_async_on_dedicated_thread(list_tools_with_transport_async(transport))
        }
    }
}

/// Lists tools through either transport from an async caller.
pub async fn list_tools_with_transport_async(transport: McpTransport) -> Result<Vec<McpTool>> {
    let mut active = session::McpTransportSession::start(transport).await?;
    let result = active.call("tools/list", None).await;
    active.shutdown().await;
    let result = result?;

    serde_json::from_value::<McpToolsList>(result)
        .context("failed to decode MCP tools/list response")
        .map(|list| list.tools)
}

/// Calls one tool through either a local stdio or hosted Streamable HTTP provider.
pub fn call_tool_with_transport(
    transport: McpTransport,
    name: &str,
    arguments: Value,
) -> Result<Value> {
    match transport {
        McpTransport::Stdio {
            command,
            shutdown_command,
        } => stdio::call_tool_with_shutdown(command, shutdown_command, name, arguments),
        McpTransport::PackagedStdio {
            command,
            shutdown_command,
        } => stdio::call_tool_with_owned_shutdown(command, shutdown_command, name, arguments),
        McpTransport::StreamableHttp { .. } => run_async_on_dedicated_thread(
            call_tool_with_transport_async(transport, name.to_string(), arguments),
        ),
    }
}

/// Calls a tool through either transport from an async caller.
pub async fn call_tool_with_transport_async(
    transport: McpTransport,
    name: impl Into<String>,
    arguments: Value,
) -> Result<Value> {
    let mut active = session::McpTransportSession::start(transport).await?;
    let result = active
        .call(
            "tools/call",
            Some(json!({
                "name": name.into(),
                "arguments": arguments
            })),
        )
        .await;
    active.shutdown().await;
    result
}

/// Runs an async transport operation from a legacy synchronous caller.
///
/// Synchronous callers get an explicit OS-thread boundary instead of
/// nesting a Tokio runtime inside an existing async worker.
fn run_async_on_dedicated_thread<F, T>(future: F) -> Result<T>
where
    F: Future<Output = Result<T>> + Send + 'static,
    T: Send + 'static,
{
    std::thread::spawn(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .context("failed to build MCP HTTP runtime")?
            .block_on(future)
    })
    .join()
    .map_err(|_| anyhow!("MCP HTTP worker panicked"))?
}
