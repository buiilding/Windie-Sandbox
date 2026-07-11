//! Tools persistence tests.

use super::*;

#[test]
fn new_conversation_defaults_to_manual_tool_approval() {
    let store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();

    let mode = store.tool_approval_mode(&conversation_id).unwrap();

    assert_eq!(mode, ToolApprovalMode::Manual);
}

#[test]
fn set_tool_approval_mode_persists() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();

    store
        .set_tool_approval_mode(&conversation_id, ToolApprovalMode::AutoApproveAttached)
        .unwrap();

    let mode = store.tool_approval_mode(&conversation_id).unwrap();

    assert_eq!(mode, ToolApprovalMode::AutoApproveAttached);
}

#[test]
fn saves_updates_and_removes_tool_schemas() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store
        .get_or_create_default_conversation("openai/test")
        .unwrap();
    let tool_schema = ToolSchema {
        name: ToolSchemaName::new("run_shell"),
        description: "Run a shell command".to_string(),
        parameters: serde_json::json!({"type":"object"}),
    };

    store
        .insert_tool_schema(&conversation_id, &tool_schema)
        .unwrap();

    let loaded = store.load_tool_schemas(&conversation_id).unwrap();
    assert_eq!(loaded, vec![tool_schema.clone()]);

    let updated = ToolSchema {
        name: ToolSchemaName::new("shell"),
        description: "Run command".to_string(),
        parameters: serde_json::json!({"type":"object","properties":{}}),
    };
    store
        .update_tool_schema(
            &conversation_id,
            &ToolSchemaName::new("run_shell"),
            &updated,
        )
        .unwrap();

    let loaded = store.load_tool_schemas(&conversation_id).unwrap();
    assert_eq!(loaded, vec![updated]);

    store
        .remove_tool_schema(&conversation_id, &ToolSchemaName::new("shell"))
        .unwrap();

    assert!(
        store
            .load_tool_schemas(&conversation_id)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn batch_attached_tool_insert_is_atomic() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store
        .get_or_create_default_conversation("openai/test")
        .unwrap();
    let first = AttachedTool::manual(ToolSchema {
        name: ToolSchemaName::new("first_tool"),
        description: "First tool".to_string(),
        parameters: serde_json::json!({"type":"object"}),
    });
    let duplicate = AttachedTool::manual(ToolSchema {
        name: ToolSchemaName::new("first_tool"),
        description: "Duplicate tool".to_string(),
        parameters: serde_json::json!({"type":"object"}),
    });

    let error = store
        .insert_attached_tools(&conversation_id, &[first, duplicate])
        .unwrap_err();

    assert!(error.to_string().contains("failed to attach tools"));
    assert!(
        store
            .load_tool_schemas(&conversation_id)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn rejects_non_object_tool_schema_parameters() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store
        .get_or_create_default_conversation("openai/test")
        .unwrap();
    let tool_schema = ToolSchema {
        name: ToolSchemaName::new("bad"),
        description: "Bad schema".to_string(),
        parameters: serde_json::json!("not an object"),
    };

    let error = store
        .insert_tool_schema(&conversation_id, &tool_schema)
        .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("tool schema parameters must be a JSON object")
    );
}

#[test]
fn rejects_invalid_tool_schema_name() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store
        .get_or_create_default_conversation("openai/test")
        .unwrap();
    let tool_schema = ToolSchema {
        name: ToolSchemaName::new("run shell"),
        description: "Run a shell command".to_string(),
        parameters: serde_json::json!({"type":"object"}),
    };

    let error = store
        .insert_tool_schema(&conversation_id, &tool_schema)
        .unwrap_err();

    assert!(error.to_string().contains("tool schema name must be"));
}

#[test]
fn rejects_empty_tool_schema_description() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store
        .get_or_create_default_conversation("openai/test")
        .unwrap();
    let tool_schema = ToolSchema {
        name: ToolSchemaName::new("run_shell"),
        description: " ".to_string(),
        parameters: serde_json::json!({"type":"object"}),
    };

    let error = store
        .insert_tool_schema(&conversation_id, &tool_schema)
        .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("tool schema description must not be empty")
    );
}

