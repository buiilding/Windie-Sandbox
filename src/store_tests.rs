//! Tests for the SQLite persistence boundary.

use super::*;
use crate::conversation::{MessagePart, ToolCall, ToolSchema, ToolSchemaName};

fn index_exists(store: &Store, index_name: &str) -> bool {
    store
        .connection
        .query_row(
            "
            SELECT EXISTS (
                SELECT 1
                FROM sqlite_master
                WHERE type = 'index'
                  AND name = ?1
            )
            ",
            [index_name],
            |row| row.get(0),
        )
        .unwrap()
}

#[test]
fn creates_default_conversation() {
    let store = Store::open_memory().unwrap();

    let conversation_id = store.get_or_create_default_conversation().unwrap();

    assert_eq!(conversation_id.as_str(), "default");
}

#[test]
fn sets_database_schema_version() {
    let store = Store::open_memory().unwrap();
    let version: i32 = store
        .connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();

    assert_eq!(version, DATABASE_SCHEMA_VERSION);
}

#[test]
fn creates_performance_indexes() {
    let store = Store::open_memory().unwrap();

    assert!(index_exists(&store, "messages_parent_idx"));
    assert!(index_exists(&store, "conversations_updated_idx"));
}

#[test]
fn rejects_newer_database_schema_version() {
    let store = Store::open_memory().unwrap();
    let newer_version = DATABASE_SCHEMA_VERSION + 1;
    store
        .connection
        .pragma_update(None, "user_version", newer_version)
        .unwrap();

    let error = store.migrate().unwrap_err();

    assert_eq!(
        error.to_string(),
        format!(
            "database schema version {newer_version} is newer than supported version {DATABASE_SCHEMA_VERSION}"
        )
    );
}

#[test]
fn migrates_active_message_id_for_existing_database() {
    let path =
        std::env::temp_dir().join(format!("windie-migration-test-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute_batch(
            "
            CREATE TABLE conversations (
                id TEXT PRIMARY KEY,
                title TEXT,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );

            CREATE TABLE messages (
                id TEXT PRIMARY KEY,
                conversation_id TEXT NOT NULL,
                parent_message_id TEXT,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                metadata TEXT,
                created_at INTEGER NOT NULL
            );

            INSERT INTO conversations (id, title, created_at, updated_at)
            VALUES ('conversation-id', NULL, 1, 3);
            INSERT INTO messages (id, conversation_id, parent_message_id, role, content, metadata, created_at)
            VALUES ('first-id', 'conversation-id', NULL, 'user', 'one', NULL, 1);
            INSERT INTO messages (id, conversation_id, parent_message_id, role, content, metadata, created_at)
            VALUES ('second-id', 'conversation-id', 'first-id', 'assistant', 'two', NULL, 2);
            PRAGMA user_version = 1;
            ",
        )
        .unwrap();
    drop(connection);

    let store = Store::open_at(&path).unwrap();
    let active_message_id = store
        .active_message_id(&ConversationId::new("conversation-id"))
        .unwrap();

    assert_eq!(active_message_id.as_deref(), Some("second-id"));

    let _ = std::fs::remove_file(path);
}

#[test]
fn creates_conversation_with_unique_id() {
    let store = Store::open_memory().unwrap();

    let first_id = store.create_conversation().unwrap();
    let second_id = store.create_conversation().unwrap();

    assert_ne!(first_id, second_id);
}

#[test]
fn lists_conversations() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation().unwrap();
    store
        .insert_message(&conversation_id, None, Role::User, "hello", None)
        .unwrap();

    let conversations = store.list_conversations().unwrap();

    assert_eq!(conversations.len(), 1);
    assert_eq!(conversations[0].id, conversation_id);
    assert_eq!(conversations[0].message_count, 1);
}

#[test]
fn sets_and_replaces_system_prompt() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation().unwrap();

    assert!(store.system_prompt(&conversation_id).unwrap().is_none());

    store
        .set_system_prompt(&conversation_id, "You are direct.")
        .unwrap();
    assert_eq!(
        store.system_prompt(&conversation_id).unwrap().as_deref(),
        Some("You are direct.")
    );

    store
        .set_system_prompt(&conversation_id, "You are concise.")
        .unwrap();
    assert_eq!(
        store.system_prompt(&conversation_id).unwrap().as_deref(),
        Some("You are concise.")
    );
}

#[test]
fn clears_system_prompt_with_empty_text() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation().unwrap();

    store
        .set_system_prompt(&conversation_id, "You are direct.")
        .unwrap();
    store.set_system_prompt(&conversation_id, "").unwrap();

    assert!(store.system_prompt(&conversation_id).unwrap().is_none());
}

#[test]
fn rejects_system_prompt_for_missing_conversation() {
    let mut store = Store::open_memory().unwrap();

    let error = store
        .set_system_prompt(&ConversationId::new("missing"), "prompt")
        .unwrap_err();

    assert!(error.to_string().contains("conversation does not exist"));
}

