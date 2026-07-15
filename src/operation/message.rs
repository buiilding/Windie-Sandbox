//! Message and system-prompt mutation operation workflows.

use super::input::{LoadedInsertPart, insert_content, load_insert_part, validate_insert_parts};
use super::*;

pub fn insert_message(
    store: &mut Store,
    conversation_id: &ConversationId,
    parent_message_id: Option<&MessageId>,
    role: Role,
    parts: &[MessageInputPart],
) -> Result<MessageId> {
    validate_insert_parts(parts)?;

    if role == Role::Tool {
        return Err(error::invalid_request(
            "role: tool messages must be created through approve or deny",
        ));
    }

    let has_image = parts.iter().any(|part| {
        matches!(
            part,
            MessageInputPart::ImagePath(_) | MessageInputPart::ImageBytes { .. }
        )
    });
    let has_multiple_parts = parts.len() > 1;
    let content = insert_content(parts);

    if has_image || has_multiple_parts {
        if role != Role::User {
            return Err(error::invalid_request(
                "multi-part input is only supported for user messages",
            ));
        }

        let loaded_parts = parts
            .iter()
            .map(load_insert_part)
            .collect::<Result<Vec<_>>>()?;
        let message_parts = loaded_parts
            .iter()
            .map(|part| match part {
                LoadedInsertPart::Text(text) => UnsavedMessagePart::Text(text.clone()),
                LoadedInsertPart::Image(image) => UnsavedMessagePart::Image(UnsavedImagePart {
                    mime_type: image.mime_type.clone(),
                    bytes: image.bytes.clone(),
                }),
            })
            .collect::<Vec<_>>();

        return store.insert_message_with_parts(
            conversation_id,
            parent_message_id,
            Role::User,
            &content,
            &message_parts,
            None,
        );
    }

    store.insert_message(conversation_id, parent_message_id, role, &content, None)
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

/// Inserts a system prompt message at the active conversation path.
pub fn set_system_prompt(
    store: &mut Store,
    conversation_id: &ConversationId,
    text: &str,
) -> Result<MessageId> {
    store.set_system_prompt(conversation_id, text)
}

/// Inserts a system prompt message at an explicit conversation path head.
pub fn set_system_prompt_at_head(
    store: &mut Store,
    conversation_id: &ConversationId,
    head_message_id: Option<&MessageId>,
    text: &str,
) -> Result<MessageId> {
    store.set_system_prompt_at_head(conversation_id, head_message_id, text)
}

/// Removes the system prompt at the active conversation path.
pub fn remove_system_prompt(
    store: &mut Store,
    conversation_id: &ConversationId,
) -> Result<MessageId> {
    store.remove_system_prompt(conversation_id)
}

/// Removes the system prompt at an explicit conversation path head.
pub fn remove_system_prompt_at_head(
    store: &mut Store,
    conversation_id: &ConversationId,
    head_message_id: Option<&MessageId>,
) -> Result<MessageId> {
    store.remove_system_prompt_at_head(conversation_id, head_message_id)
}

/// Sets the conversation default for attached tool approval.

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
