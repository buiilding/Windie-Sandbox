//! Shared operation orchestration tests.

use super::*;

#[test]
fn runtime_snapshot_is_immutable_after_configuration_changes() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation("openai/first").unwrap();
    store
        .set_conversation_reasoning_effort(&conversation_id, Some("medium"))
        .unwrap();
    store
        .set_system_prompt(&conversation_id, "first prompt")
        .unwrap();
    store
        .set_tool_approval_mode(&conversation_id, ToolApprovalMode::AutoApproveAttached)
        .unwrap();
    store
        .insert_tool_schema(
            &conversation_id,
            &ToolSchema {
                name: ToolSchemaName::new("first_tool"),
                description: "first".to_string(),
                parameters: serde_json::json!({"type": "object"}),
            },
        )
        .unwrap();

    let (model, reasoning, snapshot) =
        capture_runtime_snapshot(&store, &conversation_id, None, None).unwrap();

    store
        .set_conversation_model(&conversation_id, "openai/second")
        .unwrap();
    store
        .set_system_prompt(&conversation_id, "second prompt")
        .unwrap();
    store
        .set_tool_approval_mode(&conversation_id, ToolApprovalMode::Manual)
        .unwrap();
    store
        .remove_tool_schema(&conversation_id, &ToolSchemaName::new("first_tool"))
        .unwrap();

    assert_eq!(model.as_str(), "openai/first");
    assert_eq!(reasoning.unwrap().effort.as_deref(), Some("medium"));
    assert_eq!(snapshot.system_prompt.as_deref(), Some("first prompt"));
    assert_eq!(
        snapshot.approval_mode,
        ToolApprovalMode::AutoApproveAttached
    );
    assert_eq!(snapshot.attached_tools.len(), 1);
    assert_eq!(
        snapshot.attached_tools[0].schema_name.as_str(),
        "first_tool"
    );
}
use crate::conversation::{MessageMetadata, ToolCall};
use crate::mcp::McpCommand;
use crate::tool::{ToolAnnotations, ToolPermission, ToolProviderKind, ToolProviderRef};
use std::fs;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

#[test]
fn inserts_text_message() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = create_conversation(&store, &ModelName::new("openai/test")).unwrap();

    let message_id = insert_message(
        &mut store,
        &conversation_id,
        Role::User,
        &[MessageInputPart::Text("hello".to_string())],
    )
    .unwrap();

    let messages = active_path(&store, &conversation_id).unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].id.as_ref(), Some(&message_id));
    assert_eq!(messages[0].content, "hello");
}

#[test]
fn builds_conversation_prompt_cache_request() {
    let conversation_id = ConversationId::new("conversation-id");

    let prompt_cache = conversation_prompt_cache_request(&conversation_id);

    assert_eq!(prompt_cache.key, "windie:conversation-id");
    assert_eq!(prompt_cache.retention.as_deref(), Some("24h"));
}

#[test]
fn persisted_reasoning_resolves_without_request_override() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = create_conversation(&store, &ModelName::new("openai/test")).unwrap();
    set_conversation_reasoning_effort(&mut store, &conversation_id, Some("medium")).unwrap();

    let reasoning = resolve_reasoning_request(&store, &conversation_id, None).unwrap();

    assert_eq!(reasoning.unwrap().effort.as_deref(), Some("medium"));
}

#[test]
fn request_reasoning_overrides_persisted_reasoning() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = create_conversation(&store, &ModelName::new("openai/test")).unwrap();
    set_conversation_reasoning_effort(&mut store, &conversation_id, Some("medium")).unwrap();

    let reasoning = resolve_reasoning_request(
        &store,
        &conversation_id,
        Some(ReasoningRequest {
            effort: Some("high".to_string()),
            summary: None,
        }),
    )
    .unwrap();

    assert_eq!(reasoning.unwrap().effort.as_deref(), Some("high"));
}

