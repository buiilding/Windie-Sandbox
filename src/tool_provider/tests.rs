//! Tests for tool provider catalog, MCP mapping, and result normalization.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use anyhow::anyhow;
use serde_json::{Value, json};

use super::ToolProviderRegistry;
use super::mcp::{
    McpProviderDefinition, McpToolProvider, approved_mcp_provider, mcp_schema_name,
    mcp_tool_call_failure_result, mcp_tool_result_parts, tool_result_preview,
};
use crate::conversation::{ToolCall, UnsavedMessagePart};
use crate::mcp::{self as mcp_protocol, McpCommand, McpTool};
use crate::tool::{
    AttachedTool, ProviderToolName, ToolAnnotations, ToolDefinition, ToolPermission,
    ToolProviderId, ToolProviderKind, ToolProviderRef, ToolSchemaName,
};
use crate::tool_provider::manifest::ProviderTransport;
use crate::tool_provider::{ProviderManifest, ProviderSecret};

fn approved_cua_provider() -> McpToolProvider {
    McpToolProvider::new(approved_mcp_provider("cua-driver").unwrap())
}

fn approved_desktop_commander_provider() -> McpToolProvider {
    McpToolProvider::new(approved_mcp_provider("desktop-commander").unwrap())
}

fn approved_blender_mcp_provider() -> McpToolProvider {
    McpToolProvider::new(approved_mcp_provider("blender-mcp").unwrap())
}

fn approved_brightdata_provider() -> McpToolProvider {
    McpToolProvider::new(approved_mcp_provider("brightdata").unwrap())
}

#[test]
fn approved_provider_manifests_describe_their_runtime_requirements() {
    let providers = [
        approved_cua_provider(),
        approved_desktop_commander_provider(),
        approved_blender_mcp_provider(),
        approved_brightdata_provider(),
    ];

    let ids = providers
        .iter()
        .map(|provider| provider.manifest().provider_id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        ids,
        vec![
            "cua-driver",
            "desktop-commander",
            "blender-mcp",
            "brightdata"
        ]
    );

    for provider in providers {
        let manifest = provider.manifest();
        assert_eq!(manifest.kind, ToolProviderKind::Mcp);
        assert_eq!(manifest.transport, ProviderTransport::Stdio);
        assert!(!manifest.description.is_empty());
        assert!(!manifest.launch.program.is_empty());
        assert!(!manifest.platforms.is_empty());
        assert!(!manifest.permissions.is_empty());
    }
}

#[test]
fn brightdata_manifest_declares_required_secret() {
    let provider = approved_brightdata_provider();
    let manifest = provider.manifest();

    assert_eq!(
        manifest.secrets,
        vec![ProviderSecret::required(
            "BRIGHTDATA_API_TOKEN",
            "Bright Data API token",
        )]
    );
}

fn test_cache() -> Arc<Mutex<HashMap<ToolProviderId, Vec<ToolDefinition>>>> {
    Arc::new(Mutex::new(HashMap::new()))
}

fn cached_test_tool(provider_id: &str, tool_name: &str) -> ToolDefinition {
    ToolDefinition {
        schema_name: ToolSchemaName::new(format!("{provider_id}__{tool_name}")),
        display_name: tool_name.to_string(),
        description: format!("{tool_name} description"),
        parameters: json!({"type":"object"}),
        provider: ToolProviderRef::new(
            ToolProviderId::new(provider_id),
            ProviderToolName::new(tool_name),
            ToolProviderKind::Mcp,
        ),
        permissions: vec![ToolPermission::ExternalProcess],
        annotations: ToolAnnotations::default(),
    }
}

#[test]
fn mcp_schema_names_are_provider_prefixed() {
    assert_eq!(mcp_schema_name("cua_driver", "click"), "cua_driver__click");
    assert_eq!(
        mcp_schema_name("cua_driver", "type text"),
        "cua_driver__type_text"
    );
}