#[test]
fn loads_empty_messages_for_existing_conversation() {
    let store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation().unwrap();

    let messages = store.load_messages(&conversation_id).unwrap();

    assert!(messages.is_empty());
}

#[test]
fn loads_empty_active_path_for_empty_conversation() {
    let store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation().unwrap();

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
    let conversation_id = store.get_or_create_default_conversation().unwrap();

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
    let conversation_id = store.create_conversation().unwrap();

    let message_id = store
        .insert_message(&conversation_id, None, Role::User, "hello", None)
        .unwrap();

    let active_message_id = store.active_message_id(&conversation_id).unwrap();

    assert_eq!(active_message_id.as_deref(), Some(message_id.as_str()));
}

#[test]
fn loads_active_path() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation().unwrap();
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
    let first_conversation_id = store.create_conversation().unwrap();
    let second_conversation_id = store.create_conversation().unwrap();
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
    let conversation_id = store.create_conversation().unwrap();
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
    let first_conversation_id = store.create_conversation().unwrap();
    let second_conversation_id = store.create_conversation().unwrap();
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
    let conversation_id = store.get_or_create_default_conversation().unwrap();
    let metadata = MessageMetadata {
        tool_calls: vec![ToolCall::function(
            "call_123",
            "run_shell",
            r#"{"command":"ls"}"#,
        )],
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
    let conversation_id = store.get_or_create_default_conversation().unwrap();
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
fn saves_updates_and_removes_tool_schemas() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.get_or_create_default_conversation().unwrap();
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
fn rejects_non_object_tool_schema_parameters() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.get_or_create_default_conversation().unwrap();
    let tool_schema = ToolSchema {
        name: ToolSchemaName::new("bad"),
        description: "Bad schema".to_string(),
        parameters: serde_json::json!("not an object"),
    };

    let error = store
        .insert_tool_schema(&conversation_id, &tool_schema)
        .unwrap_err();

    assert!(error.to_string().contains("failed to encode tool schema"));
}

#[test]
fn rejects_invalid_tool_schema_name() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.get_or_create_default_conversation().unwrap();
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
    let conversation_id = store.get_or_create_default_conversation().unwrap();
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
fn saves_and_loads_image_message_parts() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.get_or_create_default_conversation().unwrap();

    store
        .insert_user_message_with_parts(
            &conversation_id,
            None,
            "what is this?",
            &[
                MessagePayload::Text("what is this?"),
                MessagePayload::Image(ImagePayload {
                    mime_type: "image/png",
                    bytes: &[1, 2, 3],
                }),
            ],
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
fn saves_and_loads_multiple_image_message_parts() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.get_or_create_default_conversation().unwrap();

    store
        .insert_user_message_with_parts(
            &conversation_id,
            None,
            "compare these",
            &[
                MessagePayload::Text("compare these"),
                MessagePayload::Image(ImagePayload {
                    mime_type: "image/png",
                    bytes: &[1, 2, 3],
                }),
                MessagePayload::Image(ImagePayload {
                    mime_type: "image/jpeg",
                    bytes: &[4, 5, 6],
                }),
            ],
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
    let conversation_id = store.get_or_create_default_conversation().unwrap();

    store
        .insert_user_message_with_parts(
            &conversation_id,
            None,
            "first\nsecond",
            &[
                MessagePayload::Text("first"),
                MessagePayload::Image(ImagePayload {
                    mime_type: "image/png",
                    bytes: &[1, 2, 3],
                }),
                MessagePayload::Text("second"),
                MessagePayload::Image(ImagePayload {
                    mime_type: "image/jpeg",
                    bytes: &[4, 5, 6],
                }),
            ],
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
    let conversation_id = store.get_or_create_default_conversation().unwrap();

    let message_id = store
        .insert_user_message_with_parts(
            &conversation_id,
            None,
            "what is this?",
            &[
                MessagePayload::Text("what is this?"),
                MessagePayload::Image(ImagePayload {
                    mime_type: "image/png",
                    bytes: &[1, 2, 3],
                }),
            ],
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
    let conversation_id = store.get_or_create_default_conversation().unwrap();

    let message_id = store
        .insert_user_message_with_parts(
            &conversation_id,
            None,
            "what is this?",
            &[
                MessagePayload::Text("what is this?"),
                MessagePayload::Image(ImagePayload {
                    mime_type: "image/png",
                    bytes: &[1, 2, 3],
                }),
            ],
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
fn loads_messages_after_checkpoint() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.get_or_create_default_conversation().unwrap();

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
    store
        .insert_message(
            &conversation_id,
            Some(&second_id),
            Role::User,
            "three",
            None,
        )
        .unwrap();

    let messages = store
        .load_messages_after(&conversation_id, Some(&first_id))
        .unwrap();

    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].content, "two");
    assert_eq!(messages[1].content, "three");
}

#[test]
fn rejects_load_messages_after_checkpoint_from_another_conversation() {
    let mut store = Store::open_memory().unwrap();
    let first_conversation_id = store.create_conversation().unwrap();
    let second_conversation_id = store.create_conversation().unwrap();
    let first_message_id = store
        .insert_message(&first_conversation_id, None, Role::User, "one", None)
        .unwrap();

    let error = store
        .load_messages_after(&second_conversation_id, Some(&first_message_id))
        .unwrap_err();

    assert!(error.to_string().contains("message does not exist"));
}

#[test]
fn rejects_load_messages_after_missing_conversation() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation().unwrap();
    let message_id = store
        .insert_message(&conversation_id, None, Role::User, "one", None)
        .unwrap();

    let error = store
        .load_messages_after(&ConversationId::new("missing"), Some(&message_id))
        .unwrap_err();

    assert!(error.to_string().contains("conversation does not exist"));
}

#[test]
fn updates_message_text_without_deleting_later_messages() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.get_or_create_default_conversation().unwrap();

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
    let first_conversation_id = store.create_conversation().unwrap();
    let second_conversation_id = store.create_conversation().unwrap();
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
    let conversation_id = store.create_conversation().unwrap();
    let message_id = store
        .insert_message(&conversation_id, None, Role::User, "hello", None)
        .unwrap();

    let error = store
        .replace_message(&ConversationId::new("missing"), &message_id, "hi")
        .unwrap_err();

    assert!(error.to_string().contains("conversation does not exist"));
}

