//! Built-in tool schema catalog.
//!
//! This module owns the read-only list of native tool schemas Windie can
//! advertise to clients. The schemas are templates only: clients must
//! explicitly persist a schema on a conversation before runtime queries send it
//! to Bifrost.

use serde_json::json;

use crate::conversation::{ToolSchema, ToolSchemaName};

/// Returns the built-in tool schemas a client may attach to a conversation.
///
/// Returning a schema here does not put the tool into a model request and does
/// not authorize execution. The conversation store remains the boundary that
/// decides which schemas are active for one conversation.
pub fn available_tool_schemas() -> Vec<ToolSchema> {
    vec![run_shell_tool_schema()]
}

/// Builds the model-facing schema for Windie's bounded local shell executor.
///
/// The JSON Schema mirrors `ShellCommand` in `shell.rs`: `command` is required,
/// while `cwd` and `timeout_ms` are optional controls interpreted by the local
/// executor after the approval boundary.
fn run_shell_tool_schema() -> ToolSchema {
    ToolSchema {
        name: ToolSchemaName::new("run_shell"),
        description: "Run a bounded local shell command after explicit user approval.".to_string(),
        parameters: json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "command": {
                    "type": "string",
                    "description": "Shell command to run through the user's default shell."
                },
                "cwd": {
                    "type": "string",
                    "description": "Optional working directory for the command."
                },
                "timeout_ms": {
                    "type": "integer",
                    "description": "Optional timeout in milliseconds, capped by Windie."
                }
            },
            "required": ["command"]
        }),
    }
}
