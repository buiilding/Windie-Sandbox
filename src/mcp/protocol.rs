//! MCP protocol contracts and JSON-RPC data shapes.
//!
//! This module contains the typed values shared by the MCP transports. It
//! does not start processes or perform network requests.

use std::fmt;
use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;

pub(super) const MCP_PROTOCOL_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
pub(super) const MCP_TOOL_CALL_TIMEOUT: Duration = Duration::from_secs(5 * 60);

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

/// Returns the request timeout for one MCP method.
pub(crate) fn request_timeout_for_method(method: &str) -> Duration {
    if method == "tools/call" {
        MCP_TOOL_CALL_TIMEOUT
    } else {
        MCP_PROTOCOL_REQUEST_TIMEOUT
    }
}

/// Process command for one approved MCP provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct McpCommand {
    pub program: &'static str,
    pub args: &'static [McpArgument],
    pub env: &'static [McpEnv],
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// A process command whose paths and arguments come from an installed package.
pub struct McpOwnedCommand {
    pub program: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub secret_env: Vec<(String, String, bool)>,
}

/// Transport used to connect Windie to one approved MCP provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpTransport {
    /// Launch a local MCP process and communicate over stdin/stdout.
    Stdio {
        command: McpCommand,
        shutdown_command: Option<McpCommand>,
    },
    /// Launch a local MCP using a command resolved from an installed package.
    PackagedStdio {
        command: McpOwnedCommand,
        shutdown_command: Option<McpOwnedCommand>,
    },
    /// Connect to a hosted MCP endpoint over Streamable HTTP.
    StreamableHttp { endpoint: McpHttpEndpoint },
}

impl McpTransport {
    /// Creates a local stdio transport without a provider shutdown command.
    pub const fn stdio(command: McpCommand) -> Self {
        Self::Stdio {
            command,
            shutdown_command: None,
        }
    }

    /// Creates a local stdio transport with a best-effort shutdown command.
    pub const fn stdio_with_shutdown(command: McpCommand, shutdown_command: McpCommand) -> Self {
        Self::Stdio {
            command,
            shutdown_command: Some(shutdown_command),
        }
    }

    /// Creates a hosted Streamable HTTP transport.
    pub fn streamable_http(endpoint: McpHttpEndpoint) -> Self {
        Self::StreamableHttp { endpoint }
    }
}

/// Hosted MCP endpoint metadata used by the Streamable HTTP transport.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpHttpEndpoint {
    /// HTTPS endpoint that accepts MCP POST requests.
    pub url: String,
    /// Authentication policy for requests to this endpoint.
    pub authorization: McpHttpAuthorization,
    /// Maximum time for initialization and catalog requests.
    pub startup_timeout: Duration,
    /// Maximum time for a tool call request.
    pub call_timeout: Duration,
}

impl McpHttpEndpoint {
    /// Builds an endpoint using Windie's conservative default timeouts.
    pub fn new(url: impl Into<String>, authorization: McpHttpAuthorization) -> Self {
        Self {
            url: url.into(),
            authorization,
            startup_timeout: MCP_PROTOCOL_REQUEST_TIMEOUT,
            call_timeout: MCP_TOOL_CALL_TIMEOUT,
        }
    }

    /// Builds an endpoint with package-declared request limits.
    pub fn with_timeouts(
        url: impl Into<String>,
        authorization: McpHttpAuthorization,
        startup_timeout: Duration,
        call_timeout: Duration,
    ) -> Self {
        Self {
            url: url.into(),
            authorization,
            startup_timeout,
            call_timeout,
        }
    }
}

/// Authentication policy for one hosted MCP endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpHttpAuthorization {
    /// Do not send an Authorization header.
    Anonymous,
    /// Require and send a Bearer header from the named Windie environment
    /// value.
    BearerEnv(String),
    /// Send a Bearer header when the named Windie environment value exists.
    OptionalBearerEnv(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// One approved MCP command argument.
///
/// Most provider arguments are fixed literals. A provider can also request a
/// path below Windie's per-user data directory; that path is resolved only
/// immediately before the child process starts so the static provider
/// definition never captures a machine-specific path.
pub enum McpArgument {
    /// Use a fixed argument owned by the approved provider definition.
    Literal(&'static str),
    /// Resolve this relative path below Windie's per-user data directory.
    WindieDataDir(&'static str),
}

impl McpArgument {
    /// Returns the provider argument representation exposed in its manifest.
    ///
    /// A dynamic path is intentionally represented as a placeholder in the
    /// catalog; the actual absolute path remains private to process launch.
    pub(crate) fn manifest_value(self) -> String {
        match self {
            Self::Literal(value) => value.to_string(),
            Self::WindieDataDir(relative_path) => {
                format!("<windie-data-dir>/{relative_path}")
            }
        }
    }
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
    /// Copy a value from Windie's user-local env into the provider child.
    ///
    /// This keeps provider secret names explicit. For example, Windie can read
    /// `BRIGHTDATA_API_TOKEN` from `~/.windie/.env` and pass it to a child
    /// process as that provider's expected `API_TOKEN`. The process environment
    /// remains a fallback for explicit shell overrides and tests.
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
pub(super) struct McpToolsList {
    pub(super) tools: Vec<McpTool>,
}

#[derive(Debug, Deserialize)]
/// JSON-RPC response envelope from an MCP server.
pub(super) struct JsonRpcResponse {
    pub(super) id: Value,
    #[serde(default)]
    pub(super) result: Option<Value>,
    #[serde(default)]
    pub(super) error: Option<JsonRpcError>,
}

#[derive(Debug, Deserialize)]
/// JSON-RPC error body from an MCP server.
pub(super) struct JsonRpcError {
    pub(super) code: i64,
    pub(super) message: String,
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
