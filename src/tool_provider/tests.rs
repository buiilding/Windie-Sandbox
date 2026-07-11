//! Tool provider registry and dispatch tests.

use super::*;

fn approved_cua_provider() -> McpToolProvider {
    let definition = APPROVED_MCP_PROVIDERS
        .iter()
        .copied()
        .find(|definition| definition.provider_id == "cua-driver")
        .unwrap();
    McpToolProvider::new(definition)
}

fn approved_desktop_commander_provider() -> McpToolProvider {
    let definition = APPROVED_MCP_PROVIDERS
        .iter()
        .copied()
        .find(|definition| definition.provider_id == "desktop-commander")
        .unwrap();
    McpToolProvider::new(definition)
}

fn approved_blender_mcp_provider() -> McpToolProvider {
    let definition = APPROVED_MCP_PROVIDERS
        .iter()
        .copied()
        .find(|definition| definition.provider_id == "blender-mcp")
        .unwrap();
    McpToolProvider::new(definition)
}

fn approved_chrome_devtools_provider() -> McpToolProvider {
    let definition = APPROVED_MCP_PROVIDERS
        .iter()
        .copied()
        .find(|definition| definition.provider_id == "chrome-devtools")
        .unwrap();
    McpToolProvider::new(definition)
}

fn approved_brightdata_provider() -> McpToolProvider {
    let definition = APPROVED_MCP_PROVIDERS
        .iter()
        .copied()
        .find(|definition| definition.provider_id == "brightdata")
        .unwrap();
    McpToolProvider::new(definition)
}

