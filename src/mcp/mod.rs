//! MCP protocol, transport, and runtime boundary.
//!
//! This module is the stable MCP facade. Concrete protocol contracts, local
//! process execution, session lifecycle, and transport routing live in focused
//! child modules so callers do not need to know how MCP is implemented.

mod chrome_devtools;
mod executor;
mod http;
mod loader;
mod mcpb;
mod protocol;
mod result;
mod session;
mod stdio;
mod tool_provider;
mod transport;

pub use protocol::{
    McpArgument, McpCommand, McpEnv, McpEnvValue, McpHttpAuthorization, McpHttpEndpoint,
    McpOwnedCommand, McpRequestTimeout, McpTool, McpToolAnnotations, McpTransport,
    request_timeout_from_error,
};
pub use session::McpSessionPool;
pub use stdio::{call_tool_with_shutdown, list_tools_with_shutdown};
pub use transport::{
    call_tool, call_tool_with_transport, call_tool_with_transport_async, list_tools,
    list_tools_with_transport, list_tools_with_transport_async, run_owned_preparation_command,
    run_preparation_command,
};

pub(crate) use chrome_devtools::ChromeDevToolsConnectionMode;
pub(crate) use loader::load_components;
pub(crate) use tool_provider::{McpProviderDefinition, McpToolProvider};

#[cfg(test)]
pub(crate) use protocol::request_timeout_for_method;
#[cfg(test)]
pub(crate) use result::{mcp_tool_call_failure_result, mcp_tool_result_parts, tool_result_preview};
#[cfg(test)]
pub(crate) use tool_provider::mcp_schema_name;
