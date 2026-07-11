//! Compactions persistence tests.

use super::*;

#[test]
fn saves_and_loads_latest_compaction() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store
        .get_or_create_default_conversation("openai/test")
        .unwrap();
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
    let conversation_id = store.create_conversation("openai/test").unwrap();

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
    let first_conversation_id = store.create_conversation("openai/test").unwrap();
    let second_conversation_id = store.create_conversation("openai/test").unwrap();
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
    let conversation_id = store.create_conversation("openai/test").unwrap();
    let message_id = store
        .insert_message(&conversation_id, None, Role::User, "hello", None)
        .unwrap();

    let error = store
        .save_compaction(&ConversationId::new("missing"), &message_id, "summary")
        .unwrap_err();

    assert!(error.to_string().contains("conversation does not exist"));
}
