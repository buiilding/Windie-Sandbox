//! Message and system-prompt mutation operation workflows.

use super::input::{insert_content, validate_insert_parts};
use super::*;

pub fn insert_message(
    store: &mut Store,
    conversation_id: &ConversationId,
    parent_message_id: Option<&MessageId>,
    role: Role,
    parts: &[MessageInputPart],
) -> Result<MessageId> {
    if role == Role::Tool {
        return Err(error::invalid_request(
            "role: tool messages must be created through approve or deny",
        ));
    }

    if role != Role::User {
        validate_insert_parts(parts)?;
        if parts.len() != 1 || !matches!(parts[0], MessageInputPart::Text(_)) {
            return Err(error::invalid_request(
                "multi-part input is only supported for user messages",
            ));
        }
        return store.insert_message(
            conversation_id,
            parent_message_id,
            role,
            &insert_content(parts),
            None,
        );
    }

    let prepared = prepare_message_input(parts)?;
    insert_prepared_user_message(store, conversation_id, parent_message_id, &prepared)
}

/// Inserts a previously validated and loaded user input.
pub fn insert_prepared_user_message(
    store: &mut Store,
    conversation_id: &ConversationId,
    parent_message_id: Option<&MessageId>,
    prepared: &PreparedMessageInput,
) -> Result<MessageId> {
    store.insert_message_with_parts(
        conversation_id,
        parent_message_id,
        Role::User,
        &prepared.content,
        &prepared.parts,
        None,
    )
}

/// Replaces visible message text without changing metadata.
pub fn update_message(
    store: &mut Store,
    conversation_id: &ConversationId,
    message_id: &MessageId,
    text: &str,
) -> Result<()> {
    store.replace_message(conversation_id, message_id, text)
}

/// Sets the conversation-wide system prompt (tree-wide, same for any head).
pub fn set_system_prompt(
    store: &mut Store,
    conversation_id: &ConversationId,
    text: &str,
) -> Result<()> {
    store.set_system_prompt(conversation_id, text)
}

/// Removes the conversation-wide system prompt (tree-wide).
pub fn remove_system_prompt(store: &mut Store, conversation_id: &ConversationId) -> Result<()> {
    store.remove_system_prompt(conversation_id)
}

/// Removes one message from the conversation tree.
pub fn remove_message(
    store: &mut Store,
    conversation_id: &ConversationId,
    message_id: &MessageId,
) -> Result<()> {
    store.remove_message(conversation_id, message_id)
}

/// Prunes descendant messages after one checkpoint message.
pub fn truncate_conversation(
    store: &mut Store,
    conversation_id: &ConversationId,
    message_id: &MessageId,
) -> Result<()> {
    store.truncate_after_message(conversation_id, message_id)
}
