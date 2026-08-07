//! Streamable HTTP MCP client.
//!
//! This module owns the hosted MCP transport used by remote providers such as
//! Parallel Search. It deliberately shares the JSON-RPC/session boundary with
//! the stdio client while keeping HTTP headers, authentication, status errors,
//! and session cleanup out of provider definitions.

use std::env;
use std::fmt;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use reqwest::blocking::{Client, RequestBuilder, Response};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{McpHttpAuthorization, McpHttpEndpoint, request_timeout_for_method};
use crate::local;

const MCP_PROTOCOL_VERSION: &str = "2025-06-18";
const HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const HTTP_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug)]
/// One active hosted Streamable HTTP MCP session.
pub(super) struct StreamableHttpSession {
    endpoint: McpHttpEndpoint,
    client: Client,
    session_id: Option<String>,
    protocol_version: String,
    next_id: u64,
}

impl StreamableHttpSession {
    /// Builds the HTTP client and completes the MCP initialization handshake.
    pub(super) fn start(endpoint: McpHttpEndpoint) -> Result<Self> {
        let client = Client::builder()
            .connect_timeout(HTTP_CONNECT_TIMEOUT)
            .build()
            .context("failed to build MCP HTTP client")?;
        let mut session = Self {
            endpoint,
            client,
            session_id: None,
            protocol_version: MCP_PROTOCOL_VERSION.to_string(),
            next_id: 0,
        };
        session.initialize()?;
        Ok(session)
    }

    /// Sends one MCP request and returns its JSON-RPC result.
    pub(super) fn call(&mut self, method: &str, params: Option<Value>) -> Result<Value> {
        self.next_id += 1;
        let request_id = self.next_id;
        let request = request_value(request_id, method, params);

        match self.send_request(&request, method, Some(request_id)) {
            Ok(Some(result)) => Ok(result),
            Ok(None) => Err(anyhow!(
                "MCP HTTP response for {method} did not include a result"
            )),
            Err(error) if is_status(&error, 404) && self.session_id.is_some() => {
                self.initialize()?;
                match self.send_request(&request, method, Some(request_id))? {
                    Some(result) => Ok(result),
                    None => Err(anyhow!(
                        "MCP HTTP response for {method} did not include a result"
                    )),
                }
            }
            Err(error) => Err(error),
        }
    }

    /// Sends one MCP notification, accepting the normal HTTP 202 response.
    fn notify(&mut self, method: &str, params: Option<Value>) -> Result<()> {
        let request = notification_value(method, params);
        self.send_request(&request, method, None)?;
        Ok(())
    }

    /// Performs initialization without carrying over an expired session.
    fn initialize(&mut self) -> Result<()> {
        self.session_id = None;
        self.protocol_version = MCP_PROTOCOL_VERSION.to_string();
        self.next_id = self.next_id.max(1);
        let request_id = self.next_id;
        let result = self
            .send_request(
                &request_value(
                    request_id,
                    "initialize",
                    Some(json!({
                        "protocolVersion": MCP_PROTOCOL_VERSION,
                        "capabilities": {},
                        "clientInfo": {
                            "name": "windie",
                            "version": env!("CARGO_PKG_VERSION")
                        }
                    })),
                ),
                "initialize",
                Some(request_id),
            )?
            .ok_or_else(|| anyhow!("MCP HTTP initialize response was empty"))?;

        if let Some(protocol_version) = result.get("protocolVersion").and_then(Value::as_str) {
            self.protocol_version = protocol_version.to_string();
        }
        self.notify("notifications/initialized", None)
    }

    /// Sends one HTTP request and decodes its JSON-RPC response.
    fn send_request(
        &mut self,
        request: &Value,
        method: &str,
        request_id: Option<u64>,
    ) -> Result<Option<Value>> {
        let timeout = request_timeout_for_method(method);
        let mut builder = self
            .client
            .post(self.endpoint.url)
            .timeout(timeout)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .json(request);

        if method != "initialize" {
            builder = builder.header("MCP-Protocol-Version", &self.protocol_version);
            if let Some(session_id) = &self.session_id {
                builder = builder.header("Mcp-Session-Id", session_id);
            }
        }
        builder = apply_authorization(builder, self.endpoint.authorization)?;

        let response = builder
            .send()
            .with_context(|| format!("MCP HTTP request failed for {method}"))?;
        self.capture_session_id(&response);

        if !response.status().is_success() {
            return Err(McpHttpStatusError {
                status: response.status().as_u16(),
                method: method.to_string(),
            }
            .into());
        }

        if response.status().as_u16() == 202 {
            return Ok(None);
        }

        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let body = response
            .text()
            .with_context(|| format!("failed to read MCP HTTP response for {method}"))?;
        if body.trim().is_empty() {
            return Ok(None);
        }

        if content_type.contains("text/event-stream") {
            parse_sse_response(&body, request_id, method)
        } else {
            parse_json_response(&body, request_id, method)
        }
    }

