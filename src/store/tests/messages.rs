//! Messages persistence tests.

use super::*;

#[test]
fn loads_empty_messages_for_existing_conversation() {
    let store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();

    let messages = store.load_messages(&conversation_id).unwrap();

    assert!(messages.is_empty());
}

#[test]
fn loads_empty_active_path_for_empty_conversation() {
    let store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();

    let active_message_id = store.active_message_id(&conversation_id).unwrap();
    let path = store.load_active_path(&conversation_id).unwrap();

    assert!(active_message_id.is_none());
    assert!(path.is_empty());
}

#[test]
fn rejects_loading_messages_from_missing_conversation() {
    let store = Store::open_memory().unwrap();

    let error = store
        .load_messages(&ConversationId::new("missing"))
        .unwrap_err();

    assert!(error.to_string().contains("conversation does not exist"));
}

#[test]
fn saves_and_loads_messages() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store
        .get_or_create_default_conversation("openai/test")
        .unwrap();

    let user_id = store
        .insert_message(&conversation_id, None, Role::User, "hello", None)
        .unwrap();
    let assistant_id = store
        .insert_message(
            &conversation_id,
            Some(&user_id),
            Role::Assistant,
            "hello back",
            None,
        )
        .unwrap();

    let messages = store.load_messages(&conversation_id).unwrap();

    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].id.as_deref(), Some(user_id.as_str()));
    assert_eq!(messages[0].content, "hello");
    assert_eq!(messages[1].id.as_deref(), Some(assistant_id.as_str()));
    assert_eq!(
        messages[1].parent_message_id.as_deref(),
        Some(user_id.as_str())
    );
    assert_eq!(messages[1].content, "hello back");
}

#[test]
fn insert_sets_active_message() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();

    let message_id = store
        .insert_message(&conversation_id, None, Role::User, "hello", None)
        .unwrap();

    let active_message_id = store.active_message_id(&conversation_id).unwrap();

    assert_eq!(active_message_id.as_deref(), Some(message_id.as_str()));
}

#[test]
fn loads_active_path() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();
    let root_id = store
        .insert_message(&conversation_id, None, Role::User, "root", None)
        .unwrap();
    let first_branch_id = store
        .insert_message(
            &conversation_id,
            Some(&root_id),
            Role::Assistant,
            "first",
            None,
        )
        .unwrap();
    let second_branch_id = store
        .insert_message(
            &conversation_id,
            Some(&root_id),
            Role::Assistant,
            "second",
            None,
        )
        .unwrap();

    store
        .set_active_message(&conversation_id, &first_branch_id)
        .unwrap();

    let path = store.load_active_path(&conversation_id).unwrap();

    assert_eq!(path.len(), 2);
    assert_eq!(path[0].id.as_deref(), Some(root_id.as_str()));
    assert_eq!(path[1].id.as_deref(), Some(first_branch_id.as_str()));
    assert_ne!(path[1].id.as_deref(), Some(second_branch_id.as_str()));
}

#[test]
fn rejects_setting_active_message_from_another_conversation() {
    let mut store = Store::open_memory().unwrap();
    let first_conversation_id = store.create_conversation("openai/test").unwrap();
    let second_conversation_id = store.create_conversation("openai/test").unwrap();
    let message_id = store
        .insert_message(&first_conversation_id, None, Role::User, "hello", None)
        .unwrap();

    let error = store
        .set_active_message(&second_conversation_id, &message_id)
        .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("message does not belong to conversation")
    );
}

#[test]
fn rejects_saving_message_to_missing_conversation() {
    let mut store = Store::open_memory().unwrap();

    let error = store
        .insert_message(
            &ConversationId::new("missing"),
            None,
            Role::User,
            "hello",
            None,
        )
        .unwrap_err();

    assert!(error.to_string().contains("conversation does not exist"));
}