#[test]
fn cua_mcp_tools_map_to_provider_backed_definitions() {
    let provider = approved_cua_provider();
    let definition = provider.definition_from_mcp_tool(McpTool {
        name: "click".to_string(),
        description: "Click somewhere".to_string(),
        input_schema: json!({"type":"object"}),
        annotations: Some(mcp_protocol::McpToolAnnotations {
            read_only_hint: Some(false),
        }),
    });

    assert_eq!(definition.schema_name.as_str(), "cua_driver__click");
    assert_eq!(definition.provider.provider_id.as_str(), "cua-driver");
    assert_eq!(definition.provider.tool_name.as_str(), "click");
    assert_eq!(definition.provider.kind, ToolProviderKind::Mcp);
    assert_eq!(
        definition.permissions,
        vec![ToolPermission::ExternalProcess]
    );
    assert_eq!(definition.annotations.read_only, Some(false));
}

#[test]
fn desktop_commander_mcp_tools_map_to_provider_backed_definitions() {
    let provider = approved_desktop_commander_provider();
    let definition = provider.definition_from_mcp_tool(McpTool {
        name: "read_file".to_string(),
        description: "Read a file".to_string(),
        input_schema: json!({"type":"object"}),
        annotations: Some(mcp_protocol::McpToolAnnotations {
            read_only_hint: Some(true),
        }),
    });

    assert_eq!(
        definition.schema_name.as_str(),
        "desktop_commander__read_file"
    );
    assert_eq!(
        definition.provider.provider_id.as_str(),
        "desktop-commander"
    );
    assert_eq!(definition.provider.tool_name.as_str(), "read_file");
    assert_eq!(definition.provider.kind, ToolProviderKind::Mcp);
    assert_eq!(
        definition.permissions,
        vec![ToolPermission::ExternalProcess]
    );
    assert_eq!(definition.annotations.read_only, Some(true));
}

#[test]
fn blender_mcp_tools_map_to_provider_backed_definitions() {
    let provider = approved_blender_mcp_provider();
    let definition = provider.definition_from_mcp_tool(McpTool {
        name: "get_scene_info".to_string(),
        description: "Get scene info".to_string(),
        input_schema: json!({"type":"object"}),
        annotations: Some(mcp_protocol::McpToolAnnotations {
            read_only_hint: Some(true),
        }),
    });

    assert_eq!(
        definition.schema_name.as_str(),
        "blender_mcp__get_scene_info"
    );
    assert_eq!(definition.provider.provider_id.as_str(), "blender-mcp");
    assert_eq!(definition.provider.tool_name.as_str(), "get_scene_info");
    assert_eq!(definition.provider.kind, ToolProviderKind::Mcp);
    assert_eq!(
        definition.permissions,
        vec![ToolPermission::ExternalProcess]
    );
    assert_eq!(definition.annotations.read_only, Some(true));
}

#[test]
fn brightdata_mcp_tools_map_to_provider_backed_definitions() {
    let provider = approved_brightdata_provider();
    let definition = provider.definition_from_mcp_tool(McpTool {
        name: "search_engine".to_string(),
        description: "Search live web results".to_string(),
        input_schema: json!({"type":"object"}),
        annotations: Some(mcp_protocol::McpToolAnnotations {
            read_only_hint: Some(true),
        }),
    });

    assert_eq!(definition.schema_name.as_str(), "brightdata__search_engine");
    assert_eq!(definition.provider.provider_id.as_str(), "brightdata");
    assert_eq!(definition.provider.tool_name.as_str(), "search_engine");
    assert_eq!(definition.provider.kind, ToolProviderKind::Mcp);
    assert_eq!(
        definition.permissions,
        vec![ToolPermission::ExternalProcess]
    );
    assert_eq!(definition.annotations.read_only, Some(true));
}

#[test]
fn desktop_commander_config_allows_every_directory() {
    let config = json!({
        "allowedDirectories": [],
        "telemetryEnabled": false,
    });

    assert_eq!(config["allowedDirectories"].as_array().unwrap().len(), 0);
    assert_eq!(config["telemetryEnabled"], false);
}