#[test]
fn rejects_direct_tool_message_insert() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = create_conversation(&store, &ModelName::new("openai/test")).unwrap();

    let error = insert_message(
        &mut store,
        &conversation_id,
        Role::Tool,
        &[MessageInputPart::Text("tool output".to_string())],
    )
    .unwrap_err();

    assert_eq!(
        error.to_string(),
        "role: tool messages must be created through approve or deny"
    );
}

#[test]
fn inserts_multi_part_message() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = create_conversation(&store, &ModelName::new("openai/test")).unwrap();
    let image_path = temp_image_path("png");
    fs::write(
        &image_path,
        [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a],
    )
    .unwrap();

    insert_message(
        &mut store,
        &conversation_id,
        Role::User,
        &[
            MessageInputPart::Text("first".to_string()),
            MessageInputPart::ImagePath(image_path.clone()),
        ],
    )
    .unwrap();

    let messages = active_path(&store, &conversation_id).unwrap();
    assert_eq!(messages[0].content, "first");
    assert_eq!(messages[0].parts.len(), 2);
    fs::remove_file(image_path).unwrap();
}

#[test]
fn inserts_loaded_image_bytes() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = create_conversation(&store, &ModelName::new("openai/test")).unwrap();

    insert_message(
        &mut store,
        &conversation_id,
        Role::User,
        &[
            MessageInputPart::Text("clipboard".to_string()),
            MessageInputPart::ImageBytes {
                mime_type: "image/png".to_string(),
                bytes: vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a],
            },
        ],
    )
    .unwrap();

    let messages = active_path(&store, &conversation_id).unwrap();
    assert_eq!(messages[0].content, "clipboard");
    assert_eq!(messages[0].parts.len(), 2);
}

#[test]
fn input_token_context_uses_synthetic_input_for_tool_only_setup() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = create_conversation(&store, &ModelName::new("openai/test")).unwrap();
    insert_tool_schema(
        &mut store,
        &conversation_id,
        &ToolSchema {
            name: ToolSchemaName::new("run_shell"),
            description: "Run a shell command".to_string(),
            parameters: serde_json::json!({"type":"object"}),
        },
    )
    .unwrap();

    let context = conversation_input_token_context(&store, &conversation_id)
        .unwrap()
        .unwrap();

    assert_eq!(
        context.source(),
        InputTokenCountSource::PrequerySyntheticInput
    );
    assert_eq!(context.model_messages.len(), 1);
    assert_eq!(context.model_messages[0].role, Role::System);
    assert_eq!(
        context.model_messages[0].content,
        SYNTHETIC_INPUT_TOKEN_COUNT_MESSAGE
    );
    assert_eq!(context.tool_schemas.len(), 1);
}

#[test]
fn inspection_snapshot_includes_runtime_state() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = create_conversation(&store, &ModelName::new("openai/test")).unwrap();
    set_conversation_model(
        &mut store,
        &conversation_id,
        &ModelName::new("anthropic/test"),
    )
    .unwrap();
    set_conversation_reasoning_effort(&mut store, &conversation_id, Some("high")).unwrap();
    set_system_prompt(&mut store, &conversation_id, "You are concise.").unwrap();
    let user_id = insert_message(
        &mut store,
        &conversation_id,
        Role::User,
        &[MessageInputPart::Text("hello".to_string())],
    )
    .unwrap();
    let tool_schema = ToolSchema {
        name: ToolSchemaName::new("run_shell"),
        description: "Run a shell command".to_string(),
        parameters: serde_json::json!({"type":"object"}),
    };
    insert_tool_schema(&mut store, &conversation_id, &tool_schema).unwrap();
    store
        .save_compaction(&conversation_id, &user_id, "hello happened")
        .unwrap();

    let report = inspect_conversation(&store, &conversation_id, None).unwrap();
    let value = serde_json::to_value(report).unwrap();

    assert_eq!(value["conversation_id"], conversation_id.as_str());
    assert_eq!(value["active_message_id"], user_id.as_str());
    assert_eq!(value["model"], "anthropic/test");
    assert_eq!(value["reasoning"]["effort"], "high");
    assert_eq!(value["system_prompt"], "You are concise.");
    assert_eq!(value["tool_schemas"][0]["name"], "run_shell");
    assert_eq!(value["messages"][0]["id"], user_id.as_str());
    assert_eq!(value["active_path"][0]["id"], user_id.as_str());
    assert_eq!(value["model_context"][0]["role"], "system");
    assert_eq!(value["latest_compaction"]["content"], "hello happened");
}