#[test]
fn remove_assistant_tool_call_deletes_tool_pair_and_preserves_later_descendant() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();
    let first_id = store
        .insert_message(&conversation_id, None, Role::User, "one", None)
        .unwrap();
    let assistant_metadata = MessageMetadata {
        tool_calls: vec![ToolCall::function(
            "call_123",
            "run_shell",
            r#"{"command":"ls"}"#,
        )],
        ..Default::default()
    };
    let assistant_id = store
        .insert_message(
            &conversation_id,
            Some(&first_id),
            Role::Assistant,
            "",
            Some(&assistant_metadata),
        )
        .unwrap();
    let tool_id = insert_tool_result(
        &mut store,
        &conversation_id,
        &assistant_id,
        "call_123",
        "{}",
    );
    let final_id = store
        .insert_message(
            &conversation_id,
            Some(&tool_id),
            Role::Assistant,
            "done",
            None,
        )
        .unwrap();

    store
        .remove_message(&conversation_id, &assistant_id)
        .unwrap();

    let messages = store.load_messages(&conversation_id).unwrap();
    let active_message_id = store.active_message_id(&conversation_id).unwrap();

    assert_eq!(
        message_ids(&messages),
        vec![first_id.to_string(), final_id.to_string()]
    );
    assert!(
        messages
            .iter()
            .all(|message| message.id.as_ref() != Some(&assistant_id))
    );
    assert!(
        messages
            .iter()
            .all(|message| message.id.as_ref() != Some(&tool_id))
    );
    assert_eq!(message_parent(&messages, &final_id), Some(&first_id));
    assert_eq!(active_message_id.as_deref(), Some(final_id.as_str()));
}

#[test]
fn remove_tool_output_deletes_tool_pair_and_preserves_later_descendant() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();
    let first_id = store
        .insert_message(&conversation_id, None, Role::User, "one", None)
        .unwrap();
    let assistant_metadata = MessageMetadata {
        tool_calls: vec![ToolCall::function(
            "call_123",
            "run_shell",
            r#"{"command":"ls"}"#,
        )],
        ..Default::default()
    };
    let assistant_id = store
        .insert_message(
            &conversation_id,
            Some(&first_id),
            Role::Assistant,
            "",
            Some(&assistant_metadata),
        )
        .unwrap();
    let tool_id = insert_tool_result(
        &mut store,
        &conversation_id,
        &assistant_id,
        "call_123",
        "{}",
    );
    let final_id = store
        .insert_message(
            &conversation_id,
            Some(&tool_id),
            Role::Assistant,
            "done",
            None,
        )
        .unwrap();

    store.remove_message(&conversation_id, &tool_id).unwrap();

    let messages = store.load_messages(&conversation_id).unwrap();

    assert_eq!(
        message_ids(&messages),
        vec![first_id.to_string(), final_id.to_string()]
    );
    assert!(
        messages
            .iter()
            .all(|message| message.id.as_ref() != Some(&assistant_id))
    );
    assert!(
        messages
            .iter()
            .all(|message| message.id.as_ref() != Some(&tool_id))
    );
    assert_eq!(message_parent(&messages, &final_id), Some(&first_id));
}

#[test]
fn remove_pending_assistant_tool_call_without_result_uses_normal_splice() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();
    let first_id = store
        .insert_message(&conversation_id, None, Role::User, "one", None)
        .unwrap();
    let assistant_metadata = MessageMetadata {
        tool_calls: vec![ToolCall::function(
            "call_123",
            "run_shell",
            r#"{"command":"ls"}"#,
        )],
        ..Default::default()
    };
    let assistant_id = store
        .insert_message(
            &conversation_id,
            Some(&first_id),
            Role::Assistant,
            "",
            Some(&assistant_metadata),
        )
        .unwrap();
    let final_id = store
        .insert_message(
            &conversation_id,
            Some(&assistant_id),
            Role::Assistant,
            "next",
            None,
        )
        .unwrap();

    store
        .remove_message(&conversation_id, &assistant_id)
        .unwrap();

    let messages = store.load_messages(&conversation_id).unwrap();

    assert_eq!(
        message_ids(&messages),
        vec![first_id.to_string(), final_id.to_string()]
    );
    assert_eq!(message_parent(&messages, &final_id), Some(&first_id));
}

