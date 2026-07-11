//! Conversations persistence tests.

use super::*;

#[test]
fn creates_default_conversation() {
    let store = Store::open_memory().unwrap();

    let conversation_id = store
        .get_or_create_default_conversation("openai/test")
        .unwrap();

    assert_eq!(conversation_id.as_str(), "default");
}

#[test]
fn creates_conversation_with_unique_id() {
    let store = Store::open_memory().unwrap();

    let first_id = store.create_conversation("openai/test").unwrap();
    let second_id = store.create_conversation("openai/test").unwrap();

    assert_ne!(first_id, second_id);
}

#[test]
fn creates_conversation_with_model() {
    let store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation("anthropic/test").unwrap();

    let model = store.conversation_model(&conversation_id).unwrap();
    let reasoning_effort = store
        .conversation_reasoning_effort(&conversation_id)
        .unwrap();
    let conversations = store.list_conversations().unwrap();

    assert_eq!(model, "anthropic/test");
    assert_eq!(reasoning_effort, None);
    assert_eq!(conversations[0].model, "anthropic/test");
}

#[test]
fn set_conversation_model_persists() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();

    store
        .set_conversation_model(&conversation_id, "anthropic/test")
        .unwrap();

    let model = store.conversation_model(&conversation_id).unwrap();

    assert_eq!(model, "anthropic/test");
}

#[test]
fn set_conversation_reasoning_effort_persists() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();

    store
        .set_conversation_reasoning_effort(&conversation_id, Some(" high "))
        .unwrap();

    let reasoning_effort = store
        .conversation_reasoning_effort(&conversation_id)
        .unwrap();

    assert_eq!(reasoning_effort.as_deref(), Some("high"));
}

#[test]
fn setting_conversation_model_clears_reasoning_effort() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();

    store
        .set_conversation_reasoning_effort(&conversation_id, Some("high"))
        .unwrap();
    store
        .set_conversation_model(&conversation_id, "anthropic/test")
        .unwrap();

    let reasoning_effort = store
        .conversation_reasoning_effort(&conversation_id)
        .unwrap();

    assert_eq!(reasoning_effort, None);
}

#[test]
fn rejects_empty_conversation_model() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();

    let create_error = store.create_conversation("  ").unwrap_err();
    let set_error = store
        .set_conversation_model(&conversation_id, "  ")
        .unwrap_err();

    assert_eq!(create_error.to_string(), "model requires non-empty text");
    assert_eq!(set_error.to_string(), "model requires non-empty text");
}

#[test]
fn lists_conversations() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();
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
    let conversation_id = store.create_conversation("openai/test").unwrap();

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
    let conversation_id = store.create_conversation("openai/test").unwrap();

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
fn deletes_conversation() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();
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
fn clear_conversation_reasoning_effort_persists() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();

    store
        .set_conversation_reasoning_effort(&conversation_id, Some("high"))
        .unwrap();
    store
        .set_conversation_reasoning_effort(&conversation_id, None)
        .unwrap();

    let reasoning_effort = store
        .conversation_reasoning_effort(&conversation_id)
        .unwrap();

    assert_eq!(reasoning_effort, None);
}
