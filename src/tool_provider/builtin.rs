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

/// Provider-native name for provider discovery.
pub const LIST_PROVIDERS_TOOL_NAME: &str = "list_providers";

/// Provider-native name for provider attachment.
pub const ATTACH_PROVIDER_TOOL_NAME: &str = "attach_provider";

/// Returns the two Windie-owned tools that are always sent to the model.
pub(super) fn definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            schema_name: ToolSchemaName::new("windie__list_providers"),
            display_name: "Windie list providers".to_string(),
            description: "List all currently available tool providers and the capabilities they expose. You must call this tool before concluding that you cannot perform a task, lack access to a capability, or need the user to do something manually. Inspect the returned providers to determine whether any of them can help complete the current task.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
            provider: builtin_ref(LIST_PROVIDERS_TOOL_NAME),
            permissions: Vec::<ToolPermission>::new(),
            annotations: ToolAnnotations {
                title: Some("List providers".to_string()),
                read_only: Some(true),
            },
        },
        ToolDefinition {
            schema_name: ToolSchemaName::new("windie__attach_provider"),
            display_name: "Windie attach provider".to_string(),
            description: "Attach a provider returned by the most recent windie__list_providers call. When an available provider could perform or contribute to the current task, you must call this tool before refusing, claiming the task is unsupported, or falling back to manual instructions. Select the provider using its exact identifier from the latest provider list. After this succeeds, use the provider's newly available tools on the next turn.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "provider_id": {
                        "type": "string",
                        "description": "Use the exact provider_id returned by the most recent windie__list_providers call. Never invent or modify a provider ID."
                    }
                },
                "required": ["provider_id"],
                "additionalProperties": false
            }),
            provider: builtin_ref(ATTACH_PROVIDER_TOOL_NAME),
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
