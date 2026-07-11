//! Shared benchmark fixture construction and temporary storage.

use super::*;

pub(super) fn with_runtime_store<T>(
    scenario: &str,
    run: impl FnOnce(&mut Store) -> Result<T>,
) -> Result<T> {
    let path = runtime_database_path(scenario);
    let result = {
        let mut store = Store::open_at(&path)?;
        run(&mut store)
    };
    remove_runtime_database_files(&path);

    result
}

/// Builds a unique SQLite path for one runtime benchmark scenario.
pub(super) fn runtime_database_path(scenario: &str) -> PathBuf {
    env::temp_dir().join(format!(
        "windie-runtime-bench-{scenario}-{}-{}.db",
        process::id(),
        Uuid::new_v4()
    ))
}

/// Removes SQLite database files created for one benchmark scenario.
pub(super) fn remove_runtime_database_files(path: &Path) {
    let _ = fs::remove_file(path);
    let _ = fs::remove_file(path.with_extension("db-wal"));
    let _ = fs::remove_file(path.with_extension("db-shm"));
}

/// Inserts a simple user message and returns its generated message ID.
pub(super) fn insert_user_message(
    store: &mut Store,
    conversation_id: &ConversationId,
    parent_message_id: Option<&MessageId>,
    content: &str,
) -> Result<MessageId> {
    store.insert_message(
        conversation_id,
        parent_message_id,
        Role::User,
        content,
        None,
    )
}

/// Attaches a test MCP provider tool to a benchmark conversation.
///
/// Runtime approval benchmarks need the same provider-backed attachment that
/// real conversations use; otherwise policy would measure the detached-tool
/// denial path instead of the approval path.
pub(super) fn attach_test_mcp_tool(
    store: &mut Store,
    conversation_id: &ConversationId,
) -> Result<()> {
    store.insert_attached_tool(conversation_id, &test_tool_definition().attached_tool())
}

/// Builds the provider-backed test tool used by runtime benchmarks.
pub(super) fn test_tool_definition() -> ToolDefinition {
    ToolDefinition {
        schema_name: crate::conversation::ToolSchemaName::new(TEST_TOOL_SCHEMA_NAME),
        display_name: "Desktop Commander read_file".to_string(),
        description: "Read a file through Desktop Commander.".to_string(),
        parameters: serde_json::json!({"type":"object"}),
        provider: ToolProviderRef::new(
            ToolProviderId::new(TEST_PROVIDER_ID),
            ProviderToolName::new(TEST_PROVIDER_TOOL_NAME),
            ToolProviderKind::Mcp,
        ),
        permissions: vec![ToolPermission::ExternalProcess],
        annotations: ToolAnnotations::default(),
    }
}

/// Creates a linear active path with alternating user and assistant messages.
pub(super) fn create_message_chain(
    store: &mut Store,
    conversation_id: &ConversationId,
    message_count: usize,
) -> Result<Option<MessageId>> {
    let mut parent_id = None;

    for index in 0..message_count {
        let role = if index % 2 == 0 {
            Role::User
        } else {
            Role::Assistant
        };
        let id = store.insert_message(
            conversation_id,
            parent_id.as_ref(),
            role,
            &format!("message {index}"),
            None,
        )?;
        parent_id = Some(id);
    }

    Ok(parent_id)
}

/// Creates one assistant tool-call message with all requested results stored.
pub(super) fn create_completed_tool_chain(
    store: &mut Store,
    conversation_id: &ConversationId,
    result_count: usize,
) -> Result<MessageId> {
    let user_id = insert_user_message(store, conversation_id, None, "use tools")?;
    let tool_calls = (0..result_count)
        .map(|index| {
            tool_call(
                index as u16,
                &format!("call_{index}"),
                "desktop_commander__read_file",
            )
        })
        .collect::<Vec<_>>();
    let assistant_id = store.insert_message(
        conversation_id,
        Some(&user_id),
        Role::Assistant,
        "",
        Some(&tool_call_metadata(tool_calls.clone())),
    )?;
    let mut parent_id = assistant_id.clone();
    for tool_call in &tool_calls {
        parent_id = insert_tool_result(store, conversation_id, &parent_id, &tool_call.id)?;
    }

    Ok(assistant_id)
}

/// Inserts one model-facing tool result under the requested chain parent.
pub(super) fn insert_tool_result(
    store: &mut Store,
    conversation_id: &ConversationId,
    parent_message_id: &MessageId,
    tool_call_id: &ToolCallId,
) -> Result<MessageId> {
    store.insert_tool_result_message(
        conversation_id,
        parent_message_id,
        tool_call_id,
        r#"{"stdout":"ok","stderr":"","exit_code":0}"#,
    )
}

/// Builds assistant metadata for requested tool calls.
pub(super) fn tool_call_metadata(tool_calls: Vec<ToolCall>) -> MessageMetadata {
    MessageMetadata {
        tool_calls,
        ..Default::default()
    }
}

/// Builds a deterministic function tool call for runtime benchmark fixtures.
pub(super) fn tool_call(index: u16, id: &str, name: &str) -> ToolCall {
    let mut tool_call = ToolCall::function(id, name, r#"{"command":"printf ok"}"#);
    tool_call.index = index;
    tool_call
}

/// Returns tiny deterministic bytes for image-part benchmark fixtures.
pub(super) fn tiny_png_bytes() -> &'static [u8] {
    &[
        0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n', 0, 0, 0, 0, b'I', b'E', b'N', b'D',
    ]
}