#[test]
fn attaches_available_provider_tool() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = create_conversation(&store, &ModelName::new("openai/test")).unwrap();
    let registry = registry_with_cached_test_tool();
    let read_file = registry
        .find_tool(
            &ToolProviderId::new("desktop-commander"),
            &ProviderToolName::new("read_file"),
        )
        .unwrap()
        .unwrap();

    let schema_name = attach_tool_with_registry(
        &mut store,
        &conversation_id,
        &ToolProviderId::new("desktop-commander"),
        &ProviderToolName::new("read_file"),
        &registry,
    )
    .unwrap();
    let attached_tools = store.load_attached_tools(&conversation_id).unwrap();

    assert_eq!(read_file.schema_name, schema_name);
    assert_eq!(schema_name.as_str(), "desktop_commander__read_file");
    assert_eq!(attached_tools.len(), 1);
    assert_eq!(
        attached_tools[0].provider.provider_id.as_str(),
        "desktop-commander"
    );
    assert_eq!(attached_tools[0].provider.tool_name.as_str(), "read_file");
}

#[test]
fn batch_attaches_available_provider_tools() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = create_conversation(&store, &ModelName::new("openai/test")).unwrap();

    let registry = registry_with_cached_test_tool();
    let schema_names = attach_tools_with_registry(
        &mut store,
        &conversation_id,
        &[ToolAttachmentInput::new(
            ToolProviderId::new("desktop-commander"),
            ProviderToolName::new("read_file"),
        )],
        &registry,
    )
    .unwrap();
    let attached_tools = store.load_attached_tools(&conversation_id).unwrap();

    assert_eq!(schema_names.len(), 1);
    assert_eq!(schema_names[0].as_str(), "desktop_commander__read_file");
    assert_eq!(attached_tools.len(), 1);
    assert_eq!(
        attached_tools[0].provider.provider_id.as_str(),
        "desktop-commander"
    );
    assert_eq!(attached_tools[0].provider.tool_name.as_str(), "read_file");
}

#[test]
fn shared_operations_match_direct_store_state() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = create_conversation(&store, &ModelName::new("openai/test")).unwrap();
    let first_id = insert_message(
        &mut store,
        &conversation_id,
        Role::User,
        &[MessageInputPart::Text("first".to_string())],
    )
    .unwrap();
    let second_id = insert_message(
        &mut store,
        &conversation_id,
        Role::Assistant,
        &[MessageInputPart::Text("second".to_string())],
    )
    .unwrap();

    activate_message(&mut store, &conversation_id, &first_id).unwrap();
    update_message(&mut store, &conversation_id, &second_id, "second updated").unwrap();
    activate_message(&mut store, &conversation_id, &second_id).unwrap();

    let path = store.load_active_path(&conversation_id).unwrap();
    assert_eq!(path.len(), 2);
    assert_eq!(path[1].content, "second updated");
    assert_eq!(
        store.active_message_id(&conversation_id).unwrap().as_ref(),
        Some(&second_id)
    );
}

