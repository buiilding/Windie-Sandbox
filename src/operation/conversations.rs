//! Conversation inspection and mutation operations.

use super::{
    ContextBuilder, ConversationId, ConversationInfo, ConversationTree, ImageInput,
    InspectionReport, Message, MessageId, MessageInputPart, ModelName, ReasoningRequest, Result,
    Role, Store, UnsavedImagePart, UnsavedMessagePart, error, read_image_input,
    validate_image_input_bytes,
};

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

/// Returns whether the configured local Bifrost gateway is running.
pub fn active_path(store: &Store, conversation_id: &ConversationId) -> Result<Vec<Message>> {
    store.load_active_path(conversation_id)
}

/// Loads the full tree and active message pointer for inspection.
pub fn conversation_tree(
    store: &Store,
    conversation_id: &ConversationId,
) -> Result<ConversationTree> {
    let messages = store.load_message_tree_view(conversation_id)?;
    let active_message_id = store.active_message_id(conversation_id)?;

    Ok(ConversationTree {
        messages,
        active_message_id,
    })
}

/// Loads the shared read-only inspection snapshot used by CLI JSON and API.
pub fn inspect_conversation(
    store: &Store,
    conversation_id: &ConversationId,
    model_override: Option<ModelName>,
) -> Result<InspectionReport> {
    let model = resolve_conversation_model(store, conversation_id, model_override)?;
    let reasoning = conversation_reasoning(store, conversation_id)?;
    let active_message_id = store.active_message_id(conversation_id)?;
    let tool_approval_mode = store.tool_approval_mode(conversation_id)?;
    let messages = store.load_message_tree_view(conversation_id)?;
    let tool_schemas = store.load_tool_schemas(conversation_id)?;
    let active_path = store.load_active_path_view(conversation_id)?;
    let system_prompt = store.system_prompt(conversation_id)?;
    let latest_compaction = store.latest_compaction(conversation_id)?;
    let execution_claims = store.tool_execution_records(conversation_id)?;
    let model_context = ContextBuilder::flatten_view(
        active_path.clone(),
        system_prompt.clone(),
        latest_compaction.as_ref(),
    );

    Ok(InspectionReport::new(
        conversation_id,
        active_message_id.as_ref(),
        model.as_str(),
        reasoning,
        system_prompt,
        tool_approval_mode,
        tool_schemas,
        messages,
        active_path,
        model_context,
        latest_compaction,
        execution_claims,
    ))
}

/// Inserts one message below the current active message.
pub fn insert_message(
    store: &mut Store,
    conversation_id: &ConversationId,
    role: Role,
    parts: &[MessageInputPart],
) -> Result<MessageId> {
    validate_insert_parts(parts)?;

    if role == Role::Tool {
        return Err(error::invalid_request(
            "role: tool messages must be created through approve or deny",
        ));
    }

    let parent_message_id = store.active_message_id(conversation_id)?;
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
            parent_message_id.as_ref(),
            Role::User,
            &content,
            &message_parts,
            None,
        );
    }

    store.insert_message(
        conversation_id,
        parent_message_id.as_ref(),
        role,
        &content,
        None,
    )
}

/// Selects one message as the active runtime node.
pub fn activate_message(
    store: &mut Store,
    conversation_id: &ConversationId,
    message_id: &MessageId,
) -> Result<()> {
    store.set_active_message(conversation_id, message_id)
}

/// Replaces visible message text without changing metadata.
pub fn update_message(
    store: &mut Store,
    conversation_id: &ConversationId,
    message_id: &MessageId,
    text: &str,
) -> Result<()> {
    ensure_conversation_has_no_active_run(store, conversation_id)?;
    store.replace_message(conversation_id, message_id, text)
}

/// Sets or replaces the conversation-level system prompt.
pub fn set_system_prompt(
    store: &mut Store,
    conversation_id: &ConversationId,
    text: &str,
) -> Result<()> {
    store.set_system_prompt(conversation_id, text)
}

/// Removes the conversation-level system prompt.
pub fn remove_system_prompt(store: &mut Store, conversation_id: &ConversationId) -> Result<()> {
    store.remove_system_prompt(conversation_id)
}

/// Sets the conversation default for attached tool approval.
pub fn remove_conversation(store: &mut Store, conversation_id: &ConversationId) -> Result<()> {
    ensure_conversation_has_no_active_run(store, conversation_id)?;
    store.remove_conversation(conversation_id)
}

/// Removes one message according to the store's current tree-removal policy.
pub fn remove_message(
    store: &mut Store,
    conversation_id: &ConversationId,
    message_id: &MessageId,
) -> Result<()> {
    ensure_conversation_has_no_active_run(store, conversation_id)?;
    store.remove_message(conversation_id, message_id)
}

/// Prunes descendant messages after one checkpoint message.
pub fn truncate_conversation(
    store: &mut Store,
    conversation_id: &ConversationId,
    message_id: &MessageId,
) -> Result<()> {
    ensure_conversation_has_no_active_run(store, conversation_id)?;
    store.truncate_after_message(conversation_id, message_id)
}

fn ensure_conversation_has_no_active_run(
    store: &Store,
    conversation_id: &ConversationId,
) -> Result<()> {
    if let Some(run) = store.active_runtime_run(conversation_id)? {
        return Err(error::invalid_request(format!(
            "conversation has a running action: {}",
            run.id
        )));
    }
    Ok(())
}

/// Copies a conversation through one checkpoint into a new conversation.
pub fn fork_conversation(
    store: &mut Store,
    conversation_id: &ConversationId,
    message_id: &MessageId,
) -> Result<ConversationId> {
    store.fork_conversation_at_message(conversation_id, message_id)
}

enum LoadedInsertPart {
    Text(String),
    Image(ImageInput),
}

/// Reads image parts through the image input boundary.
fn load_insert_part(part: &MessageInputPart) -> Result<LoadedInsertPart> {
    match part {
        MessageInputPart::Text(text) => Ok(LoadedInsertPart::Text(text.clone())),
        MessageInputPart::ImagePath(path) => read_image_input(path).map(LoadedInsertPart::Image),
        MessageInputPart::ImageBytes { mime_type, bytes } => {
            validate_image_input_bytes(mime_type, bytes)?;
            Ok(LoadedInsertPart::Image(ImageInput {
                mime_type: mime_type.clone(),
                bytes: bytes.clone(),
            }))
        }
    }
}

/// Validates that an insert carries at least one meaningful user input.
fn validate_insert_parts(parts: &[MessageInputPart]) -> Result<()> {
    if parts.is_empty() {
        return Err(error::invalid_request("message requires text or parts"));
    }
    if parts.iter().all(empty_text_part) {
        return Err(error::invalid_request(
            "message requires non-empty text or an image",
        ));
    }

    Ok(())
}

/// Returns whether a part contributes no content.
fn empty_text_part(part: &MessageInputPart) -> bool {
    match part {
        MessageInputPart::Text(text) => text.is_empty(),
        MessageInputPart::ImagePath(_) | MessageInputPart::ImageBytes { .. } => false,
    }
}

/// Builds the plain text preview stored in the message row.
fn insert_content(parts: &[MessageInputPart]) -> String {
    parts
        .iter()
        .filter_map(|part| match part {
            MessageInputPart::Text(text) => Some(text.as_str()),
            MessageInputPart::ImagePath(_) | MessageInputPart::ImageBytes { .. } => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}
