//! Tests for the SQLite persistence boundary.

use super::*;
use crate::conversation::{
    MessagePart, TokenUsage, ToolCall, UnsavedImagePart, UnsavedMessagePart,
};
use crate::session::{SessionId, SessionStatus};
use crate::tool::ToolProviderId;
use crate::tool::{ToolApprovalMode, ToolSchema, ToolSchemaName};
use crate::tool_provider::ProviderInstallState;

fn unsaved_text(text: &str) -> UnsavedMessagePart {
    UnsavedMessagePart::Text(text.to_string())
}

fn unsaved_image(mime_type: &str, bytes: &[u8]) -> UnsavedMessagePart {
    UnsavedMessagePart::Image(UnsavedImagePart {
        mime_type: mime_type.to_string(),
        bytes: bytes.to_vec(),
    })
}

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
fn provider_lifecycle_state_persists_and_uninstalls() {
    let store = Store::open_memory().unwrap();
    let provider_id = ToolProviderId::new("desktop-commander");

    let installed = store.install_provider(&provider_id).unwrap();
    assert_eq!(installed.state, ProviderInstallState::Installed);
    assert!(installed.error.is_none());

    let enabled = store
        .set_provider_state(&provider_id, ProviderInstallState::Enabled, None)
        .unwrap();
    assert_eq!(enabled.state, ProviderInstallState::Enabled);

    let broken = store
        .record_provider_health(
            &provider_id,
            ProviderInstallState::Broken,
            Some("npx is missing"),
        )
        .unwrap();
    assert_eq!(broken.state, ProviderInstallState::Broken);
    assert_eq!(broken.error.as_deref(), Some("npx is missing"));
    assert!(broken.last_health_check_at.is_some());

    store.uninstall_provider(&provider_id).unwrap();
    assert!(
        store
            .load_installed_provider(&provider_id)
            .unwrap()
            .is_none()
    );
}

fn message_parent<'a>(messages: &'a [Message], message_id: &MessageId) -> Option<&'a MessageId> {
    messages
        .iter()
        .find(|message| message.id.as_ref() == Some(message_id))
        .and_then(|message| message.parent_message_id.as_ref())
}

fn message_ids(messages: &[Message]) -> Vec<String> {
    messages
        .iter()
        .filter_map(|message| message.id.as_ref())
        .map(ToString::to_string)
        .collect()
}

fn image_asset_count(store: &Store) -> i64 {
    store
        .connection
        .query_row("SELECT COUNT(*) FROM image_assets", [], |row| row.get(0))
        .unwrap()
}

fn insert_tool_result(
    store: &mut Store,
    conversation_id: &ConversationId,
    parent_message_id: &MessageId,
    tool_call_id: &str,
    content: &str,
) -> MessageId {
    store
        .insert_tool_result_message(
            conversation_id,
            parent_message_id,
            &ToolCallId::new(tool_call_id),
            content,
        )
        .unwrap()
}

fn insert_tool_result_with_parts(
    store: &mut Store,
    conversation_id: &ConversationId,
    parent_message_id: &MessageId,
    tool_call_id: &str,
    content: &str,
    parts: &[UnsavedMessagePart],
) -> MessageId {
    store
        .insert_run_tool_result_message_with_parts(
            conversation_id,
            parent_message_id,
            &ToolCallId::new(tool_call_id),
            content,
            parts,
        )
        .unwrap()
}