#[test]
fn deny_tool_persists_tool_result() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = create_conversation(&store, &ModelName::new("openai/test")).unwrap();
    let user_id = store
        .insert_message(&conversation_id, None, Role::User, "run command", None)
        .unwrap();
    store
        .insert_message(
            &conversation_id,
            Some(&user_id),
            Role::Assistant,
            "",
            Some(&MessageMetadata {
                tool_calls: vec![ToolCall::function(
                    "call_123",
                    "run_shell",
                    r#"{"command":"printf no"}"#,
                )],
                ..Default::default()
            }),
        )
        .unwrap();

    let result = deny_tool(&mut store, &conversation_id, &ToolCallId::new("call_123")).unwrap();
    let messages = store.load_active_path(&conversation_id).unwrap();

    assert!(!result.success);
    assert_eq!(messages.last().unwrap().role, Role::Tool);
    assert_eq!(
        messages
            .last()
            .unwrap()
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.tool_call_id.as_ref())
            .map(ToolCallId::as_str),
        Some("call_123")
    );
}

fn temp_image_path(extension: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let counter = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);

    std::env::temp_dir().join(format!(
        "windie-operation-{}-{nanos}-{counter}.{extension}",
        std::process::id()
    ))
}

fn registry_with_cached_test_tool() -> ToolProviderRegistry {
    ToolProviderRegistry::with_test_mcp_provider(
        "desktop-commander",
        "desktop_commander",
        "Desktop Commander",
        McpCommand {
            program: "windie-test-unused-mcp-provider",
            args: &[],
            env: &[],
        },
        vec![desktop_commander_read_file_definition()],
    )
}

fn desktop_commander_read_file_definition() -> ToolDefinition {
    ToolDefinition {
        schema_name: ToolSchemaName::new("desktop_commander__read_file"),
        display_name: "Desktop Commander read_file".to_string(),
        description: "Read a file through Desktop Commander.".to_string(),
        parameters: serde_json::json!({"type":"object"}),
        provider: ToolProviderRef::new(
            ToolProviderId::new("desktop-commander"),
            ProviderToolName::new("read_file"),
            ToolProviderKind::Mcp,
        ),
        permissions: vec![ToolPermission::ExternalProcess],
        annotations: ToolAnnotations::default(),
    }
}

#[test]
fn destructive_mutations_reject_while_conversation_is_running() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = create_conversation(&store, &ModelName::new("openai/test")).unwrap();
    let message_id = store
        .insert_message(&conversation_id, None, Role::User, "hello", None)
        .unwrap();
    store.create_runtime_run(&conversation_id).unwrap();

    assert!(
        update_message(&mut store, &conversation_id, &message_id, "changed")
            .unwrap_err()
            .to_string()
            .contains("running action")
    );
    assert!(
        remove_message(&mut store, &conversation_id, &message_id)
            .unwrap_err()
            .to_string()
            .contains("running action")
    );
    assert!(
        truncate_conversation(&mut store, &conversation_id, &message_id)
            .unwrap_err()
            .to_string()
            .contains("running action")
    );
    assert!(
        remove_conversation(&mut store, &conversation_id)
            .unwrap_err()
            .to_string()
            .contains("running action")
    );
}

#[test]
fn non_destructive_path_operations_remain_available_while_running() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = create_conversation(&store, &ModelName::new("openai/test")).unwrap();
    let root_id = store
        .insert_message(&conversation_id, None, Role::User, "root", None)
        .unwrap();
    store.create_runtime_run(&conversation_id).unwrap();

    let branch_id = insert_message(
        &mut store,
        &conversation_id,
        Role::User,
        &[MessageInputPart::Text("branch".to_string())],
    )
    .unwrap();
    activate_message(&mut store, &conversation_id, &root_id).unwrap();
    let forked_id = fork_conversation(&mut store, &conversation_id, &root_id).unwrap();

    assert_eq!(
        store.active_message_id(&conversation_id).unwrap(),
        Some(root_id)
    );
    assert!(
        store
            .load_path_to_message(&conversation_id, &branch_id)
            .is_ok()
    );
    assert!(store.load_active_path(&forked_id).is_ok());
}