#[test]
fn saves_message_with_parent_from_same_conversation() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();
    let parent_id = store
        .insert_message(&conversation_id, None, Role::User, "hello", None)
        .unwrap();

    let message_id = store
        .insert_message(
            &conversation_id,
            Some(&parent_id),
            Role::Assistant,
            "hello back",
            None,
        )
        .unwrap();

    let messages = store.load_messages(&conversation_id).unwrap();

    assert_eq!(messages.len(), 2);
    assert_eq!(messages[1].id.as_deref(), Some(message_id.as_str()));
    assert_eq!(
        messages[1].parent_message_id.as_deref(),
        Some(parent_id.as_str())
    );
}

#[test]
fn rejects_message_parent_from_another_conversation() {
    let mut store = Store::open_memory().unwrap();
    let first_conversation_id = store.create_conversation("openai/test").unwrap();
    let second_conversation_id = store.create_conversation("openai/test").unwrap();
    let parent_id = store
        .insert_message(&first_conversation_id, None, Role::User, "hello", None)
        .unwrap();

    let error = store
        .insert_message(
            &second_conversation_id,
            Some(&parent_id),
            Role::Assistant,
            "hello back",
            None,
        )
        .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("message does not belong to conversation")
    );
}

#[test]
fn preserves_metadata() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store
        .get_or_create_default_conversation("openai/test")
        .unwrap();
    let metadata = MessageMetadata {
        tool_calls: vec![ToolCall::function(
            "call_123",
            "run_shell",
            r#"{"command":"ls"}"#,
        )],
        usage: Some(TokenUsage {
            input_tokens: Some(12),
            output_tokens: Some(3),
            total_tokens: Some(15),
            raw: serde_json::json!({
                "input_tokens": 12,
                "output_tokens": 3,
                "total_tokens": 15,
                "output_tokens_details": {
                    "reasoning_tokens": 1
                }
            }),
        }),
        ..Default::default()
    };

    store
        .insert_message(&conversation_id, None, Role::Assistant, "", Some(&metadata))
        .unwrap();

    let messages = store.load_messages(&conversation_id).unwrap();

    assert_eq!(messages[0].metadata.as_ref(), Some(&metadata));
}

#[test]
fn replacing_message_text_preserves_metadata() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store
        .get_or_create_default_conversation("openai/test")
        .unwrap();
    let metadata = MessageMetadata {
        tool_calls: vec![ToolCall::function(
            "call_123",
            "run_shell",
            r#"{"command":"ls"}"#,
        )],
        reasoning: Some("thinking".to_string()),
        ..Default::default()
    };
    let message_id = store
        .insert_message(&conversation_id, None, Role::Assistant, "", Some(&metadata))
        .unwrap();

    store
        .replace_message(&conversation_id, &message_id, "visible text")
        .unwrap();

    let messages = store.load_messages(&conversation_id).unwrap();

    assert_eq!(messages[0].content, "visible text");
    assert_eq!(messages[0].metadata.as_ref(), Some(&metadata));
}

#[test]
fn saves_and_loads_image_message_parts() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store
        .get_or_create_default_conversation("openai/test")
        .unwrap();

    store
        .insert_message_with_parts(
            &conversation_id,
            None,
            Role::User,
            "what is this?",
            &[
                unsaved_text("what is this?"),
                unsaved_image("image/png", &[1, 2, 3]),
            ],
            None,
        )
        .unwrap();

    let messages = store.load_messages(&conversation_id).unwrap();

    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].content, "what is this?");
    assert_eq!(messages[0].parts.len(), 2);
    assert!(matches!(&messages[0].parts[0], MessagePart::Text(text) if text == "what is this?"));
    assert!(matches!(&messages[0].parts[1], MessagePart::Image(image)
        if image.mime_type == "image/png" && image.bytes == vec![1, 2, 3]));
}

