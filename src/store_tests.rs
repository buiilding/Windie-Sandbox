//! Tests for the SQLite persistence boundary.

use super::*;

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
        .append_message(&conversation_id, None, Role::User, "hello", None)
        .unwrap();

    let conversations = store.list_conversations().unwrap();

    assert_eq!(conversations.len(), 1);
    assert_eq!(conversations[0].id, conversation_id);
    assert_eq!(conversations[0].message_count, 1);
}

#[test]
fn loads_empty_messages_for_existing_conversation() {
    let store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation().unwrap();

    let messages = store.load_messages(&conversation_id).unwrap();

    assert!(messages.is_empty());
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
        .append_message(&conversation_id, None, Role::User, "hello", None)
        .unwrap();
    let assistant_id = store
        .append_message(
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
fn rejects_saving_message_to_missing_conversation() {
    let mut store = Store::open_memory().unwrap();

    let error = store
        .append_message(
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
        .append_message(&conversation_id, None, Role::User, "hello", None)
        .unwrap();

    let message_id = store
        .append_message(
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
        .append_message(&first_conversation_id, None, Role::User, "hello", None)
        .unwrap();

    let error = store
        .append_message(
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

    store
        .append_message(
            &conversation_id,
            None,
            Role::Assistant,
            "",
            Some(r#"{"tool_calls":[]}"#),
        )
        .unwrap();

    let messages = store.load_messages(&conversation_id).unwrap();

    assert_eq!(
        messages[0].metadata.as_deref(),
        Some(r#"{"tool_calls":[]}"#)
    );
}

#[test]
fn loads_messages_after_checkpoint() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.get_or_create_default_conversation().unwrap();

    let first_id = store
        .append_message(&conversation_id, None, Role::User, "one", None)
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(2));
    let second_id = store
        .append_message(
            &conversation_id,
            Some(&first_id),
            Role::Assistant,
            "two",
            None,
        )
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(2));
    store
        .append_message(
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
        .append_message(&first_conversation_id, None, Role::User, "one", None)
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
        .append_message(&conversation_id, None, Role::User, "one", None)
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
        .append_message(&conversation_id, None, Role::User, "helo", None)
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(2));
    let assistant_id = store
        .append_message(
            &conversation_id,
            Some(&user_id),
            Role::Assistant,
            "hello back",
            None,
        )
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(2));
    store
        .append_message(
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
        .append_message(&first_conversation_id, None, Role::User, "hello", None)
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
        .append_message(&conversation_id, None, Role::User, "hello", None)
        .unwrap();

    let error = store
        .replace_message(&ConversationId::new("missing"), &message_id, "hi")
        .unwrap_err();

    assert!(error.to_string().contains("conversation does not exist"));
}

#[test]
fn deletes_message_and_reconnects_child_messages() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation().unwrap();
    let first_id = store
        .append_message(&conversation_id, None, Role::User, "one", None)
        .unwrap();
    let second_id = store
        .append_message(
            &conversation_id,
            Some(&first_id),
            Role::Assistant,
            "two",
            None,
        )
        .unwrap();
    let third_id = store
        .append_message(
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
    let compaction = store.latest_compaction(&conversation_id).unwrap();

    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].id.as_deref(), Some(first_id.as_str()));
    assert_eq!(messages[1].id.as_deref(), Some(third_id.as_str()));
    assert_eq!(
        messages[1].parent_message_id.as_deref(),
        Some(first_id.as_str())
    );
    assert!(compaction.is_none());
}

#[test]
fn rejects_deleting_message_from_another_conversation() {
    let mut store = Store::open_memory().unwrap();
    let first_conversation_id = store.create_conversation().unwrap();
    let second_conversation_id = store.create_conversation().unwrap();
    let message_id = store
        .append_message(&first_conversation_id, None, Role::User, "hello", None)
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
        .append_message(&conversation_id, None, Role::User, "one", None)
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(2));
    let second_id = store
        .append_message(
            &conversation_id,
            Some(&first_id),
            Role::Assistant,
            "two",
            None,
        )
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(2));
    let third_id = store
        .append_message(
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
    let compaction = store.latest_compaction(&conversation_id).unwrap();

    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].id.as_deref(), Some(first_id.as_str()));
    assert_eq!(messages[1].id.as_deref(), Some(second_id.as_str()));
    assert_eq!(messages[0].content, "one");
    assert_eq!(messages[1].content, "two");
    assert!(compaction.is_none());
}

#[test]
fn rejects_truncating_after_message_from_another_conversation() {
    let mut store = Store::open_memory().unwrap();
    let first_conversation_id = store.create_conversation().unwrap();
    let second_conversation_id = store.create_conversation().unwrap();
    let message_id = store
        .append_message(&first_conversation_id, None, Role::User, "hello", None)
        .unwrap();

    let error = store
        .truncate_after_message(&second_conversation_id, &message_id)
        .unwrap_err();

    assert!(error.to_string().contains("message does not exist"));
}

#[test]
fn forks_conversation_at_message() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation().unwrap();
    let first_id = store
        .append_message(&conversation_id, None, Role::User, "one", None)
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(2));
    let second_id = store
        .append_message(
            &conversation_id,
            Some(&first_id),
            Role::Assistant,
            "two",
            Some(r#"{"source":"test"}"#),
        )
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(2));
    store
        .append_message(
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

    assert_ne!(forked_conversation_id, conversation_id);
    assert_eq!(source_messages.len(), 3);
    assert_eq!(forked_messages.len(), 2);
    assert_eq!(forked_messages[0].role, Role::User);
    assert_eq!(forked_messages[0].content, "one");
    assert_eq!(forked_messages[1].role, Role::Assistant);
    assert_eq!(forked_messages[1].content, "two");
    assert_eq!(
        forked_messages[1].metadata.as_deref(),
        Some(r#"{"source":"test"}"#)
    );
    assert_ne!(forked_messages[0].id.as_deref(), Some(first_id.as_str()));
    assert_eq!(
        forked_messages[1].parent_message_id.as_deref(),
        forked_messages[0].id.as_deref()
    );
}

#[test]
fn rejects_forking_at_message_from_another_conversation() {
    let mut store = Store::open_memory().unwrap();
    let first_conversation_id = store.create_conversation().unwrap();
    let second_conversation_id = store.create_conversation().unwrap();
    let message_id = store
        .append_message(&first_conversation_id, None, Role::User, "hello", None)
        .unwrap();

    let error = store
        .fork_conversation_at_message(&second_conversation_id, &message_id)
        .unwrap_err();

    assert!(error.to_string().contains("message does not exist"));
}

#[test]
fn deletes_conversation() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation().unwrap();
    let message_id = store
        .append_message(&conversation_id, None, Role::User, "hello", None)
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
        .append_message(&conversation_id, None, Role::User, "hello", None)
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
        .append_message(&first_conversation_id, None, Role::User, "hello", None)
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
        .append_message(&conversation_id, None, Role::User, "hello", None)
        .unwrap();

    let error = store
        .save_compaction(&ConversationId::new("missing"), &message_id, "summary")
        .unwrap_err();

    assert!(error.to_string().contains("conversation does not exist"));
}