    /// Captures the session ID returned by the server during initialization.
    fn capture_session_id(&mut self, response: &Response) {
        if let Some(session_id) = response
            .headers()
            .get("Mcp-Session-Id")
            .and_then(|value| value.to_str().ok())
        {
            self.session_id = Some(session_id.to_string());
        }
    }
}

impl Drop for StreamableHttpSession {
    /// Terminates the remote MCP session on idle cleanup or process shutdown.
    fn drop(&mut self) {
        let Some(session_id) = self.session_id.take() else {
            return;
        };

        let Ok(mut builder) = apply_authorization(
            self.client
                .delete(self.endpoint.url)
                .timeout(HTTP_SHUTDOWN_TIMEOUT)
                .header("Mcp-Session-Id", session_id),
            self.endpoint.authorization,
        ) else {
            return;
        };
        builder = builder.header("MCP-Protocol-Version", &self.protocol_version);
        let _ = builder.send();
    }
}

#[derive(Debug)]
/// Safe HTTP status error that never stores request headers or credentials.
struct McpHttpStatusError {
    status: u16,
    method: String,
}

impl fmt::Display for McpHttpStatusError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let description = match self.status {
            401 => "authentication failed",
            402 => "quota or billing requirement",
            429 => "rate limited",
            500..=599 => "remote provider unavailable",
            _ => "remote provider rejected the request",
        };
        write!(
            formatter,
            "MCP HTTP {description} during {} (status {})",
            self.method, self.status
        )
    }
}

impl std::error::Error for McpHttpStatusError {}

/// Returns whether an error is one of the HTTP statuses handled by retry logic.
fn is_status(error: &anyhow::Error, status: u16) -> bool {
    error
        .downcast_ref::<McpHttpStatusError>()
        .is_some_and(|http_error| http_error.status == status)
}

/// Adds an optional provider Bearer token without exposing it to diagnostics.
fn apply_authorization(
    builder: RequestBuilder,
    authorization: McpHttpAuthorization,
) -> Result<RequestBuilder> {
    let (name, required) = match authorization {
        McpHttpAuthorization::Anonymous => return Ok(builder),
        McpHttpAuthorization::BearerEnv(name) => (name, true),
        McpHttpAuthorization::OptionalBearerEnv(name) => (name, false),
    };

    let token = local::env_value(name)?.or_else(|| env::var(name).ok());
    match token.filter(|value| !value.trim().is_empty()) {
        Some(token) => Ok(builder.bearer_auth(token)),
        None if required => Err(anyhow!("missing MCP HTTP credential {name}")),
        None => Ok(builder),
    }
}

#[derive(Debug, Deserialize)]
struct JsonRpcResponse {
    id: Value,
    #[serde(default)]
    result: Option<Value>,
    #[serde(default)]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Deserialize)]
struct JsonRpcError {
    code: i64,
    message: String,
}

/// Creates one JSON-RPC request value.
fn request_value(id: u64, method: &str, params: Option<Value>) -> Value {
    let mut request = json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
    });
    if let Some(params) = params {
        request["params"] = params;
    }
    request
}

/// Creates one JSON-RPC notification value.
fn notification_value(method: &str, params: Option<Value>) -> Value {
    let mut notification = json!({
        "jsonrpc": "2.0",
        "method": method,
    });
    if let Some(params) = params {
        notification["params"] = params;
    }
    notification
}

/// Parses one JSON response and verifies its JSON-RPC result.
fn parse_json_response(body: &str, request_id: Option<u64>, method: &str) -> Result<Option<Value>> {
    let response = serde_json::from_str::<JsonRpcResponse>(body)
        .with_context(|| format!("failed to decode MCP HTTP JSON response for {method}"))?;
    parse_rpc_response(response, request_id, method)
}