#[test]
fn loads_image_asset_only_for_owning_conversation() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();
    let other_conversation_id = store.create_conversation("openai/test").unwrap();

    store
        .insert_message_with_parts(
            &conversation_id,
            None,
            Role::User,
            "image",
            &[
                unsaved_text("image"),
                unsaved_image("image/png", &[1, 2, 3]),
            ],
            None,
        )
        .unwrap();

    let messages = store.load_messages(&conversation_id).unwrap();
    let image_asset_id = match &messages[0].parts[1] {
        MessagePart::Image(image) => image.asset_id.clone(),
        _ => panic!("expected image part"),
    };

    let image = store
        .load_conversation_image_asset(&conversation_id, &image_asset_id)
        .unwrap();
    assert_eq!(image.mime_type, "image/png");
    assert_eq!(image.bytes, vec![1, 2, 3]);

    let error = store
        .load_conversation_image_asset(&other_conversation_id, &image_asset_id)
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        format!("image asset does not exist in conversation: {image_asset_id}")
    );
}

#[test]
fn saves_and_loads_tool_message_parts() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store
        .get_or_create_default_conversation("openai/test")
        .unwrap();
    let user_id = store
        .insert_message(&conversation_id, None, Role::User, "use screenshot", None)
        .unwrap();
    let assistant_id = store
        .insert_message(
            &conversation_id,
            Some(&user_id),
            Role::Assistant,
            "",
            Some(&MessageMetadata {
                tool_calls: vec![ToolCall::function("call_123", "screenshot", "{}")],
                ..Default::default()
            }),
        )
        .unwrap();

    insert_tool_result_with_parts(
        &mut store,
        &conversation_id,
        &assistant_id,
        "call_123",
        "screenshot\n[image: image/png, 3 bytes]",
        &[
            unsaved_text("screenshot"),
            unsaved_image("image/png", &[1, 2, 3]),
        ],
    );

    let messages = store.load_messages(&conversation_id).unwrap();

    assert_eq!(messages.len(), 3);
    assert_eq!(messages[2].role, Role::Tool);
    assert_eq!(
        messages[2]
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.tool_call_id.as_ref())
            .map(ToolCallId::as_str),
        Some("call_123")
    );
    assert_eq!(messages[2].parts.len(), 2);
    assert!(matches!(&messages[2].parts[0], MessagePart::Text(text) if text == "screenshot"));
    assert!(matches!(&messages[2].parts[1], MessagePart::Image(image)
        if image.mime_type == "image/png" && image.bytes == vec![1, 2, 3]));
}

#[test]
fn saves_and_loads_multiple_image_message_parts() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store
        .get_or_create_default_conversation("openai/test")
        .unwrap();

    store
        .insert_message_with_parts(
            &conversation_id,
            None,
            Role::User,
            "compare these",
            &[
                unsaved_text("compare these"),
                unsaved_image("image/png", &[1, 2, 3]),
                unsaved_image("image/jpeg", &[4, 5, 6]),
            ],
            None,
        )
        .unwrap();

    let messages = store.load_messages(&conversation_id).unwrap();

    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].content, "compare these");
    assert_eq!(messages[0].parts.len(), 3);
    assert!(matches!(&messages[0].parts[0], MessagePart::Text(text) if text == "compare these"));
    assert!(matches!(&messages[0].parts[1], MessagePart::Image(image)
        if image.mime_type == "image/png" && image.bytes == vec![1, 2, 3]));
    assert!(matches!(&messages[0].parts[2], MessagePart::Image(image)
        if image.mime_type == "image/jpeg" && image.bytes == vec![4, 5, 6]));
}

