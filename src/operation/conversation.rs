//! Conversation-level operation workflows.

use super::*;

/// Creates an empty persisted conversation with its default model.
pub fn create_conversation(store: &Store, model: &ModelName) -> Result<ConversationId> {
    store.create_conversation(model.as_str())
}

/// Loads the persisted model for one conversation.
pub fn conversation_model(store: &Store, conversation_id: &ConversationId) -> Result<ModelName> {
    Ok(ModelName::new(store.conversation_model(conversation_id)?))
}

/// Loads the conversation-level reasoning request, if one is persisted.
pub fn conversation_reasoning(
    store: &Store,
    conversation_id: &ConversationId,
) -> Result<Option<ReasoningRequest>> {
    Ok(store
        .conversation_reasoning_effort(conversation_id)?
        .map(|effort| ReasoningRequest {
            effort: Some(effort),
            summary: None,
        }))
}

/// Sets the persisted model for future conversation turns.
pub fn set_conversation_model(
    store: &mut Store,
    conversation_id: &ConversationId,
    model: &ModelName,
) -> Result<()> {
    store.set_conversation_model(conversation_id, model.as_str())
}

/// Sets the conversation-level reasoning effort used by future turns.
pub fn set_conversation_reasoning_effort(
    store: &mut Store,
    conversation_id: &ConversationId,
    effort: Option<&str>,
) -> Result<Option<ReasoningRequest>> {
    store.set_conversation_reasoning_effort(conversation_id, effort)?;
    conversation_reasoning(store, conversation_id)
}

/// Resolves the model for a runtime operation.
///
/// A caller-supplied model is a one-request override. Without one, Windie uses
/// the conversation's persisted model.
pub fn resolve_conversation_model(
    store: &Store,
    conversation_id: &ConversationId,
    model_override: Option<ModelName>,
) -> Result<ModelName> {
    match model_override {
        Some(model) => Ok(model),
        None => conversation_model(store, conversation_id),
    }
}

/// Lists persisted conversations without loading message bodies.
pub fn list_conversations(store: &Store) -> Result<Vec<ConversationInfo>> {
    store.list_conversations()
}

pub fn set_tool_approval_mode(
    store: &mut Store,
    conversation_id: &ConversationId,
    mode: ToolApprovalMode,
) -> Result<()> {
    store.set_tool_approval_mode(conversation_id, mode)
}

pub fn remove_conversation(store: &mut Store, conversation_id: &ConversationId) -> Result<()> {
    store.remove_conversation(conversation_id)
}

pub fn fork_conversation(
    store: &mut Store,
    conversation_id: &ConversationId,
    message_id: &MessageId,
) -> Result<ConversationId> {
    store.fork_conversation_at_message(conversation_id, message_id)
}
