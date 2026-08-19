//! Tests for tool provider catalog, MCP mapping, and result normalization.

use anyhow::anyhow;
use serde_json::{Value, json};

use super::ToolProviderRegistry;
use crate::conversation::{ToolCall, UnsavedMessagePart};
use crate::mcp::ChromeDevToolsConnectionMode;
use crate::mcp::{self as mcp_protocol, McpTransport};
use crate::mcp::{
    mcp_schema_name, mcp_tool_call_failure_result, mcp_tool_result_parts, tool_result_preview,
};
use crate::plugin::PluginStore;
use crate::tool::{
    AttachedTool, ProviderToolName, ToolAnnotations, ToolPermission, ToolProviderId,
    ToolProviderKind, ToolProviderRef, ToolSchemaName,
};

#[test]
fn packaged_chrome_devtools_mode_updates_package_owned_transport() {
    let root = std::env::temp_dir().join(format!(
        "windie-packaged-chrome-mode-test-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    let store = PluginStore::new(&root);
    let plugin = store.install_bundled("chrome-devtools").unwrap();
    let registry = ToolProviderRegistry::new();
    registry.register_plugin(&plugin).unwrap();

    registry
        .set_chrome_devtools_mode(ChromeDevToolsConnectionMode::Existing)
        .unwrap();
    let provider = registry
        .mcp_provider(&ToolProviderId::new("chrome-devtools"))
        .unwrap();
    let McpTransport::PackagedStdio { command, .. } = provider.transport() else {
        panic!("installed Chrome DevTools should retain its packaged transport");
    };
    assert!(
        command.env.iter().any(|(name, value)| {
            name == "WINDIE_CHROME_CONNECTION_MODE" && value == "existing"
        })
    );

    registry.unregister_plugin(&plugin).unwrap();
    store.remove_plugin("chrome-devtools").unwrap();
    std::fs::remove_dir_all(root).unwrap();
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
fn registry_recognizes_cua_driver_after_package_registration() {
    let root = std::env::temp_dir().join(format!(
        "windie-cua-driver-registry-test-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    let store = PluginStore::new(&root);
    let plugin = store.install_bundled("cua-driver").unwrap();
    let registry = ToolProviderRegistry::new();
    registry.register_plugin(&plugin).unwrap();
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
    registry.unregister_plugin(&plugin).unwrap();
    store.remove_plugin("cua-driver").unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn registry_recognizes_brightdata_after_package_registration() {
    let root = std::env::temp_dir().join(format!(
        "windie-brightdata-registry-test-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    let store = PluginStore::new(&root);
    let plugin = store.install_bundled("brightdata").unwrap();
    let registry = ToolProviderRegistry::new();
    registry.register_plugin(&plugin).unwrap();
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
    registry.unregister_plugin(&plugin).unwrap();
    store.remove_plugin("brightdata").unwrap();
    std::fs::remove_dir_all(root).unwrap();
}