#[test]
fn saves_and_loads_interleaved_text_and_image_message_parts() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store
        .get_or_create_default_conversation("openai/test")
        .unwrap();

    store
        .insert_message_with_parts(
            &conversation_id,
            None,
            Role::User,
            "first\nsecond",
            &[
                unsaved_text("first"),
                unsaved_image("image/png", &[1, 2, 3]),
                unsaved_text("second"),
                unsaved_image("image/jpeg", &[4, 5, 6]),
            ],
            None,
        )
        .unwrap();

    let messages = store.load_messages(&conversation_id).unwrap();

    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].content, "first\nsecond");
    assert_eq!(messages[0].parts.len(), 4);
    assert!(matches!(&messages[0].parts[0], MessagePart::Text(text) if text == "first"));
    assert!(matches!(&messages[0].parts[1], MessagePart::Image(image)
        if image.mime_type == "image/png" && image.bytes == vec![1, 2, 3]));
    assert!(matches!(&messages[0].parts[2], MessagePart::Text(text) if text == "second"));
    assert!(matches!(&messages[0].parts[3], MessagePart::Image(image)
        if image.mime_type == "image/jpeg" && image.bytes == vec![4, 5, 6]));
}

#[test]
fn updates_image_message_text_part_with_content() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store
        .get_or_create_default_conversation("openai/test")
        .unwrap();

    let message_id = store
        .insert_message_with_parts(
            &conversation_id,
            None,
            Role::User,
            "what is this?",
            &[
                unsaved_text("what is this?"),
                unsaved_image("image/png", &[1, 2, 3]),
            ],
            None,
        )
        .unwrap();

    store
        .replace_message(&conversation_id, &message_id, "describe this")
        .unwrap();

    let messages = store.load_messages(&conversation_id).unwrap();

    assert_eq!(messages[0].content, "describe this");
    assert_eq!(messages[0].parts.len(), 2);
    assert!(matches!(&messages[0].parts[0], MessagePart::Text(text) if text == "describe this"));
    assert!(matches!(&messages[0].parts[1], MessagePart::Image(image)
        if image.mime_type == "image/png" && image.bytes == vec![1, 2, 3]));
}

#[test]
fn updating_image_message_to_empty_text_removes_text_part() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store
        .get_or_create_default_conversation("openai/test")
        .unwrap();

    let message_id = store
        .insert_message_with_parts(
            &conversation_id,
            None,
            Role::User,
            "what is this?",
            &[
                unsaved_text("what is this?"),
                unsaved_image("image/png", &[1, 2, 3]),
            ],
            None,
        )
        .unwrap();

    store
        .replace_message(&conversation_id, &message_id, "")
        .unwrap();

    let messages = store.load_messages(&conversation_id).unwrap();

    assert_eq!(messages[0].content, "");
    assert_eq!(messages[0].parts.len(), 1);
    assert!(matches!(&messages[0].parts[0], MessagePart::Image(image)
        if image.mime_type == "image/png" && image.bytes == vec![1, 2, 3]));
}

#[test]
fn updates_message_text_without_deleting_later_messages() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store
        .get_or_create_default_conversation("openai/test")
        .unwrap();

    let user_id = store
        .insert_message(&conversation_id, None, Role::User, "helo", None)
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(2));
    let assistant_id = store
        .insert_message(
            &conversation_id,
            Some(&user_id),
            Role::Assistant,
            "hello back",
            None,
        )
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(2));
    store
        .insert_message(
            &conversation_id,
            Some(&assistant_id),
            Role::User,
            "next",
            None,
        )
        .unwrap();

    store
        .replace_message(&conversation_id, &user_id, "hello")
        .unwrap();

    let messages = store.load_messages(&conversation_id).unwrap();

    assert_eq!(messages.len(), 3);
    assert_eq!(messages[0].id.as_deref(), Some(user_id.as_str()));
    assert_eq!(messages[0].role, Role::User);
    assert_eq!(messages[0].content, "hello");
    assert_eq!(messages[1].content, "hello back");
    assert_eq!(messages[2].content, "next");
}