#[test]
fn creates_default_conversation() {
    let store = Store::open_memory().unwrap();

    let conversation_id = store
        .get_or_create_default_conversation("openai/test")
        .unwrap();

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
    assert!(index_exists(&store, "messages_id_conversation_idx"));
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
fn rejects_older_database_schema_version() {
    let store = Store::open_memory().unwrap();
    let older_version = DATABASE_SCHEMA_VERSION - 1;
    store
        .connection
        .pragma_update(None, "user_version", older_version)
        .unwrap();

    let error = store.migrate().unwrap_err();

    assert_eq!(
        error.to_string(),
        format!(
            "database schema version {older_version} is older than supported version {DATABASE_SCHEMA_VERSION}; remove the old Windie database or recreate it"
        )
    );
}

#[test]
fn rejects_existing_unversioned_database_schema() {
    let store = Store::open_memory().unwrap();
    store
        .connection
        .pragma_update(None, "user_version", 0)
        .unwrap();

    let error = store.migrate().unwrap_err();

    assert_eq!(
        error.to_string(),
        "existing unversioned Windie database is not supported; remove the old Windie database or recreate it"
    );
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
fn system_prompt_is_tree_wide_same_for_any_head() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();
    let root_id = store
        .insert_message(&conversation_id, None, Role::User, "root", None)
        .unwrap();
    let branch_id = store
        .insert_message(&conversation_id, Some(&root_id), Role::User, "branch", None)
        .unwrap();
    let sibling_id = store
        .insert_message(
            &conversation_id,
            Some(&root_id),
            Role::User,
            "sibling",
            None,
        )
        .unwrap();

    store
        .set_system_prompt(&conversation_id, "global prompt")
        .unwrap();

    assert_eq!(
        store.system_prompt(&conversation_id).unwrap().as_deref(),
        Some("global prompt")
    );
    // Both heads should see same prompt via ContextBuilder (tree-wide)
    assert_eq!(
        store.system_prompt(&conversation_id).unwrap().as_deref(),
        Some("global prompt")
    );
    // Ensure branch ids exist still
    assert!(
        store
            .load_path_to_message(&conversation_id, &branch_id)
            .is_ok()
    );
    assert!(
        store
            .load_path_to_message(&conversation_id, &sibling_id)
            .is_ok()
    );
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
    let conversation_id = store.create_conversation("openai/test").unwrap();

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
fn loads_path_to_message() {
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

    let path = store
        .load_path_to_message(&conversation_id, &first_branch_id)
        .unwrap();

    assert_eq!(path.len(), 2);
    assert_eq!(path[0].id.as_deref(), Some(root_id.as_str()));
    assert_eq!(path[1].id.as_deref(), Some(first_branch_id.as_str()));
    assert_ne!(path[1].id.as_deref(), Some(second_branch_id.as_str()));
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
fn tool_schemas_are_tree_wide_same_for_any_head() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();
    let shared_tool = ToolSchema {
        name: ToolSchemaName::new("shared_tool"),
        description: "Shared tool".to_string(),
        parameters: serde_json::json!({"type":"object"}),
    };
    let root_id = store
        .insert_message(&conversation_id, None, Role::User, "root", None)
        .unwrap();
    let branch_id = store
        .insert_message(&conversation_id, Some(&root_id), Role::User, "branch", None)
        .unwrap();
    let sibling_id = store
        .insert_message(
            &conversation_id,
            Some(&root_id),
            Role::User,
            "sibling",
            None,
        )
        .unwrap();
    store
        .insert_tool_schema(&conversation_id, &shared_tool)
        .unwrap();

    assert_eq!(
        store.load_tool_schemas(&conversation_id).unwrap(),
        vec![shared_tool.clone()]
    );
    // Both branches see same tools tree-wide
    assert!(
        store
            .load_path_to_message(&conversation_id, &branch_id)
            .is_ok()
    );
    assert!(
        store
            .load_path_to_message(&conversation_id, &sibling_id)
            .is_ok()
    );
    assert_eq!(
        store.load_tool_schemas(&conversation_id).unwrap(),
        vec![shared_tool]
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
fn loads_messages_after_checkpoint() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store
        .get_or_create_default_conversation("openai/test")
        .unwrap();

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
    let first_conversation_id = store.create_conversation("openai/test").unwrap();
    let second_conversation_id = store.create_conversation("openai/test").unwrap();
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
    let conversation_id = store.create_conversation("openai/test").unwrap();
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
    let compaction = store.latest_compaction(&conversation_id).unwrap();

    assert_eq!(
        message_ids(&messages),
        vec![first_id.to_string(), third_id.to_string()]
    );
    assert_eq!(messages[0].id.as_deref(), Some(first_id.as_str()));
    assert_eq!(message_parent(&messages, &third_id), Some(&first_id));
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

    assert_eq!(messages.len(), 3);
    assert_eq!(message_parent(&messages, &third_id), Some(&first_id));
    assert_eq!(message_parent(&messages, &fourth_id), Some(&first_id));
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

    assert_eq!(message_ids(&messages), vec![first_id.to_string()]);
}

#[test]
fn remove_message_clears_deleted_runtime_run_head() {
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
    let session_id = SessionId::new("run-delete-head");
    store
        .create_session(
            &session_id,
            &conversation_id,
            Some(&second_id),
            "openai/test",
            None,
        )
        .unwrap();

    store.remove_message(&conversation_id, &second_id).unwrap();

    let run = store.load_session(&session_id).unwrap();

    assert_eq!(run.start_head_message_id, None);
    assert_eq!(run.current_head_message_id, None);
    // A ready branch whose head was deleted is not running, so it stays ready
    // rather than being cancelled; only its dangling head pointer is cleared.
    assert_eq!(run.status, SessionStatus::Ready);
}

#[test]
fn remove_message_clears_deleted_session_start_but_keeps_surviving_current_head() {
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
    let session_id = SessionId::new("run-delete-start");
    store
        .create_session(
            &session_id,
            &conversation_id,
            Some(&second_id),
            "openai/test",
            None,
        )
        .unwrap();
    store
        .update_session_head(&session_id, Some(&third_id))
        .unwrap();

    store.remove_message(&conversation_id, &second_id).unwrap();

    let run = store.load_session(&session_id).unwrap();
    let messages = store.load_messages(&conversation_id).unwrap();

    assert_eq!(run.start_head_message_id, None);
    assert_eq!(run.current_head_message_id.as_ref(), Some(&third_id));
    // The surviving current head keeps the branch resumable; deleting only the
    // start pointer does not cancel a ready branch.
    assert_eq!(run.status, SessionStatus::Ready);
    assert_eq!(message_parent(&messages, &third_id), Some(&first_id));
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
    store.remove_message(&conversation_id, &first_id).unwrap();

    let messages = store.load_messages(&conversation_id).unwrap();

    assert_eq!(
        message_ids(&messages),
        vec![second_id.to_string(), third_id.to_string()]
    );
    assert!(message_parent(&messages, &second_id).is_none());
    assert!(message_parent(&messages, &third_id).is_none());
}

#[test]
fn remove_middle_node_splices_descendant_to_parent() {
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

    let messages = store.load_messages(&conversation_id).unwrap();

    assert_eq!(message_parent(&messages, &third_id), Some(&first_id));
}

#[test]
fn remove_ancestor_keeps_descendant_path_loadable() {
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

    let path = store
        .load_path_to_message(&conversation_id, &third_id)
        .unwrap();

    assert_eq!(
        message_ids(&path),
        vec![first_id.to_string(), third_id.to_string()]
    );
}

#[test]
fn remove_message_keeps_unrelated_branch() {
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

    store.remove_message(&conversation_id, &second_id).unwrap();

    let messages = store.load_messages(&conversation_id).unwrap();

    assert!(
        messages
            .iter()
            .any(|message| message.id.as_ref() == Some(&third_id))
    );
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
    let compaction = store.latest_compaction(&conversation_id).unwrap();

    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].id.as_deref(), Some(first_id.as_str()));
    assert_eq!(messages[1].id.as_deref(), Some(second_id.as_str()));
    assert_eq!(messages[0].content, "one");
    assert_eq!(messages[1].content, "two");
    assert!(compaction.is_none());
}

#[test]
fn truncate_clears_deleted_runtime_run_head() {
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
    let session_id = SessionId::new("run-truncate-head");
    store
        .create_session(
            &session_id,
            &conversation_id,
            Some(&third_id),
            "openai/test",
            None,
        )
        .unwrap();

    store
        .truncate_after_message(&conversation_id, &first_id)
        .unwrap();

    let run = store.load_session(&session_id).unwrap();

    assert_eq!(run.start_head_message_id, None);
    assert_eq!(run.current_head_message_id, None);
    // Truncating away a ready branch's head clears the dangling pointers but
    // leaves the branch ready, since it was never running.
    assert_eq!(run.status, SessionStatus::Ready);
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
    let fork_tool = ToolSchema {
        name: ToolSchemaName::new("fork_tool"),
        description: "Forked path tool".to_string(),
        parameters: serde_json::json!({"type":"object"}),
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
    store
        .set_system_prompt(&conversation_id, "fork prompt")
        .unwrap();
    store
        .insert_tool_schema(&conversation_id, &fork_tool)
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
    let forked_model = store.conversation_model(&forked_conversation_id).unwrap();
    let forked_reasoning_effort = store
        .conversation_reasoning_effort(&forked_conversation_id)
        .unwrap();
    let forked_system_prompt = store.system_prompt(&forked_conversation_id).unwrap();
    let forked_tool_schemas = store.load_tool_schemas(&forked_conversation_id).unwrap();

    assert_ne!(forked_conversation_id, conversation_id);
    assert_eq!(forked_model, "anthropic/test");
    assert_eq!(forked_reasoning_effort.as_deref(), Some("high"));
    assert_eq!(forked_system_prompt.as_deref(), Some("fork prompt"));
    assert_eq!(forked_tool_schemas, vec![fork_tool]);
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
fn queues_and_materializes_inputs_in_fifo_order() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();
    let root_id = store
        .insert_message(&conversation_id, None, Role::Assistant, "root", None)
        .unwrap();
    let session_id = SessionId::new("session-queue-fifo");
    store
        .create_session(
            &session_id,
            &conversation_id,
            Some(&root_id),
            "openai/test",
            None,
        )
        .unwrap();

    let first_input = store
        .enqueue_session_input(&session_id, "first queued", &[unsaved_text("first queued")])
        .unwrap();
    let second_input = store
        .enqueue_session_input(
            &session_id,
            "second queued",
            &[unsaved_text("second queued")],
        )
        .unwrap();
    assert_ne!(first_input, second_input);
    assert_eq!(store.session_input_count(&session_id).unwrap(), 2);

    let first = store
        .materialize_next_session_input(&session_id)
        .unwrap()
        .unwrap();
    let session = store.load_session(&session_id).unwrap();
    assert_eq!(first.id, first_input);
    assert_eq!(first.content, "first queued");
    assert_eq!(
        session
            .current_head_message_id
            .as_ref()
            .unwrap()
            .to_string(),
        store
            .load_messages(&conversation_id)
            .unwrap()
            .last()
            .unwrap()
            .id
            .as_ref()
            .unwrap()
            .to_string()
    );
    assert_eq!(store.session_input_count(&session_id).unwrap(), 1);

    let second = store
        .materialize_next_session_input(&session_id)
        .unwrap()
        .unwrap();
    assert_eq!(second.id, second_input);
    assert_eq!(second.content, "second queued");
    assert_eq!(store.session_input_count(&session_id).unwrap(), 0);
    let messages = store
        .load_path_to_message(
            &conversation_id,
            &store
                .load_session(&session_id)
                .unwrap()
                .current_head_message_id
                .unwrap(),
        )
        .unwrap();
    assert_eq!(messages[1].content, "first queued");
    assert_eq!(messages[2].content, "second queued");
}

#[test]
fn removes_terminal_session_events_and_exclusive_branch_messages() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();
    let root_id = store
        .insert_message(&conversation_id, None, Role::User, "root", None)
        .unwrap();
    let branch_id = store
        .insert_message(
            &conversation_id,
            Some(&root_id),
            Role::Assistant,
            "branch",
            None,
        )
        .unwrap();
    let session_id = SessionId::new("session-delete-exclusive");
    store
        .create_session(
            &session_id,
            &conversation_id,
            Some(&root_id),
            "openai/test",
            None,
        )
        .unwrap();
    store
        .update_session_head(&session_id, Some(&branch_id))
        .unwrap();
    store
        .update_session_status(&session_id, SessionStatus::Completed, None)
        .unwrap();
    store
        .append_session_event(
            &session_id,
            SessionEvent::Completed {
                message_id: Some(branch_id.to_string()),
            },
        )
        .unwrap();

    store.remove_session(&session_id).unwrap();

    assert!(store.load_session(&session_id).is_err());
    assert!(store.load_session_events_after(&session_id, None).is_err());
    assert_eq!(
        message_ids(&store.load_messages(&conversation_id).unwrap()),
        vec![root_id.to_string()]
    );
}

#[test]
fn removes_session_preserves_messages_needed_by_another_session() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();
    let root_id = store
        .insert_message(&conversation_id, None, Role::User, "root", None)
        .unwrap();
    let shared_id = store
        .insert_message(
            &conversation_id,
            Some(&root_id),
            Role::Assistant,
            "shared",
            None,
        )
        .unwrap();
    let deleted_id = store
        .insert_message(
            &conversation_id,
            Some(&shared_id),
            Role::User,
            "deleted branch",
            None,
        )
        .unwrap();
    let surviving_id = store
        .insert_message(
            &conversation_id,
            Some(&shared_id),
            Role::User,
            "surviving branch",
            None,
        )
        .unwrap();

    let deleted_session_id = SessionId::new("session-delete-branch");
    store
        .create_session(
            &deleted_session_id,
            &conversation_id,
            Some(&shared_id),
            "openai/test",
            None,
        )
        .unwrap();
    store
        .update_session_head(&deleted_session_id, Some(&deleted_id))
        .unwrap();
    store
        .update_session_status(&deleted_session_id, SessionStatus::Completed, None)
        .unwrap();

    let surviving_session_id = SessionId::new("session-delete-survivor");
    store
        .create_session(
            &surviving_session_id,
            &conversation_id,
            Some(&shared_id),
            "openai/test",
            None,
        )
        .unwrap();
    store
        .update_session_head(&surviving_session_id, Some(&surviving_id))
        .unwrap();

    store.remove_session(&deleted_session_id).unwrap();

    let ids = message_ids(&store.load_messages(&conversation_id).unwrap());
    assert!(!ids.contains(&deleted_id.to_string()));
    assert!(ids.contains(&shared_id.to_string()));
    assert!(ids.contains(&surviving_id.to_string()));
    assert!(store.load_session(&surviving_session_id).is_ok());
}

