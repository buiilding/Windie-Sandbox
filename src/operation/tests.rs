//! Operation workflow tests.

use super::*;
use crate::mcp::McpCommand;
use crate::tool::{ToolAnnotations, ToolPermission, ToolProviderKind, ToolProviderRef};
use crate::tool_provider::ProviderInstallState;
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
        None,
        Role::User,
        &[MessageInputPart::Text("hello".to_string())],
    )
    .unwrap();

    let messages = store
        .load_path_to_message(&conversation_id, &message_id)
        .unwrap();
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
fn openai_reasoning_effort_requests_visible_summary() {
    let reasoning = reasoning_request_for_model(
        &ModelName::new("openai/gpt-5.5"),
        Some(ReasoningRequest {
            effort: Some("high".to_string()),
            summary: None,
        }),
    )
    .unwrap();

    assert_eq!(reasoning.effort.as_deref(), Some("high"));
    assert_eq!(reasoning.summary.as_deref(), Some("auto"));
}

#[test]
fn openai_reasoning_preserves_explicit_summary() {
    let reasoning = reasoning_request_for_model(
        &ModelName::new("openai/gpt-5.5"),
        Some(ReasoningRequest {
            effort: Some("high".to_string()),
            summary: Some("detailed".to_string()),
        }),
    )
    .unwrap();

    assert_eq!(reasoning.effort.as_deref(), Some("high"));
    assert_eq!(reasoning.summary.as_deref(), Some("detailed"));
}

#[test]
fn anthropic_reasoning_does_not_request_openai_summary() {
    let reasoning = reasoning_request_for_model(
        &ModelName::new("anthropic/claude-fable-5"),
        Some(ReasoningRequest {
            effort: Some("high".to_string()),
            summary: None,
        }),
    )
    .unwrap();

    assert_eq!(reasoning.effort.as_deref(), Some("high"));
    assert_eq!(reasoning.summary, None);
}

#[test]
fn empty_reasoning_request_stays_absent() {
    let reasoning = reasoning_request_for_model(
        &ModelName::new("openai/gpt-5.5"),
        Some(ReasoningRequest {
            effort: None,
            summary: None,
        }),
    );

    assert_eq!(reasoning, None);
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
        None,
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
        None,
        Role::User,
        &[
            MessageInputPart::Text("first".to_string()),
            MessageInputPart::ImagePath(image_path.clone()),
        ],
    )
    .unwrap();

    let messages = store.load_messages(&conversation_id).unwrap();
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
        None,
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

    let messages = store.load_messages(&conversation_id).unwrap();
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

    let context = conversation_input_token_context(&store, &conversation_id, None)
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
        None,
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

    let report = inspect_conversation(&store, &conversation_id, Some(&user_id), None).unwrap();
    let value = serde_json::to_value(report).unwrap();

    assert_eq!(value["conversation_id"], conversation_id.as_str());
    assert_eq!(value["head_message_id"], user_id.as_str());
    assert_eq!(value["model"], "anthropic/test");
    assert_eq!(value["reasoning"]["effort"], "high");
    assert_eq!(value["system_prompt"], "You are concise.");
    assert_eq!(value["tool_schemas"][0]["name"], "run_shell");
    // Tree-wide: system prompt is stored in conversations table, not as a message in the tree.
    assert_eq!(value["messages"][0]["id"], user_id.as_str());
    assert_eq!(value["path"][0]["id"], user_id.as_str());
    // model_context = [system_prompt, compaction] when compaction is through the head
    assert_eq!(value["model_context"][0]["role"], "system");
    assert_eq!(value["model_context"][0]["content"], "You are concise.");
    assert_eq!(
        value["model_context"][1]["content"],
        "Previous conversation summary:\nhello happened"
    );
    assert_eq!(value["latest_compaction"]["content"], "hello happened");
}

#[test]
fn attaches_available_provider_tool() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = create_conversation(&store, &ModelName::new("openai/test")).unwrap();
    let registry = registry_with_cached_test_tool();
    enable_test_provider(&store);
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
    enable_test_provider(&store);
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
fn provider_manager_persists_lifecycle_transitions_and_health() {
    let store = Store::open_memory().unwrap();
    let registry = registry_with_cached_test_tool();
    let provider_id = ToolProviderId::new("desktop-commander");

    let installed = install_provider(&store, &registry, &provider_id).unwrap();
    assert_eq!(
        installed.installation.unwrap().state,
        ProviderInstallState::Installed
    );

    let enabled = enable_provider(&store, &registry, &provider_id).unwrap();
    assert_eq!(
        enabled.installation.unwrap().state,
        ProviderInstallState::Enabled
    );

    let healthy = health_check_provider(&store, &registry, &provider_id).unwrap();
    assert_eq!(
        healthy.installation.unwrap().state,
        ProviderInstallState::Enabled
    );

    let disabled = disable_provider(&store, &registry, &provider_id).unwrap();
    assert_eq!(
        disabled.installation.unwrap().state,
        ProviderInstallState::Disabled
    );

    uninstall_provider(&store, &registry, &provider_id).unwrap();
    assert!(
        store
            .load_installed_provider(&provider_id)
            .unwrap()
            .is_none()
    );
}