/// Parses a Streamable HTTP SSE body until the matching JSON-RPC response.
fn parse_sse_response(body: &str, request_id: Option<u64>, method: &str) -> Result<Option<Value>> {
    let mut data_lines = Vec::new();
    for line in body.lines().chain(std::iter::once("")) {
        if let Some(data) = line.strip_prefix("data:") {
            data_lines.push(data.trim_start());
            continue;
        }
        if data_lines.is_empty() {
            continue;
        }

        let data = data_lines.join("\n");
        data_lines.clear();
        if data == "[DONE]" {
            continue;
        }
        let response = serde_json::from_str::<JsonRpcResponse>(&data)
            .with_context(|| format!("failed to decode MCP HTTP SSE response for {method}"))?;
        if request_id.is_none_or(|id| response.id == json!(id)) {
            return parse_rpc_response(response, request_id, method);
        }
    }

    Err(anyhow!(
        "MCP HTTP SSE response did not include a result for {method}"
    ))
}

/// Converts a JSON-RPC response or error into the provider result value.
fn parse_rpc_response(
    response: JsonRpcResponse,
    request_id: Option<u64>,
    method: &str,
) -> Result<Option<Value>> {
    if request_id.is_some_and(|id| response.id != json!(id)) {
        return Err(anyhow!(
            "MCP HTTP response ID did not match request for {method}"
        ));
    }
    if let Some(error) = response.error {
        return Err(anyhow!(
            "MCP error {} from {method}: {}",
            error.code,
            error.message
        ));
    }
    Ok(response.result)
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::thread;

    use super::*;
    use crate::mcp::McpRequestTimeout;

    #[test]
    fn parses_sse_response() {
        let body =
            "event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"ok\":true}}\n\n";
        let result = parse_sse_response(body, Some(1), "tools/list")
            .unwrap()
            .unwrap();
        assert_eq!(result["ok"], true);
    }

    #[test]
    fn optional_authentication_can_be_anonymous() {
        let request = apply_authorization(
            Client::new().get("https://example.test"),
            McpHttpAuthorization::Anonymous,
        )
        .unwrap()
        .build()
        .unwrap();
        assert!(request.headers().get("authorization").is_none());
    }

    #[test]
    fn required_authentication_reports_only_the_credential_name() {
        let error = apply_authorization(
            Client::new().get("https://example.test"),
            McpHttpAuthorization::BearerEnv("WINDIE_TEST_MISSING_MCP_HTTP_TOKEN"),
        )
        .unwrap_err()
        .to_string();

        assert_eq!(
            error,
            "missing MCP HTTP credential WINDIE_TEST_MISSING_MCP_HTTP_TOKEN"
        );
        assert!(!error.contains("Bearer"));
    }

    #[test]
    fn timeout_error_uses_the_mcp_timeout_contract() {
        let timeout = McpRequestTimeout::new(
            "parallel-search",
            "tools/call",
            request_timeout_for_method("tools/call"),
        );
        assert_eq!(timeout.timeout_seconds(), 300);
    }

    #[test]
    fn session_uses_streamable_http_headers_and_cleans_up() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let endpoint_url: &'static str =
            Box::leak(format!("http://{address}/mcp").into_boxed_str());
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut requests = Vec::new();
            for index in 0..4 {
                let request = read_http_request(&mut stream).unwrap();
                requests.push(request);
                match index {
                    0 => write_json_response(
                        &mut stream,
                        200,
                        Some("Mcp-Session-Id: test-session\r\n"),
                        r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-06-18","capabilities":{},"serverInfo":{"name":"fixture","version":"1"}}}"#,
                    ),
                    1 => write_empty_response(&mut stream, 202),
                    2 => write_json_response(
                        &mut stream,
                        200,
                        None,
                        r#"{"jsonrpc":"2.0","id":2,"result":{"tools":[]}}"#,
                    ),
                    3 => write_empty_response(&mut stream, 200),
                    _ => unreachable!(),
                }
            }
            requests
        });

        {
            let mut session = StreamableHttpSession::start(McpHttpEndpoint {
                url: endpoint_url,
                authorization: McpHttpAuthorization::Anonymous,
            })
            .unwrap();
            let result = session.call("tools/list", None).unwrap();
            assert_eq!(result["tools"], json!([]));
        }

        let requests = server.join().unwrap();
        assert!(contains_header(
            &requests[0],
            "accept: application/json, text/event-stream"
        ));
        assert!(!contains_header(&requests[0], "mcp-session-id:"));
        assert!(contains_header(
            &requests[1],
            "mcp-session-id: test-session"
        ));
        assert!(contains_header(
            &requests[2],
            "mcp-session-id: test-session"
        ));
        assert!(contains_header(
            &requests[2],
            "mcp-protocol-version: 2025-06-18"
        ));
        assert!(requests[3].starts_with("DELETE /mcp HTTP/1.1"));
        assert!(contains_header(
            &requests[3],
            "mcp-session-id: test-session"
        ));
    }

    #[test]
    fn expired_session_reinitializes_and_retries_once() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let endpoint_url: &'static str =
            Box::leak(format!("http://{address}/mcp").into_boxed_str());
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut requests = Vec::new();
            for index in 0..7 {
                let request = read_http_request(&mut stream).unwrap();
                requests.push(request);
                match index {
                    0 => write_json_response(
                        &mut stream,
                        200,
                        Some("Mcp-Session-Id: first-session\r\n"),
                        r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-06-18","capabilities":{},"serverInfo":{"name":"fixture","version":"1"}}}"#,
                    ),
                    1 => write_empty_response(&mut stream, 202),
                    2 => write_empty_response(&mut stream, 404),
                    3 => write_json_response(
                        &mut stream,
                        200,
                        Some("Mcp-Session-Id: second-session\r\n"),
                        r#"{"jsonrpc":"2.0","id":2,"result":{"protocolVersion":"2025-06-18","capabilities":{},"serverInfo":{"name":"fixture","version":"1"}}}"#,
                    ),
                    4 => write_empty_response(&mut stream, 202),
                    5 => write_json_response(
                        &mut stream,
                        200,
                        None,
                        r#"{"jsonrpc":"2.0","id":2,"result":{"tools":[]}}"#,
                    ),
                    6 => write_empty_response(&mut stream, 200),
                    _ => unreachable!(),
                }
            }
            requests
        });

        {
            let mut session = StreamableHttpSession::start(McpHttpEndpoint {
                url: endpoint_url,
                authorization: McpHttpAuthorization::Anonymous,
            })
            .unwrap();
            let result = session.call("tools/list", None).unwrap();
            assert_eq!(result["tools"], json!([]));
        }

        let requests = server.join().unwrap();
        assert!(contains_header(
            &requests[2],
            "mcp-session-id: first-session"
        ));
        assert!(!contains_header(
            &requests[3],
            "mcp-session-id: first-session"
        ));
        assert!(contains_header(
            &requests[4],
            "mcp-session-id: second-session"
        ));
        assert!(contains_header(
            &requests[5],
            "mcp-protocol-version: 2025-06-18"
        ));
        assert!(contains_header(
            &requests[5],
            "mcp-session-id: second-session"
        ));
        assert!(requests[6].starts_with("DELETE /mcp HTTP/1.1"));
        assert!(contains_header(
            &requests[6],
            "mcp-session-id: second-session"
        ));
    }

    fn read_http_request(stream: &mut TcpStream) -> Option<String> {
        let mut bytes = Vec::new();
        let mut buffer = [0_u8; 1];
        while !bytes.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut buffer).ok()?;
            bytes.push(buffer[0]);
        }
        let headers = String::from_utf8_lossy(&bytes).to_string();
        let content_length = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length").then_some(value)
            })
            .and_then(|value| value.trim().parse::<usize>().ok())
            .unwrap_or(0);
        let mut body = vec![0_u8; content_length];
        stream.read_exact(&mut body).ok()?;
        bytes.extend(body);
        Some(String::from_utf8_lossy(&bytes).to_string())
    }

    fn write_json_response(
        stream: &mut TcpStream,
        status: u16,
        extra_headers: Option<&str>,
        body: &str,
    ) {
        let headers = extra_headers.unwrap_or_default();
        let response = format!(
            "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: keep-alive\r\n{headers}\r\n{body}",
            body.len()
        );
        stream.write_all(response.as_bytes()).unwrap();
        stream.flush().unwrap();
    }

    fn write_empty_response(stream: &mut TcpStream, status: u16) {
        let response =
            format!("HTTP/1.1 {status} OK\r\nContent-Length: 0\r\nConnection: keep-alive\r\n\r\n");
        stream.write_all(response.as_bytes()).unwrap();
        stream.flush().unwrap();
    }

    fn contains_header(request: &str, expected: &str) -> bool {
        request.to_ascii_lowercase().contains(expected)
    }
}
