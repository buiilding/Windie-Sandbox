//! Terminal adapters for direct conversation message mutations.

use anyhow::Result;

use crate::cli::InsertPart;
use crate::conversation::{ConversationId, MessageId, Role};
use crate::operation::{self, MessageInputPart};
use crate::output::TerminalOutput;
use crate::store::Store;

/// Inserts one explicit message into a conversation.
pub(crate) fn insert_message(
    conversation_id: ConversationId,
    head_message_id: Option<MessageId>,
    role: Role,
    parts: &[InsertPart],
) -> Result<()> {
    let mut store = Store::open()?;
    let input_parts = message_input_parts(parts);
    let message_id = operation::insert_message(
        &mut store,
        &conversation_id,
        head_message_id.as_ref(),
        role,
        &input_parts,
    )?;

    TerminalOutput.inserted_message(&message_id);
    Ok(())
}

/// Converts parsed CLI insert parts into the shared operation input shape.
fn message_input_parts(parts: &[InsertPart]) -> Vec<MessageInputPart> {
    parts
        .iter()
        .map(|part| match part {
            InsertPart::Text(text) => MessageInputPart::Text(text.clone()),
            InsertPart::Image(path) => MessageInputPart::ImagePath(path.clone()),
        })
        .collect()
}

/// Replaces one message's text without querying the model.
pub(crate) fn update_message(
    conversation_id: ConversationId,
    message_id: MessageId,
    text: &str,
) -> Result<()> {
    let mut store = Store::open()?;
    operation::update_message(&mut store, &conversation_id, &message_id, text)?;

    TerminalOutput.updated_message(&message_id);
    Ok(())
}

/// Sets or replaces the root-scoped system prompt.
pub(crate) fn set_system_prompt(conversation_id: ConversationId, text: &str) -> Result<()> {
    let mut store = Store::open()?;
    operation::set_system_prompt(&mut store, &conversation_id, text)?;

    TerminalOutput.set_system_prompt(&conversation_id);
    Ok(())
}

/// Clears the root-scoped system prompt.
pub(crate) fn remove_system_prompt(conversation_id: ConversationId) -> Result<()> {
    let mut store = Store::open()?;
    operation::remove_system_prompt(&mut store, &conversation_id)?;

    TerminalOutput.removed_system_prompt(&conversation_id);
    Ok(())
}

/// Deletes one message while preserving the remaining conversation chain.
pub(crate) fn remove_message(conversation_id: ConversationId, message_id: MessageId) -> Result<()> {
    let mut store = Store::open()?;
    operation::remove_message(&mut store, &conversation_id, &message_id)?;

    TerminalOutput.removed_message(&message_id);
    Ok(())
}

/// Prunes descendant messages after a checkpoint message inside one conversation.
pub(crate) fn truncate_conversation(
    conversation_id: ConversationId,
    message_id: MessageId,
) -> Result<()> {
    let mut store = Store::open()?;
    operation::truncate_conversation(&mut store, &conversation_id, &message_id)?;

    TerminalOutput.truncated_conversation(&conversation_id, &message_id);
    Ok(())
}