#[test]
fn rejects_updating_message_from_another_conversation() {
    let mut store = Store::open_memory().unwrap();
    let first_conversation_id = store.create_conversation("openai/test").unwrap();
    let second_conversation_id = store.create_conversation("openai/test").unwrap();
    let message_id = store
        .insert_message(&first_conversation_id, None, Role::User, "hello", None)
        .unwrap();

    let error = store
        .replace_message(&second_conversation_id, &message_id, "hi")
        .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("message does not belong to conversation")
    );
}

#[test]
fn rejects_updating_message_in_missing_conversation() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();
    let message_id = store
        .insert_message(&conversation_id, None, Role::User, "hello", None)
        .unwrap();

    let error = store
        .replace_message(&ConversationId::new("missing"), &message_id, "hi")
        .unwrap_err();

    assert!(error.to_string().contains("conversation does not exist"));
}

#[test]
fn remove_message_splices_middle_node_from_chain() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();
    let first_id = store
        .insert_message(&conversation_id, None, Role::User, "one", None)
        .unwrap();
    let second_id = store
        .insert_message(
            &conversation_id,
            Some(&first_id),
            Role::Assistant,
            "two",
            None,
        )
        .unwrap();
    let third_id = store
        .insert_message(
            &conversation_id,
            Some(&second_id),
            Role::User,
            "three",
            None,
        )
        .unwrap();
    store
        .save_compaction(&conversation_id, &third_id, "summary")
        .unwrap();

    store.remove_message(&conversation_id, &second_id).unwrap();

    let messages = store.load_messages(&conversation_id).unwrap();
    let active_message_id = store.active_message_id(&conversation_id).unwrap();
    let compaction = store.latest_compaction(&conversation_id).unwrap();

    assert_eq!(
        message_ids(&messages),
        vec![first_id.to_string(), third_id.to_string()]
    );
    assert_eq!(messages[0].id.as_deref(), Some(first_id.as_str()));
    assert_eq!(message_parent(&messages, &third_id), Some(&first_id));
    assert_eq!(active_message_id.as_deref(), Some(third_id.as_str()));
    assert!(compaction.is_none());
}

#[test]
fn remove_message_splices_branch_point() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();
    let first_id = store
        .insert_message(&conversation_id, None, Role::User, "one", None)
        .unwrap();
    let second_id = store
        .insert_message(
            &conversation_id,
            Some(&first_id),
            Role::Assistant,
            "two",
            None,
        )
        .unwrap();
    let third_id = store
        .insert_message(
            &conversation_id,
            Some(&second_id),
            Role::User,
            "three",
            None,
        )
        .unwrap();
    let fourth_id = store
        .insert_message(&conversation_id, Some(&second_id), Role::User, "four", None)
        .unwrap();

    store.remove_message(&conversation_id, &second_id).unwrap();

    let messages = store.load_messages(&conversation_id).unwrap();
    let active_message_id = store.active_message_id(&conversation_id).unwrap();

    assert_eq!(messages.len(), 3);
    assert_eq!(message_parent(&messages, &third_id), Some(&first_id));
    assert_eq!(message_parent(&messages, &fourth_id), Some(&first_id));
    assert_eq!(active_message_id.as_deref(), Some(fourth_id.as_str()));
}

#[test]
fn remove_message_deletes_leaf_only() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();
    let first_id = store
        .insert_message(&conversation_id, None, Role::User, "one", None)
        .unwrap();
    let second_id = store
        .insert_message(
            &conversation_id,
            Some(&first_id),
            Role::Assistant,
            "two",
            None,
        )
        .unwrap();

    store.remove_message(&conversation_id, &second_id).unwrap();

    let messages = store.load_messages(&conversation_id).unwrap();
    let active_message_id = store.active_message_id(&conversation_id).unwrap();

    assert_eq!(message_ids(&messages), vec![first_id.to_string()]);
    assert_eq!(active_message_id.as_deref(), Some(first_id.as_str()));
}