#[test]
fn deletes_message_subtree() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation().unwrap();
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

    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].id.as_deref(), Some(first_id.as_str()));
    assert_eq!(active_message_id.as_deref(), Some(first_id.as_str()));
    assert!(compaction.is_none());
}

#[test]
fn rejects_deleting_message_from_another_conversation() {
    let mut store = Store::open_memory().unwrap();
    let first_conversation_id = store.create_conversation().unwrap();
    let second_conversation_id = store.create_conversation().unwrap();
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
    let conversation_id = store.create_conversation().unwrap();
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
    let first_conversation_id = store.create_conversation().unwrap();
    let second_conversation_id = store.create_conversation().unwrap();
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
    let conversation_id = store.create_conversation().unwrap();
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

    assert_ne!(forked_conversation_id, conversation_id);
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
    let first_conversation_id = store.create_conversation().unwrap();
    let second_conversation_id = store.create_conversation().unwrap();
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
fn deletes_conversation() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation().unwrap();
    let message_id = store
        .insert_message(&conversation_id, None, Role::User, "hello", None)
        .unwrap();
    store
        .save_compaction(&conversation_id, &message_id, "summary")
        .unwrap();

    store.remove_conversation(&conversation_id).unwrap();

    assert!(store.list_conversations().unwrap().is_empty());
}

#[test]
fn rejects_deleting_missing_conversation() {
    let mut store = Store::open_memory().unwrap();

    let error = store
        .remove_conversation(&ConversationId::new("missing"))
        .unwrap_err();

    assert!(error.to_string().contains("conversation does not exist"));
}

#[test]
fn saves_and_loads_latest_compaction() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.get_or_create_default_conversation().unwrap();
    let message_id = store
        .insert_message(&conversation_id, None, Role::User, "hello", None)
        .unwrap();

    let compaction_id = store
        .save_compaction(&conversation_id, &message_id, "summary")
        .unwrap();

    let compaction = store.latest_compaction(&conversation_id).unwrap().unwrap();

    assert_eq!(compaction.id, compaction_id);
    assert_eq!(compaction.conversation_id, conversation_id);
    assert_eq!(compaction.through_message_id, message_id);
    assert_eq!(compaction.content, "summary");
}

#[test]
fn loads_no_latest_compaction_for_existing_conversation() {
    let store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation().unwrap();

    let compaction = store.latest_compaction(&conversation_id).unwrap();

    assert!(compaction.is_none());
}

#[test]
fn rejects_latest_compaction_for_missing_conversation() {
    let store = Store::open_memory().unwrap();

    let error = store
        .latest_compaction(&ConversationId::new("missing"))
        .unwrap_err();

    assert!(error.to_string().contains("conversation does not exist"));
}

#[test]
fn rejects_compaction_checkpoint_from_another_conversation() {
    let mut store = Store::open_memory().unwrap();
    let first_conversation_id = store.create_conversation().unwrap();
    let second_conversation_id = store.create_conversation().unwrap();
    let message_id = store
        .insert_message(&first_conversation_id, None, Role::User, "hello", None)
        .unwrap();

    let error = store
        .save_compaction(&second_conversation_id, &message_id, "summary")
        .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("message does not belong to conversation")
    );
}

#[test]
fn rejects_saving_compaction_to_missing_conversation() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation().unwrap();
    let message_id = store
        .insert_message(&conversation_id, None, Role::User, "hello", None)
        .unwrap();

    let error = store
        .save_compaction(&ConversationId::new("missing"), &message_id, "summary")
        .unwrap_err();

    assert!(error.to_string().contains("conversation does not exist"));
}
