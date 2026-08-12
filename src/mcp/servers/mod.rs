//! MCP tool provider backend family.
//!
//! MCP is currently Windie's only implemented executable tool backend. This
//! module keeps the generic MCP adapter separate from the individual
//! code-approved MCP server definitions.

mod approved;
mod basic_memory;
mod blender;
mod brightdata;
mod chrome_devtools;
mod cua;
mod desktop_commander;
mod executor;
mod parallel;
mod provider;
mod result;

pub(crate) use approved::approved_mcp_providers;
pub(crate) use chrome_devtools::ChromeDevToolsConnectionMode;
pub(crate) use provider::{McpProviderDefinition, McpToolProvider};

#[cfg(test)]
pub(crate) use approved::approved_mcp_provider;
#[cfg(test)]
pub(crate) use provider::mcp_schema_name;
#[cfg(test)]
pub(crate) use result::{mcp_tool_call_failure_result, mcp_tool_result_parts, tool_result_preview};