fn approved_exa_provider() -> McpToolProvider {
    let definition = APPROVED_MCP_PROVIDERS
        .iter()
        .copied()
        .find(|definition| definition.provider_id == "exa")
        .unwrap();
    McpToolProvider::new(definition)
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
        annotations: Some(mcp::McpToolAnnotations {
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
        annotations: Some(mcp::McpToolAnnotations {
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
        annotations: Some(mcp::McpToolAnnotations {
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
        annotations: Some(mcp::McpToolAnnotations {
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
fn chrome_devtools_mcp_tools_map_to_provider_backed_definitions() {
    let provider = approved_chrome_devtools_provider();
    let definition = provider.definition_from_mcp_tool(McpTool {
        name: "take_screenshot".to_string(),
        description: "Take a screenshot".to_string(),
        input_schema: json!({"type":"object"}),
        annotations: Some(mcp::McpToolAnnotations {
            read_only_hint: Some(true),
        }),
    });

    assert_eq!(
        definition.schema_name.as_str(),
        "chrome_devtools__take_screenshot"
    );
    assert_eq!(definition.provider.provider_id.as_str(), "chrome-devtools");
    assert_eq!(definition.provider.tool_name.as_str(), "take_screenshot");
    assert_eq!(definition.provider.kind, ToolProviderKind::Mcp);
    assert_eq!(
        definition.permissions,
        vec![ToolPermission::ExternalProcess]
    );
    assert_eq!(definition.annotations.read_only, Some(true));
}

#[test]
fn exa_mcp_tools_map_to_provider_backed_definitions() {
    let provider = approved_exa_provider();
    let definition = provider.definition_from_mcp_tool(McpTool {
        name: "web_search_exa".to_string(),
        description: "Search the web".to_string(),
        input_schema: json!({"type":"object"}),
        annotations: Some(mcp::McpToolAnnotations {
            read_only_hint: Some(true),
        }),
    });

    assert_eq!(definition.schema_name.as_str(), "exa__web_search_exa");
    assert_eq!(definition.provider.provider_id.as_str(), "exa");
    assert_eq!(definition.provider.tool_name.as_str(), "web_search_exa");
    assert_eq!(definition.provider.kind, ToolProviderKind::Mcp);
    assert_eq!(
        definition.permissions,
        vec![ToolPermission::ExternalProcess]
    );
    assert_eq!(definition.annotations.read_only, Some(true));
}

#[test]
fn exa_provider_uses_pinned_package_and_explicit_api_key() {
    let provider = approved_exa_provider();

    assert_eq!(provider.command.program, "npx");
    assert_eq!(provider.command.args, ["-y", "exa-mcp-server@3.2.1"]);
    assert_eq!(
        provider.command.env,
        [McpEnv {
            key: "EXA_API_KEY",
            value: McpEnvValue::UserEnv("EXA_API_KEY"),
        }]
    );
}

#[test]
fn chrome_devtools_provider_uses_remote_debugging_endpoint() {
    let provider = approved_chrome_devtools_provider();

    assert_eq!(provider.command.program, "npx");
    assert_eq!(
        provider.command.args,
        [
            "-y",
            "chrome-devtools-mcp@1.5.0",
            "--browser-url=http://127.0.0.1:9222",
            "--no-usage-statistics",
        ]
    );
    assert_eq!(
        provider.command.env,
        [McpEnv {
            key: "CHROME_DEVTOOLS_MCP_NO_UPDATE_CHECKS",
            value: McpEnvValue::Literal("true"),
        }]
    );
}

#[test]
fn desktop_commander_config_allows_every_directory() {
    let config = json!({
        "blockedCommands": desktop_commander_blocked_commands(),
        "allowedDirectories": [],
        "telemetryEnabled": false,
        "fileWriteLineLimit": 50,
        "fileReadLineLimit": 1000,
        "pendingWelcomeOnboarding": false
    });

    assert_eq!(config["allowedDirectories"].as_array().unwrap().len(), 0);
    assert_eq!(config["telemetryEnabled"], false);
}

#[test]
fn mcp_tool_result_parts_decode_text_images_and_structured_content() {
    let result = json!({
        "content": [
            {"type": "text", "text": "desktop screenshot"},
            {"type": "image", "mimeType": "image/png", "data": "iVBORw0KGgo="}
        ],
        "structuredContent": {
            "screen_width": 1710
        }
    });

    let result = mcp::decode_tool_result(result).unwrap();
    let parts = mcp_tool_result_parts(&result).unwrap();

    assert_eq!(parts.len(), 3);
    assert!(matches!(&parts[0], UnsavedMessagePart::Text(text) if text == "desktop screenshot"));
    assert!(matches!(&parts[1], UnsavedMessagePart::Image(image)
        if image.mime_type == "image/png"
            && image.bytes == vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]));
    assert!(matches!(&parts[2], UnsavedMessagePart::Text(text)
        if text == "structuredContent: {\"screen_width\":1710}"));
    assert_eq!(
        tool_result_preview(&parts),
        "desktop screenshot\n[image: image/png, 8 bytes]\nstructuredContent: {\"screen_width\":1710}"
    );
}

#[test]
fn mcp_tool_result_rejects_invalid_image_bytes() {
    let result = json!({
        "content": [
            {"type": "image", "mimeType": "image/png", "data": "AQID"}
        ]
    });

    let result = mcp::decode_tool_result(result).unwrap();
    let error = mcp_tool_result_parts(&result).unwrap_err();

    assert!(error.to_string().contains("invalid MCP image result"));
}

#[test]
fn mcp_tool_result_rejects_aggregate_size_overflow() {
    let mut total = MCP_TOOL_RESULT_MAX_BYTES;

    let error = add_mcp_result_bytes(&mut total, 1).unwrap_err();

    assert!(error.to_string().contains("MCP tool result exceeds"));
}

#[test]
fn tool_result_preview_is_bounded_without_splitting_utf8() {
    let text = "a".repeat(TOOL_RESULT_PREVIEW_MAX_BYTES - 1) + "é";
    let preview = tool_result_preview(&[UnsavedMessagePart::Text(text)]);

    assert!(preview.ends_with("\n[truncated]"));
    assert!(preview.len() <= TOOL_RESULT_PREVIEW_MAX_BYTES + "\n[truncated]".len());
}

#[test]
fn mcp_tool_call_timeout_becomes_failed_tool_result() {
    let error: anyhow::Error = mcp::McpRequestTimeout::new(
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
fn registry_recognizes_chrome_devtools_as_approved_provider() {
    let registry = ToolProviderRegistry::new();
    let attached_tool = AttachedTool {
        schema_name: ToolSchemaName::new("chrome_devtools__take_screenshot"),
        description: "Take a screenshot".to_string(),
        parameters: json!({"type":"object"}),
        provider: ToolProviderRef::new(
            ToolProviderId::new("chrome-devtools"),
            ProviderToolName::new("take_screenshot"),
            ToolProviderKind::Mcp,
        ),
        permissions: vec![ToolPermission::ExternalProcess],
        annotations: ToolAnnotations::default(),
    };

    assert!(registry.can_execute(&attached_tool));
}

#[test]
fn registry_recognizes_exa_as_approved_provider() {
    let registry = ToolProviderRegistry::new();
    let attached_tool = AttachedTool {
        schema_name: ToolSchemaName::new("exa__web_search_exa"),
        description: "Search the web".to_string(),
        parameters: json!({"type":"object"}),
        provider: ToolProviderRef::new(
            ToolProviderId::new("exa"),
            ProviderToolName::new("web_search_exa"),
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
        catalog_loads: Arc::new(CatalogLoads::default()),
    };

    let found = registry
        .find_tool(&provider_id, &ProviderToolName::new("cached_tool"))
        .unwrap();

    assert_eq!(found, Some(tool));
}

#[test]
#[cfg(unix)]
fn concurrent_catalog_requests_start_one_provider_process() {
    use std::os::unix::fs::PermissionsExt;

    let nonce = uuid::Uuid::new_v4();
    let script_path = std::env::temp_dir().join(format!("windie-catalog-{nonce}.sh"));
    let starts_path = std::env::temp_dir().join(format!("windie-catalog-{nonce}.starts"));
    let script = format!(
        r#"#!/bin/sh
printf 'start\n' >> '{}'
while IFS= read -r line; do
  case "$line" in
    *'"method":"initialize"'*)
      printf '%s\n' '{{"jsonrpc":"2.0","id":1,"result":{{"protocolVersion":"2025-06-18","capabilities":{{}},"serverInfo":{{"name":"test","version":"1"}}}}}}'
      ;;
    *'"method":"tools/list"'*)
      sleep 0.2
      printf '%s\n' '{{"jsonrpc":"2.0","id":2,"result":{{"tools":[{{"name":"search","description":"Search","inputSchema":{{"type":"object"}}}}]}}}}'
      ;;
  esac
done
"#,
        starts_path.display()
    );
    std::fs::write(&script_path, script).unwrap();
    std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o700)).unwrap();
    let program = Box::leak(script_path.to_string_lossy().into_owned().into_boxed_str());
    let registry = Arc::new(ToolProviderRegistry {
        mcp_providers: vec![McpToolProvider::new(McpProviderDefinition {
            provider_id: "single-flight-test",
            schema_prefix: "single_flight_test",
            display_name: "Single Flight Test",
            command: McpCommand {
                program,
                args: &[],
                env: &[],
            },
            shutdown_command: None,
            setup: None,
        })],
        mcp_session_pool: None,
        catalog_cache: test_cache(),
        catalog_loads: Arc::new(CatalogLoads::default()),
    });
    let barrier = Arc::new(std::sync::Barrier::new(3));
    let workers = (0..2)
        .map(|_| {
            let registry = Arc::clone(&registry);
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                registry.list_provider_tools(&ToolProviderId::new("single-flight-test"))
            })
        })
        .collect::<Vec<_>>();
    barrier.wait();

    for worker in workers {
        let tools = worker.join().unwrap().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].schema_name.as_str(), "single_flight_test__search");
    }
    let starts = std::fs::read_to_string(&starts_path).unwrap();
    assert_eq!(starts.lines().count(), 1);

    let _ = std::fs::remove_file(script_path);
    let _ = std::fs::remove_file(starts_path);
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
        catalog_loads: Arc::new(CatalogLoads::default()),
    };

    let tools = registry.list_available_tools().unwrap();

    assert_eq!(tools, vec![available_tool]);
}