#[test]
fn mcp_tool_result_parts_decode_text_images_and_structured_content() {
    let result = json!({
        "content": [
            {"type": "text", "text": "desktop screenshot"},
            {"type": "image", "mimeType": "image/png", "data": "AQID"}
        ],
        "structuredContent": {
            "screen_width": 1710
        }
    });

    let parts = mcp_tool_result_parts(&result).unwrap();

    assert_eq!(parts.len(), 3);
    assert!(matches!(&parts[0], UnsavedMessagePart::Text(text) if text == "desktop screenshot"));
    assert!(matches!(&parts[1], UnsavedMessagePart::Image(image)
        if image.mime_type == "image/png" && image.bytes == vec![1, 2, 3]));
    assert!(matches!(&parts[2], UnsavedMessagePart::Text(text)
        if text == "structuredContent: {\"screen_width\":1710}"));
    assert_eq!(
        tool_result_preview(&parts),
        "desktop screenshot\n[image: image/png, 3 bytes]\nstructuredContent: {\"screen_width\":1710}"
    );
}

#[test]
fn mcp_tool_call_timeout_becomes_failed_tool_result() {
    let error: anyhow::Error = mcp_protocol::McpRequestTimeout::new(
        "desktop-commander",
        "tools/call",
        std::time::Duration::from_secs(300),
    )
    .into();
    let tool_call = ToolCall::function("call_123", "desktop_commander__read_file", "{}");

    let result = mcp_tool_call_failure_result(
        &ToolProviderId::new("desktop-commander"),
        &tool_call,
        &error,
    );
    let content = serde_json::from_str::<Value>(&result.content).unwrap();

    assert!(!result.success);
    assert_eq!(result.tool_call_id.as_str(), "call_123");
    assert_eq!(result.tool_name, "desktop_commander__read_file");
    assert_eq!(content["error"], "MCP provider timed out");
    assert_eq!(content["provider"], "desktop-commander");
    assert_eq!(content["method"], "tools/call");
    assert_eq!(content["timeout_ms"], 300_000);
    assert_eq!(content["timeout_seconds"], 300);
}

#[test]
fn mcp_tool_call_process_error_becomes_failed_tool_result() {
    let error = anyhow!("provider exited early");
    let tool_call = ToolCall::function("call_123", "desktop_commander__read_file", "{}");

    let result = mcp_tool_call_failure_result(
        &ToolProviderId::new("desktop-commander"),
        &tool_call,
        &error,
    );
    let content = serde_json::from_str::<Value>(&result.content).unwrap();

    assert!(!result.success);
    assert_eq!(content["error"], "MCP provider tool call failed");
    assert_eq!(content["detail"], "provider exited early");
    assert_eq!(content["provider"], "desktop-commander");
    assert_eq!(content["method"], "tools/call");
}

#[test]
fn registry_executes_only_approved_mcp_provider_ids() {
    let registry = ToolProviderRegistry::new();
    let attached_tool = AttachedTool {
        schema_name: ToolSchemaName::new("other__click"),
        description: "Click somewhere".to_string(),
        parameters: json!({"type":"object"}),
        provider: ToolProviderRef::new(
            ToolProviderId::new("other-mcp"),
            ProviderToolName::new("click"),
            ToolProviderKind::Mcp,
        ),
        permissions: vec![ToolPermission::ExternalProcess],
        annotations: ToolAnnotations::default(),
    };

    assert!(!registry.can_execute(&attached_tool));
}

#[test]
fn registry_recognizes_cua_driver_as_approved_mcp_provider() {
    let registry = ToolProviderRegistry::new();
    let attached_tool = AttachedTool {
        schema_name: ToolSchemaName::new("cua_driver__click"),
        description: "Click somewhere".to_string(),
        parameters: json!({"type":"object"}),
        provider: ToolProviderRef::new(
            ToolProviderId::new("cua-driver"),
            ProviderToolName::new("click"),
            ToolProviderKind::Mcp,
        ),
        permissions: vec![ToolPermission::ExternalProcess],
        annotations: ToolAnnotations::default(),
    };

    assert!(registry.can_execute(&attached_tool));
}