#[test]
fn remove_root_with_one_child_promotes_child_to_root() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();
    let first_id = store
        .insert_message(&conversation_id, None, Role::User, "one", None)
        .unwrap();
    let second_id = store
        .insert_message(
            &conversation_id,
            Some(&first_id),
            Role::Assistant,
            "two",
            None,
        )
        .unwrap();

    store.remove_message(&conversation_id, &first_id).unwrap();

    let messages = store.load_messages(&conversation_id).unwrap();

    assert_eq!(message_ids(&messages), vec![second_id.to_string()]);
    assert!(message_parent(&messages, &second_id).is_none());
}

#[test]
fn remove_root_with_multiple_children_promotes_children_to_roots() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();
    let first_id = store
        .insert_message(&conversation_id, None, Role::User, "one", None)
        .unwrap();
    let second_id = store
        .insert_message(
            &conversation_id,
            Some(&first_id),
            Role::Assistant,
            "two",
            None,
        )
        .unwrap();
    let third_id = store
        .insert_message(
            &conversation_id,
            Some(&first_id),
            Role::Assistant,
            "three",
            None,
        )
        .unwrap();
    store
        .set_active_message(&conversation_id, &first_id)
        .unwrap();

    store.remove_message(&conversation_id, &first_id).unwrap();

    let messages = store.load_messages(&conversation_id).unwrap();
    let active_message_id = store.active_message_id(&conversation_id).unwrap();

    assert_eq!(
        message_ids(&messages),
        vec![second_id.to_string(), third_id.to_string()]
    );
    assert!(message_parent(&messages, &second_id).is_none());
    assert!(message_parent(&messages, &third_id).is_none());
    assert_eq!(active_message_id.as_deref(), Some(second_id.as_str()));
}

#[test]
fn remove_active_middle_node_moves_active_to_parent() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();
    let first_id = store
        .insert_message(&conversation_id, None, Role::User, "one", None)
        .unwrap();
    let second_id = store
        .insert_message(
            &conversation_id,
            Some(&first_id),
            Role::Assistant,
            "two",
            None,
        )
        .unwrap();
    store
        .insert_message(
            &conversation_id,
            Some(&second_id),
            Role::User,
            "three",
            None,
        )
        .unwrap();
    store
        .set_active_message(&conversation_id, &second_id)
        .unwrap();

    store.remove_message(&conversation_id, &second_id).unwrap();

    let active_message_id = store.active_message_id(&conversation_id).unwrap();

    assert_eq!(active_message_id.as_deref(), Some(first_id.as_str()));
}

#[test]
fn remove_ancestor_keeps_descendant_active() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();
    let first_id = store
        .insert_message(&conversation_id, None, Role::User, "one", None)
        .unwrap();
    let second_id = store
        .insert_message(
            &conversation_id,
            Some(&first_id),
            Role::Assistant,
            "two",
            None,
        )
        .unwrap();
    let third_id = store
        .insert_message(
            &conversation_id,
            Some(&second_id),
            Role::User,
            "three",
            None,
        )
        .unwrap();

    store.remove_message(&conversation_id, &second_id).unwrap();

    let active_message_id = store.active_message_id(&conversation_id).unwrap();
    let active_path = store.load_active_path(&conversation_id).unwrap();

    assert_eq!(active_message_id.as_deref(), Some(third_id.as_str()));
    assert_eq!(
        message_ids(&active_path),
        vec![first_id.to_string(), third_id.to_string()]
    );
}

#[test]
fn remove_message_keeps_unrelated_active() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();
    let first_id = store
        .insert_message(&conversation_id, None, Role::User, "one", None)
        .unwrap();
    let second_id = store
        .insert_message(
            &conversation_id,
            Some(&first_id),
            Role::Assistant,
            "two",
            None,
        )
        .unwrap();
    let third_id = store
        .insert_message(
            &conversation_id,
            Some(&first_id),
            Role::Assistant,
            "three",
            None,
        )
        .unwrap();
    store
        .set_active_message(&conversation_id, &third_id)
        .unwrap();

    store.remove_message(&conversation_id, &second_id).unwrap();

    let active_message_id = store.active_message_id(&conversation_id).unwrap();

    assert_eq!(active_message_id.as_deref(), Some(third_id.as_str()));
}