#[test]
fn remove_multi_tool_call_assistant_deletes_tool_group() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();
    let first_id = store
        .insert_message(&conversation_id, None, Role::User, "one", None)
        .unwrap();
    let metadata = MessageMetadata {
        tool_calls: vec![
            ToolCall::function("call_1", "run_shell", r#"{"command":"ls"}"#),
            ToolCall::function("call_2", "run_shell", r#"{"command":"pwd"}"#),
        ],
        ..Default::default()
    };
    let assistant_id = store
        .insert_message(
            &conversation_id,
            Some(&first_id),
            Role::Assistant,
            "",
            Some(&metadata),
        )
        .unwrap();
    let first_tool_id =
        insert_tool_result(&mut store, &conversation_id, &assistant_id, "call_1", "{}");
    let second_tool_id =
        insert_tool_result(&mut store, &conversation_id, &first_tool_id, "call_2", "{}");
    let final_id = store
        .insert_message(
            &conversation_id,
            Some(&second_tool_id),
            Role::Assistant,
            "done",
            None,
        )
        .unwrap();

    store
        .remove_message(&conversation_id, &assistant_id)
        .unwrap();

    let messages = store.load_messages(&conversation_id).unwrap();

    assert_eq!(
        message_ids(&messages),
        vec![first_id.to_string(), final_id.to_string()]
    );
    assert!(
        messages
            .iter()
            .all(|message| message.id.as_ref() != Some(&assistant_id))
    );
    assert!(
        messages
            .iter()
            .all(|message| message.id.as_ref() != Some(&first_tool_id))
    );
    assert!(
        messages
            .iter()
            .all(|message| message.id.as_ref() != Some(&second_tool_id))
    );
    assert_eq!(message_parent(&messages, &final_id), Some(&first_id));
}

#[test]
fn remove_multi_tool_output_deletes_whole_tool_group() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();
    let first_id = store
        .insert_message(&conversation_id, None, Role::User, "one", None)
        .unwrap();
    let metadata = MessageMetadata {
        tool_calls: vec![
            ToolCall::function("call_1", "run_shell", r#"{"command":"ls"}"#),
            ToolCall::function("call_2", "run_shell", r#"{"command":"pwd"}"#),
        ],
        ..Default::default()
    };
    let assistant_id = store
        .insert_message(
            &conversation_id,
            Some(&first_id),
            Role::Assistant,
            "",
            Some(&metadata),
        )
        .unwrap();
    let first_tool_id =
        insert_tool_result(&mut store, &conversation_id, &assistant_id, "call_1", "{}");
    let second_tool_id =
        insert_tool_result(&mut store, &conversation_id, &first_tool_id, "call_2", "{}");
    let final_id = store
        .insert_message(
            &conversation_id,
            Some(&second_tool_id),
            Role::Assistant,
            "done",
            None,
        )
        .unwrap();

    store
        .remove_message(&conversation_id, &first_tool_id)
        .unwrap();

    let messages = store.load_messages(&conversation_id).unwrap();

    assert_eq!(
        message_ids(&messages),
        vec![first_id.to_string(), final_id.to_string()]
    );
    assert!(
        messages
            .iter()
            .all(|message| message.id.as_ref() != Some(&assistant_id))
    );
    assert!(
        messages
            .iter()
            .all(|message| message.id.as_ref() != Some(&first_tool_id))
    );
    assert!(
        messages
            .iter()
            .all(|message| message.id.as_ref() != Some(&second_tool_id))
    );
    assert_eq!(message_parent(&messages, &final_id), Some(&first_id));
}

#[test]
fn remove_second_multi_tool_output_deletes_whole_tool_group() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();
    let first_id = store
        .insert_message(&conversation_id, None, Role::User, "one", None)
        .unwrap();
    let metadata = MessageMetadata {
        tool_calls: vec![
            ToolCall::function("call_1", "run_shell", r#"{"command":"ls"}"#),
            ToolCall::function("call_2", "run_shell", r#"{"command":"pwd"}"#),
        ],
        ..Default::default()
    };
    let assistant_id = store
        .insert_message(
            &conversation_id,
            Some(&first_id),
            Role::Assistant,
            "",
            Some(&metadata),
        )
        .unwrap();
    let first_tool_id =
        insert_tool_result(&mut store, &conversation_id, &assistant_id, "call_1", "{}");
    let second_tool_id =
        insert_tool_result(&mut store, &conversation_id, &first_tool_id, "call_2", "{}");
    let final_id = store
        .insert_message(
            &conversation_id,
            Some(&second_tool_id),
            Role::Assistant,
            "done",
            None,
        )
        .unwrap();

    store
        .remove_message(&conversation_id, &second_tool_id)
        .unwrap();

    let messages = store.load_messages(&conversation_id).unwrap();

    assert_eq!(
        message_ids(&messages),
        vec![first_id.to_string(), final_id.to_string()]
    );
    assert!(
        messages
            .iter()
            .all(|message| message.id.as_ref() != Some(&assistant_id))
    );
    assert!(
        messages
            .iter()
            .all(|message| message.id.as_ref() != Some(&first_tool_id))
    );
    assert!(
        messages
            .iter()
            .all(|message| message.id.as_ref() != Some(&second_tool_id))
    );
    assert_eq!(message_parent(&messages, &final_id), Some(&first_id));
}

#[test]
fn generic_insert_rejects_role_tool_message() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();

    let error = store
        .insert_message(
            &conversation_id,
            None,
            Role::Tool,
            "{}",
            Some(&MessageMetadata {
                tool_call_id: Some(ToolCallId::new("call_123")),
                ..Default::default()
            }),
        )
        .unwrap_err();

    assert_eq!(
        error.to_string(),
        "role: tool messages must be created through insert_tool_result_message"
    );
}

#[test]
fn tool_result_insert_without_matching_assistant_parent_rejects() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();
    let first_id = store
        .insert_message(&conversation_id, None, Role::User, "one", None)
        .unwrap();

    let error = store
        .insert_tool_result_message(
            &conversation_id,
            &first_id,
            &ToolCallId::new("call_123"),
            "{}",
        )
        .unwrap_err();

    assert_eq!(
        error.to_string(),
        "role: tool result parent must be an assistant tool-call message or tool result chain"
    );
    assert_eq!(store.load_messages(&conversation_id).unwrap().len(), 1);
}

#[test]
fn remove_assistant_tool_call_ignores_same_tool_call_id_on_other_branch() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();
    let first_id = store
        .insert_message(&conversation_id, None, Role::User, "one", None)
        .unwrap();
    let assistant_metadata = MessageMetadata {
        tool_calls: vec![ToolCall::function(
            "call_123",
            "run_shell",
            r#"{"command":"ls"}"#,
        )],
        ..Default::default()
    };
    let assistant_id = store
        .insert_message(
            &conversation_id,
            Some(&first_id),
            Role::Assistant,
            "",
            Some(&assistant_metadata),
        )
        .unwrap();
    let tool_id = insert_tool_result(
        &mut store,
        &conversation_id,
        &assistant_id,
        "call_123",
        "{}",
    );
    let other_root_id = store
        .insert_message(&conversation_id, None, Role::User, "other", None)
        .unwrap();
    let other_assistant_id = store
        .insert_message(
            &conversation_id,
            Some(&other_root_id),
            Role::Assistant,
            "",
            Some(&assistant_metadata),
        )
        .unwrap();
    let other_tool_id = insert_tool_result(
        &mut store,
        &conversation_id,
        &other_assistant_id,
        "call_123",
        "{}",
    );
    let final_id = store
        .insert_message(
            &conversation_id,
            Some(&tool_id),
            Role::Assistant,
            "done",
            None,
        )
        .unwrap();

    store
        .remove_message(&conversation_id, &assistant_id)
        .unwrap();

    let messages = store.load_messages(&conversation_id).unwrap();

    assert!(
        messages
            .iter()
            .all(|message| message.id.as_ref() != Some(&assistant_id))
    );
    assert!(
        messages
            .iter()
            .all(|message| message.id.as_ref() != Some(&tool_id))
    );
    assert!(
        messages
            .iter()
            .any(|message| message.id.as_ref() == Some(&other_assistant_id))
    );
    assert!(
        messages
            .iter()
            .any(|message| message.id.as_ref() == Some(&other_tool_id))
    );
    assert_eq!(message_parent(&messages, &final_id), Some(&first_id));
    assert_eq!(
        message_parent(&messages, &other_tool_id),
        Some(&other_assistant_id)
    );
}