#[test]
fn registry_recognizes_blender_mcp_as_approved_provider() {
    let registry = ToolProviderRegistry::new();
    let attached_tool = AttachedTool {
        schema_name: ToolSchemaName::new("blender_mcp__get_scene_info"),
        description: "Get scene info".to_string(),
        parameters: json!({"type":"object"}),
        provider: ToolProviderRef::new(
            ToolProviderId::new("blender-mcp"),
            ProviderToolName::new("get_scene_info"),
            ToolProviderKind::Mcp,
        ),
        permissions: vec![ToolPermission::ExternalProcess],
        annotations: ToolAnnotations::default(),
    };

    assert!(registry.can_execute(&attached_tool));
}

#[test]
fn registry_recognizes_brightdata_as_approved_provider() {
    let registry = ToolProviderRegistry::new();
    let attached_tool = AttachedTool {
        schema_name: ToolSchemaName::new("brightdata__search_engine"),
        description: "Search live web results".to_string(),
        parameters: json!({"type":"object"}),
        provider: ToolProviderRef::new(
            ToolProviderId::new("brightdata"),
            ProviderToolName::new("search_engine"),
            ToolProviderKind::Mcp,
        ),
        permissions: vec![ToolPermission::ExternalProcess],
        annotations: ToolAnnotations::default(),
    };

    assert!(registry.can_execute(&attached_tool));
}

#[test]
fn registry_finds_tools_from_cached_provider_catalog() {
    let provider_id = ToolProviderId::new("missing-mcp");
    let tool = cached_test_tool(provider_id.as_str(), "cached_tool");
    let catalog_cache = test_cache();
    catalog_cache
        .lock()
        .unwrap()
        .insert(provider_id.clone(), vec![tool.clone()]);
    let registry = ToolProviderRegistry {
        mcp_providers: vec![McpToolProvider::new(McpProviderDefinition {
            manifest: ProviderManifest::mcp_stdio(
                "missing-mcp",
                "Missing MCP",
                "Test MCP provider.",
                "windie-missing-mcp-provider",
                &[],
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
            ),
            provider_id: "missing-mcp",
            schema_prefix: "missing_mcp",
            display_name: "Missing MCP",
            command: McpCommand {
                program: "windie-missing-mcp-provider",
                args: &[],
                env: &[],
            },
            shutdown_command: None,
            setup: None,
        })],
        mcp_session_pool: None,
        catalog_cache,
    };

    let found = registry
        .find_tool(&provider_id, &ProviderToolName::new("cached_tool"))
        .unwrap();

    assert_eq!(found, Some(tool));
}

#[test]
fn unavailable_mcp_provider_does_not_hide_other_provider_tools() {
    let available_provider_id = ToolProviderId::new("available-mcp");
    let available_tool = cached_test_tool(available_provider_id.as_str(), "cached_tool");
    let catalog_cache = test_cache();
    catalog_cache
        .lock()
        .unwrap()
        .insert(available_provider_id, vec![available_tool.clone()]);
    let registry = ToolProviderRegistry {
        mcp_providers: vec![
            McpToolProvider::new(McpProviderDefinition {
                manifest: ProviderManifest::mcp_stdio(
                    "available-mcp",
                    "Available MCP",
                    "Test MCP provider.",
                    "windie-missing-mcp-provider",
                    &[],
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                ),
                provider_id: "available-mcp",
                schema_prefix: "available_mcp",
                display_name: "Available MCP",
                command: McpCommand {
                    program: "windie-missing-mcp-provider",
                    args: &[],
                    env: &[],
                },
                shutdown_command: None,
                setup: None,
            }),
            McpToolProvider::new(McpProviderDefinition {
                manifest: ProviderManifest::mcp_stdio(
                    "missing-mcp",
                    "Missing MCP",
                    "Test MCP provider.",
                    "windie-missing-mcp-provider",
                    &[],
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                ),
                provider_id: "missing-mcp",
                schema_prefix: "missing_mcp",
                display_name: "Missing MCP",
                command: McpCommand {
                    program: "windie-missing-mcp-provider",
                    args: &[],
                    env: &[],
                },
                shutdown_command: None,
                setup: None,
            }),
        ],
        mcp_session_pool: None,
        catalog_cache,
    };

    let tools = registry.list_available_tools().unwrap();

    assert_eq!(tools, vec![available_tool]);
}