#[test]
fn remove_message_clears_compactions_and_deletes_orphan_image_assets() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();
    let first_id = store
        .insert_message_with_parts(
            &conversation_id,
            None,
            Role::User,
            "image",
            &[
                unsaved_text("image"),
                unsaved_image("image/png", &[1, 2, 3]),
            ],
            None,
        )
        .unwrap();
    let second_id = store
        .insert_message(
            &conversation_id,
            Some(&first_id),
            Role::Assistant,
            "two",
            None,
        )
        .unwrap();
    store
        .save_compaction(&conversation_id, &second_id, "summary")
        .unwrap();

    assert_eq!(image_asset_count(&store), 1);

    store.remove_message(&conversation_id, &first_id).unwrap();

    let messages = store.load_messages(&conversation_id).unwrap();
    let compaction = store.latest_compaction(&conversation_id).unwrap();

    assert_eq!(message_ids(&messages), vec![second_id.to_string()]);
    assert!(message_parent(&messages, &second_id).is_none());
    assert_eq!(image_asset_count(&store), 0);
    assert!(compaction.is_none());
}

#[test]
fn rejects_deleting_message_from_another_conversation() {
    let mut store = Store::open_memory().unwrap();
    let first_conversation_id = store.create_conversation("openai/test").unwrap();
    let second_conversation_id = store.create_conversation("openai/test").unwrap();
    let message_id = store
        .insert_message(&first_conversation_id, None, Role::User, "hello", None)
        .unwrap();

    let error = store
        .remove_message(&second_conversation_id, &message_id)
        .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("message does not belong to conversation")
    );
}

#[test]
fn truncates_conversation_after_message() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();
    let first_id = store
        .insert_message(&conversation_id, None, Role::User, "one", None)
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(2));
    let second_id = store
        .insert_message(
            &conversation_id,
            Some(&first_id),
            Role::Assistant,
            "two",
            None,
        )
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(2));
    let third_id = store
        .insert_message(
            &conversation_id,
            Some(&second_id),
            Role::User,
            "three",
            None,
        )
        .unwrap();
    store
        .save_compaction(&conversation_id, &third_id, "summary")
        .unwrap();

    store
        .truncate_after_message(&conversation_id, &second_id)
        .unwrap();

    let messages = store.load_messages(&conversation_id).unwrap();
    let active_message_id = store.active_message_id(&conversation_id).unwrap();
    let compaction = store.latest_compaction(&conversation_id).unwrap();

    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].id.as_deref(), Some(first_id.as_str()));
    assert_eq!(messages[1].id.as_deref(), Some(second_id.as_str()));
    assert_eq!(messages[0].content, "one");
    assert_eq!(messages[1].content, "two");
    assert_eq!(active_message_id.as_deref(), Some(second_id.as_str()));
    assert!(compaction.is_none());
}

#[test]
fn rejects_truncating_after_message_from_another_conversation() {
    let mut store = Store::open_memory().unwrap();
    let first_conversation_id = store.create_conversation("openai/test").unwrap();
    let second_conversation_id = store.create_conversation("openai/test").unwrap();
    let message_id = store
        .insert_message(&first_conversation_id, None, Role::User, "hello", None)
        .unwrap();

    let error = store
        .truncate_after_message(&second_conversation_id, &message_id)
        .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("message does not belong to conversation")
    );
}

