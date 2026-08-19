//! Windie-owned model control tools.
//!
//! These tools are always available to the model. They do not represent an
//! installed provider and are not persisted in conversation tool schemas.
//! Their stateful behavior is executed by the runtime using the existing
//! provider registry and conversation attachment operations.

use serde_json::json;

use crate::tool::{
    ProviderToolName, ToolAnnotations, ToolDefinition, ToolPermission, ToolProviderId,
    ToolProviderKind, ToolProviderRef, ToolSchemaName,
};

/// Stable provider ID used by Windie-owned tools.
pub const BUILTIN_PROVIDER_ID: &str = "windie";

/// Provider-native name for reading one installed skill.
pub const READ_SKILL_TOOL_NAME: &str = "read_skill";

/// Provider-native name for attaching one plugin-owned MCP.
pub const ATTACH_MCP_TOOL_NAME: &str = "attach_mcp";

/// Returns Windie's control tools that are always sent to the model.
pub(super) fn definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            schema_name: ToolSchemaName::new("windie__read_skill"),
            display_name: "Windie read skill".to_string(),
            description: "Read the complete instructions for one skill listed inside an installed plugin. Use the exact plugin_id and skill_id from the current Installed plugins index. Skill instructions are not included in the index until you request them.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "plugin_id": {
                        "type": "string",
                        "description": "Exact installed plugin ID from the current plugin index."
                    },
                    "skill_id": {
                        "type": "string",
                        "description": "Exact skill ID nested inside that plugin."
                    }
                },
                "required": ["plugin_id", "skill_id"],
                "additionalProperties": false
            }),
            provider: builtin_ref(READ_SKILL_TOOL_NAME),
            permissions: Vec::<ToolPermission>::new(),
            annotations: ToolAnnotations {
                title: Some("Read skill".to_string()),
                read_only: Some(true),
            },
        },
        ToolDefinition {
            schema_name: ToolSchemaName::new("windie__attach_mcp"),
            display_name: "Windie attach plugin MCP".to_string(),
            description: "Attach the discovered tool schemas for one MCP nested inside an installed plugin. Use the exact plugin_id and mcp_id from the current Installed plugins index. After this succeeds, the MCP tools are available on the next model turn.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "plugin_id": {
                        "type": "string",
                        "description": "Exact installed plugin ID from the current plugin index."
                    },
                    "mcp_id": {
                        "type": "string",
                        "description": "Exact MCP component ID nested inside that plugin."
                    }
                },
                "required": ["plugin_id", "mcp_id"],
                "additionalProperties": false
            }),
            provider: builtin_ref(ATTACH_MCP_TOOL_NAME),
            permissions: Vec::<ToolPermission>::new(),
            annotations: ToolAnnotations::default(),
        },
    ]
}

fn builtin_ref(tool_name: &str) -> ToolProviderRef {
    ToolProviderRef::new(
        ToolProviderId::new(BUILTIN_PROVIDER_ID),
        ProviderToolName::new(tool_name),
        ToolProviderKind::Builtin,
    )
}
