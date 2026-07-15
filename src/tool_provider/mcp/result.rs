//! MCP tool-call result normalization.
//!
//! This module converts MCP `tools/call` successes and failures into Windie's
//! provider-neutral `ToolExecutionResult` shape.

use anyhow::{Context, Result, anyhow};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Value, json};

use crate::conversation::{ToolCall, UnsavedImagePart, UnsavedMessagePart};
use crate::mcp;
use crate::tool::{ToolExecutionResult, ToolProviderId};

/// Converts an MCP `tools/call` operation failure into a model-facing tool
/// result.
///
/// At this point policy has already approved the model's tool call. Returning a
/// failed result lets runtime persist a linked `role: tool` message so the next
/// model turn can observe the failure instead of losing the tool-call contract
/// to an outer operation error.
pub(in crate::tool_provider) fn mcp_tool_call_failure_result(
    provider_id: &ToolProviderId,
    tool_call: &ToolCall,
    error: &anyhow::Error,
) -> ToolExecutionResult {
    let content = if let Some(timeout) = mcp::request_timeout_from_error(error) {
        json!({
            "error": "MCP provider timed out",
            "detail": timeout.to_string(),
            "provider": timeout.provider.as_str(),
            "method": timeout.method.as_str(),
            "timeout_ms": timeout.timeout_ms(),
            "timeout_seconds": timeout.timeout_seconds()
        })
    } else {
        json!({
            "error": "MCP provider tool call failed",
            "detail": error.to_string(),
            "provider": provider_id.as_str(),
            "method": "tools/call"
        })
    };

    ToolExecutionResult {
        tool_call_id: tool_call.id.clone(),
        tool_name: tool_call.name().to_string(),
        content: content.to_string(),
        parts: Vec::new(),
        success: false,
    }
}

/// Converts an MCP `tools/call` result into Windie's model-facing message
/// parts.
///
/// MCP can return text and binary images in the same content array. Windie
/// stores those images through `message_parts` and `image_assets` so the
/// Responses request can replay them as image blocks instead of base64 text.
pub(in crate::tool_provider) fn mcp_tool_result_parts(
    result: &Value,
) -> Result<Vec<UnsavedMessagePart>> {
    let mut parts = Vec::new();

    if let Some(content) = result.get("content").and_then(Value::as_array) {
        for item in content {
            match item.get("type").and_then(Value::as_str) {
                Some("text") => {
                    if let Some(text) = item.get("text").and_then(Value::as_str) {
                        parts.push(UnsavedMessagePart::Text(text.to_string()));
                    }
                }
                Some("image") => {
                    let data = item
                        .get("data")
                        .and_then(Value::as_str)
                        .ok_or_else(|| anyhow!("MCP image result did not include data"))?;
                    let mime_type = item
                        .get("mimeType")
                        .or_else(|| item.get("mime_type"))
                        .and_then(Value::as_str)
                        .unwrap_or("image/png");
                    let bytes = STANDARD
                        .decode(data)
                        .context("failed to decode MCP image result")?;
                    parts.push(UnsavedMessagePart::Image(UnsavedImagePart {
                        mime_type: mime_type.to_string(),
                        bytes,
                    }));
                }
                Some(other) => parts.push(UnsavedMessagePart::Text(format!(
                    "Unsupported MCP content block: {other}"
                ))),
                None => {}
            }
        }
    }

    if let Some(structured_content) = result.get("structuredContent")
        && !structured_content.is_null()
    {
        parts.push(UnsavedMessagePart::Text(format!(
            "structuredContent: {structured_content}"
        )));
    }

    if parts.is_empty() {
        parts.push(UnsavedMessagePart::Text(result.to_string()));
    }

    Ok(parts)
}

/// Builds the compact visible text stored on the tool message row.
pub(in crate::tool_provider) fn tool_result_preview(parts: &[UnsavedMessagePart]) -> String {
    let mut lines = Vec::new();

    for part in parts {
        match part {
            UnsavedMessagePart::Text(text) => lines.push(text.clone()),
            UnsavedMessagePart::Image(image) => lines.push(format!(
                "[image: {}, {} bytes]",
                image.mime_type,
                image.bytes.len()
            )),
        }
    }

    lines.join("\n")
}