#[test]
fn forks_conversation_at_message() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();
    store
        .set_conversation_model(&conversation_id, "anthropic/test")
        .unwrap();
    store
        .set_conversation_reasoning_effort(&conversation_id, Some("high"))
        .unwrap();
    let metadata = MessageMetadata {
        tool_calls: vec![ToolCall::function(
            "call_456",
            "run_shell",
            r#"{"command":"pwd"}"#,
        )],
        ..Default::default()
    };
    let first_id = store
        .insert_message(&conversation_id, None, Role::User, "one", None)
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(2));
    let second_id = store
        .insert_message(
            &conversation_id,
            Some(&first_id),
            Role::Assistant,
            "two",
            Some(&metadata),
        )
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(2));
    store
        .insert_message(
            &conversation_id,
            Some(&second_id),
            Role::User,
            "three",
            None,
        )
        .unwrap();

    let forked_conversation_id = store
        .fork_conversation_at_message(&conversation_id, &second_id)
        .unwrap();

    let source_messages = store.load_messages(&conversation_id).unwrap();
    let forked_messages = store.load_messages(&forked_conversation_id).unwrap();
    let forked_active_message_id = store.active_message_id(&forked_conversation_id).unwrap();
    let forked_model = store.conversation_model(&forked_conversation_id).unwrap();
    let forked_reasoning_effort = store
        .conversation_reasoning_effort(&forked_conversation_id)
        .unwrap();

    assert_ne!(forked_conversation_id, conversation_id);
    assert_eq!(forked_model, "anthropic/test");
    assert_eq!(forked_reasoning_effort.as_deref(), Some("high"));
    assert_eq!(source_messages.len(), 3);
    assert_eq!(forked_messages.len(), 2);
    assert_eq!(forked_messages[0].role, Role::User);
    assert_eq!(forked_messages[0].content, "one");
    assert_eq!(forked_messages[1].role, Role::Assistant);
    assert_eq!(forked_messages[1].content, "two");
    assert_eq!(forked_messages[1].metadata.as_ref(), Some(&metadata));
    assert_ne!(forked_messages[0].id.as_deref(), Some(first_id.as_str()));
    assert_eq!(
        forked_messages[1].parent_message_id.as_deref(),
        forked_messages[0].id.as_deref()
    );
    assert_eq!(
        forked_active_message_id.as_deref(),
        forked_messages[1].id.as_deref()
    );
}

#[test]
fn rejects_forking_at_message_from_another_conversation() {
    let mut store = Store::open_memory().unwrap();
    let first_conversation_id = store.create_conversation("openai/test").unwrap();
    let second_conversation_id = store.create_conversation("openai/test").unwrap();
    let message_id = store
        .insert_message(&first_conversation_id, None, Role::User, "hello", None)
        .unwrap();

    let error = store
        .fork_conversation_at_message(&second_conversation_id, &message_id)
        .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("message does not belong to conversation")
    );
}

#[test]
fn branch_response_does_not_override_a_new_active_path() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();
    let original_parent = store
        .insert_message(&conversation_id, None, Role::User, "original", None)
        .unwrap();
    let selected_branch = store
        .insert_message(
            &conversation_id,
            Some(&original_parent),
            Role::User,
            "new branch",
            None,
        )
        .unwrap();

    let response = store
        .insert_assistant_message_on_branch(
            &conversation_id,
            Some(&original_parent),
            "late response",
            None,
        )
        .unwrap();

    assert_eq!(
        store.active_message_id(&conversation_id).unwrap(),
        Some(selected_branch)
    );
    assert_eq!(
        store
            .load_path_to_message(&conversation_id, &response)
            .unwrap()
            .last()
            .unwrap()
            .content,
        "late response"
    );
}

#[test]
fn branch_response_becomes_active_when_selection_is_unchanged() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();
    let parent_id = store
        .insert_message(&conversation_id, None, Role::User, "question", None)
        .unwrap();

    let response_id = store
        .insert_assistant_message_on_branch(&conversation_id, Some(&parent_id), "response", None)
        .unwrap();

    assert_eq!(
        store.active_message_id(&conversation_id).unwrap(),
        Some(response_id)
    );
}
