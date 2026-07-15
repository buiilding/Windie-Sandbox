//! MCP tool provider backend family.
//!
//! MCP is currently Windie's only implemented executable tool backend. This
//! module keeps the generic MCP adapter separate from the individual
//! code-approved MCP server definitions.

mod approved;
mod blender;
mod brightdata;
mod cua;
mod desktop_commander;
mod executor;
mod provider;
mod result;

pub(in crate::tool_provider) use approved::approved_mcp_providers;
pub(in crate::tool_provider) use provider::{McpProviderDefinition, McpToolProvider};

#[cfg(test)]
pub(in crate::tool_provider) use approved::approved_mcp_provider;
#[cfg(test)]
pub(in crate::tool_provider) use provider::mcp_schema_name;
#[cfg(test)]
pub(in crate::tool_provider) use result::{
    mcp_tool_call_failure_result, mcp_tool_result_parts, tool_result_preview,
};
