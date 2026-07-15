//! MCP tool executor.
//!
//! This module runs an already-approved MCP tool call. Policy and approval
//! happen before this layer; execution only validates the provider mapping,
//! calls MCP, and normalizes the MCP result.

use anyhow::Result;
use serde_json::Value;

use super::provider::McpToolProvider;
use super::result::{mcp_tool_call_failure_result, mcp_tool_result_parts, tool_result_preview};
use crate::conversation::ToolCall;
use crate::error;
use crate::mcp::{self, McpSessionPool};
use crate::tool::{AttachedTool, ToolExecutionResult};

impl McpToolProvider {
    /// Executes one approved MCP tool call.
    pub(in crate::tool_provider) async fn call_tool(
        &self,
        attached_tool: &AttachedTool,
        tool_call: &ToolCall,
        session_pool: Option<&McpSessionPool>,
    ) -> Result<ToolExecutionResult> {
        if attached_tool.provider.provider_id != self.provider_id
            || tool_call.name() != attached_tool.schema_name.as_str()
        {
            return Err(error::invalid_request(format!(
                "unknown tool: {}",
                tool_call.name()
            )));
        }
        let arguments = match serde_json::from_str::<Value>(tool_call.arguments()) {
            Ok(arguments) => arguments,
            Err(error) => {
                return Ok(ToolExecutionResult::failure(
                    tool_call.id.clone(),
                    tool_call.name(),
                    format!("invalid tool arguments: {error}"),
                ));
            }
        };
        self.prepare()?;
        let result = match if let Some(session_pool) = session_pool {
            session_pool.call_tool(
                self.provider_id.as_str(),
                self.command,
                self.shutdown_command,
                attached_tool.provider.tool_name.as_str(),
                arguments,
            )
        } else {
            mcp::call_tool_with_shutdown(
                self.command,
                self.shutdown_command,
                attached_tool.provider.tool_name.as_str(),
                arguments,
            )
        } {
            Ok(result) => result,
            Err(error) => {
                return Ok(mcp_tool_call_failure_result(
                    &self.provider_id,
                    tool_call,
                    &error,
                ));
            }
        };
        let success = !result
            .get("isError")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        let normalized = match mcp_tool_result_parts(&result) {
            Ok(parts) => parts,
            Err(error) => {
                return Ok(ToolExecutionResult::failure(
                    tool_call.id.clone(),
                    tool_call.name(),
                    error.to_string(),
                ));
            }
        };

        let mut execution_result = ToolExecutionResult::success_with_parts(
            tool_call.id.clone(),
            tool_call.name(),
            tool_result_preview(&normalized),
            normalized,
        );
        execution_result.success = success;

        Ok(execution_result)
    }
}