#[test]
fn desktop_commander_setup_verifies_and_enables_provider() {
    let store = Store::open_memory().unwrap();
    let registry = registry_with_cached_test_tool();
    let provider_id = ToolProviderId::new("desktop-commander");

    let setup = setup_provider(&store, &registry, &provider_id).unwrap();
    let installation = setup.installation.unwrap();

    assert_eq!(installation.state, ProviderInstallState::Enabled);
    assert!(installation.error.is_none());
    assert!(installation.last_health_check_at.is_some());

    let tools = available_tools_with_registry(&store, &registry).unwrap();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].provider.provider_id, provider_id);
    assert_eq!(
        enabled_provider_statuses(&store, &registry).unwrap().len(),
        1
    );
}

#[test]
fn uninstalled_provider_is_not_exposed_to_tool_catalog_or_attachment() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = create_conversation(&store, &ModelName::new("openai/test")).unwrap();
    let registry = registry_with_cached_test_tool();
    let provider_id = ToolProviderId::new("desktop-commander");

    assert!(
        available_tools_with_registry(&store, &registry)
            .unwrap()
            .is_empty()
    );
    assert!(
        enabled_provider_statuses(&store, &registry)
            .unwrap()
            .is_empty()
    );

    let error = attach_tool_with_registry(
        &mut store,
        &conversation_id,
        &provider_id,
        &ProviderToolName::new("read_file"),
        &registry,
    )
    .unwrap_err();
    assert!(error.to_string().contains("provider is not installed"));
}

#[test]
fn one_click_setup_rejects_unimplemented_provider() {
    let store = Store::open_memory().unwrap();
    let registry = ToolProviderRegistry::new();

    let error = setup_provider(&store, &registry, &ToolProviderId::new("blender-mcp")).unwrap_err();

    assert!(
        error
            .to_string()
            .contains("one-click setup is not implemented")
    );
}

#[test]
fn shared_operations_match_direct_store_state() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = create_conversation(&store, &ModelName::new("openai/test")).unwrap();
    let first_id = insert_message(
        &mut store,
        &conversation_id,
        None,
        Role::User,
        &[MessageInputPart::Text("first".to_string())],
    )
    .unwrap();
    let second_id = insert_message(
        &mut store,
        &conversation_id,
        Some(&first_id),
        Role::Assistant,
        &[MessageInputPart::Text("second".to_string())],
    )
    .unwrap();

    update_message(&mut store, &conversation_id, &second_id, "second updated").unwrap();

    let path = store
        .load_path_to_message(&conversation_id, &second_id)
        .unwrap();
    assert_eq!(path.len(), 2);
    assert_eq!(path[1].content, "second updated");
}

#[test]
fn create_session_branch_captures_requested_head() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = create_conversation(&store, &ModelName::new("openai/test")).unwrap();
    let user_id = insert_message(
        &mut store,
        &conversation_id,
        None,
        Role::User,
        &[MessageInputPart::Text("hello".to_string())],
    )
    .unwrap();

    let session = store
        .create_session(
            &SessionId::fresh(),
            &conversation_id,
            Some(&user_id),
            "openai/test",
            None,
        )
        .unwrap();

    assert_eq!(session.conversation_id, conversation_id);
    assert_eq!(session.start_head_message_id.as_ref(), Some(&user_id));
    assert_eq!(session.current_head_message_id.as_ref(), Some(&user_id));
    assert_eq!(session.model, "openai/test");
    assert_eq!(session.status, SessionStatus::Ready);
}

#[test]
fn resume_session_from_wakeup_resolves_waiting_approval() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = create_conversation(&store, &ModelName::new("openai/test")).unwrap();
    let head_message_id = insert_message(
        &mut store,
        &conversation_id,
        None,
        Role::Assistant,
        &[MessageInputPart::Text("tool call pending".to_string())],
    )
    .unwrap();
    let session_id = SessionId::fresh();
    store
        .create_session(
            &session_id,
            &conversation_id,
            Some(&head_message_id),
            "openai/test",
            None,
        )
        .unwrap();
    store
        .update_session_status(&session_id, SessionStatus::WaitingForApproval, None)
        .unwrap();

    let resume = resume_session_from_wakeup(
        &store,
        crate::wakeup::Wakeup::ApproveTool(crate::wakeup::ToolDecisionWakeup {
            session_id: session_id.clone(),
            tool_call_id: ToolCallId::new("call_1"),
        }),
    )
    .unwrap()
    .unwrap();

    assert_eq!(resume.session.id, session_id);
    assert_eq!(
        resume.action,
        SessionResumeAction::ApproveTool(ToolCallId::new("call_1"))
    );
}

#[test]
fn resume_session_from_wakeup_ignores_non_waiting_approval() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = create_conversation(&store, &ModelName::new("openai/test")).unwrap();
    let head_message_id = insert_message(
        &mut store,
        &conversation_id,
        None,
        Role::Assistant,
        &[MessageInputPart::Text("complete".to_string())],
    )
    .unwrap();
    let session_id = SessionId::fresh();
    store
        .create_session(
            &session_id,
            &conversation_id,
            Some(&head_message_id),
            "openai/test",
            None,
        )
        .unwrap();
    store
        .update_session_status(&session_id, SessionStatus::Completed, None)
        .unwrap();

    let resume = resume_session_from_wakeup(
        &store,
        crate::wakeup::Wakeup::DenyTool(crate::wakeup::ToolDecisionWakeup {
            session_id,
            tool_call_id: ToolCallId::new("call_1"),
        }),
    )
    .unwrap();

    assert!(resume.is_none());
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

fn enable_test_provider(store: &Store) {
    let provider_id = ToolProviderId::new("desktop-commander");
    store.install_provider(&provider_id).unwrap();
    store
        .set_provider_state(&provider_id, ProviderInstallState::Enabled, None)
        .unwrap();
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