#[test]
fn refuses_to_remove_live_session() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();
    let session_id = SessionId::new("session-delete-live");
    store
        .create_session(&session_id, &conversation_id, None, "openai/test", None)
        .unwrap();
    store
        .update_session_status(&session_id, SessionStatus::Running, None)
        .unwrap();

    let error = store.remove_session(&session_id).unwrap_err();

    assert!(error.to_string().contains("cannot delete a running"));
    assert!(store.load_session(&session_id).is_ok());
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

#[test]
fn enumerates_root_to_leaf_paths() {
    let mut store = Store::open_memory().unwrap();
    let conversation_id = store.create_conversation("openai/test").unwrap();

    let root = store
        .insert_message(&conversation_id, None, Role::User, "root", None)
        .unwrap();
    let left = store
        .insert_message(&conversation_id, Some(&root), Role::User, "left", None)
        .unwrap();
    let right = store
        .insert_message(&conversation_id, Some(&root), Role::User, "right", None)
        .unwrap();
    let ll = store
        .insert_message(&conversation_id, Some(&left), Role::User, "ll", None)
        .unwrap();
    let lr = store
        .insert_message(&conversation_id, Some(&left), Role::User, "lr", None)
        .unwrap();
    let rl = store
        .insert_message(&conversation_id, Some(&right), Role::User, "rl", None)
        .unwrap();
    let lll = store
        .insert_message(&conversation_id, Some(&ll), Role::User, "lll", None)
        .unwrap();
    let llr = store
        .insert_message(&conversation_id, Some(&ll), Role::User, "llr", None)
        .unwrap();
    let lrl = store
        .insert_message(&conversation_id, Some(&lr), Role::User, "lrl", None)
        .unwrap();
    let lrr = store
        .insert_message(&conversation_id, Some(&lr), Role::User, "lrr", None)
        .unwrap();
    let rll = store
        .insert_message(&conversation_id, Some(&rl), Role::User, "rll", None)
        .unwrap();
    let rlr = store
        .insert_message(&conversation_id, Some(&rl), Role::User, "rlr", None)
        .unwrap();

    let paths = store.root_to_leaf_paths(&conversation_id).unwrap();

    let path_keys: Vec<Vec<String>> = paths
        .iter()
        .map(|path| path.iter().map(|id| id.as_str().to_string()).collect())
        .collect();
    let expected: Vec<Vec<String>> = vec![
        vec![
            root.as_str().to_string(),
            left.as_str().to_string(),
            ll.as_str().to_string(),
            lll.as_str().to_string(),
        ],
        vec![
            root.as_str().to_string(),
            left.as_str().to_string(),
            ll.as_str().to_string(),
            llr.as_str().to_string(),
        ],
        vec![
            root.as_str().to_string(),
            left.as_str().to_string(),
            lr.as_str().to_string(),
            lrl.as_str().to_string(),
        ],
        vec![
            root.as_str().to_string(),
            left.as_str().to_string(),
            lr.as_str().to_string(),
            lrr.as_str().to_string(),
        ],
        vec![
            root.as_str().to_string(),
            right.as_str().to_string(),
            rl.as_str().to_string(),
            rll.as_str().to_string(),
        ],
        vec![
            root.as_str().to_string(),
            right.as_str().to_string(),
            rl.as_str().to_string(),
            rlr.as_str().to_string(),
        ],
    ];
    let mut actual_sorted = path_keys.clone();
    let mut expected_sorted = expected.clone();
    actual_sorted.sort_by_key(|a| a.join("/"));
    expected_sorted.sort_by_key(|a| a.join("/"));

    assert_eq!(actual_sorted, expected_sorted);

    for path in &path_keys {
        assert_eq!(path.first().unwrap(), root.as_str());
    }

    let isolated = store.create_conversation("openai/test").unwrap();
    store
        .insert_message(&isolated, None, Role::User, "only", None)
        .unwrap();
    let only_paths = store.root_to_leaf_paths(&isolated).unwrap();
    assert_eq!(only_paths.len(), 1);
    assert_eq!(only_paths[0].len(), 1);

    let empty = store.create_conversation("openai/test").unwrap();
    assert!(store.root_to_leaf_paths(&empty).unwrap().is_empty());
}
