//! Read-only operation snapshots for CLI JSON, API, and developer inspection.

use super::*;

/// Full durable message tree.
pub struct ConversationTree {
    pub messages: Vec<Message>,
}

#[derive(Debug, Serialize)]
/// Machine-readable snapshot of one conversation's current sessiontime state.
///
/// CLI JSON and API inspection both serialize this same operation-level shape.
/// It deliberately summarizes image bytes instead of exposing raw image data.
pub struct InspectionReport {
    conversation_id: String,
    head_message_id: Option<String>,
    model: String,
    reasoning: Option<ReasoningRequest>,
    system_prompt: Option<String>,
    tool_approval_mode: ToolApprovalMode,
    tool_schemas: Vec<ToolSchema>,
    messages: Vec<InspectionMessage>,
    path: Vec<InspectionMessage>,
    model_context: Vec<InspectionMessage>,
    latest_compaction: Option<InspectionCompaction>,
}

impl InspectionReport {
    /// Builds the serializable inspection report from loaded runtime data.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        conversation_id: &ConversationId,
        head_message_id: Option<&MessageId>,
        model: &str,
        reasoning: Option<ReasoningRequest>,
        system_prompt: Option<String>,
        tool_approval_mode: ToolApprovalMode,
        tool_schemas: Vec<ToolSchema>,
        messages: Vec<Message>,
        path: Vec<Message>,
        model_context: Vec<Message>,
        latest_compaction: Option<Compaction>,
    ) -> Self {
        Self {
            conversation_id: conversation_id.as_str().to_string(),
            head_message_id: head_message_id.map(|id| id.as_str().to_string()),
            model: model.to_string(),
            reasoning,
            system_prompt,
            tool_approval_mode,
            tool_schemas,
            messages: inspection_messages(messages),
            path: inspection_messages(path),
            model_context: inspection_messages(model_context),
            latest_compaction: latest_compaction.map(InspectionCompaction::from_compaction),
        }
    }
}

#[derive(Debug, Serialize)]
/// Serializable message shape for inspection JSON.
struct InspectionMessage {
    id: Option<String>,
    parent_message_id: Option<String>,
    role: Role,
    content: String,
    parts: Vec<InspectionMessagePart>,
    metadata: Option<MessageMetadata>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
/// Serializable message part that never includes raw image bytes.
enum InspectionMessagePart {
    Text {
        text: String,
    },
    Image {
        asset_id: String,
        mime_type: String,
        byte_count: usize,
    },
}

#[derive(Debug, Serialize)]
/// Serializable compaction checkpoint shape for inspection JSON.
struct InspectionCompaction {
    id: String,
    conversation_id: String,
    through_message_id: String,
    content: String,
    created_at: i64,
}

impl InspectionCompaction {
    /// Converts a store compaction row into the public inspection shape.
    fn from_compaction(compaction: Compaction) -> Self {
        Self {
            id: compaction.id.as_str().to_string(),
            conversation_id: compaction.conversation_id.as_str().to_string(),
            through_message_id: compaction.through_message_id.as_str().to_string(),
            content: compaction.content,
            created_at: compaction.created_at,
        }
    }
}

/// Converts loaded runtime messages into the public inspection message shape.
fn inspection_messages(messages: Vec<Message>) -> Vec<InspectionMessage> {
    messages
        .into_iter()
        .map(|message| InspectionMessage {
            id: message.id.map(|id| id.as_str().to_string()),
            parent_message_id: message.parent_message_id.map(|id| id.as_str().to_string()),
            role: message.role,
            content: message.content,
            parts: inspection_message_parts(message.parts),
            metadata: message.metadata,
        })
        .collect()
}

/// Converts typed message parts while preserving order and hiding image bytes.
fn inspection_message_parts(parts: Vec<MessagePart>) -> Vec<InspectionMessagePart> {
    parts
        .into_iter()
        .map(|part| match part {
            MessagePart::Text(text) => InspectionMessagePart::Text { text },
            MessagePart::Image(image) => InspectionMessagePart::Image {
                asset_id: image.asset_id.as_str().to_string(),
                mime_type: image.mime_type,
                byte_count: image.bytes.len(),
            },
        })
        .collect()
}

pub fn conversation_tree(
    store: &Store,
    conversation_id: &ConversationId,
) -> Result<ConversationTree> {
    let messages = store.load_message_tree(conversation_id)?;

    Ok(ConversationTree { messages })
}

/// Loads the shared read-only inspection snapshot used by CLI JSON and API.
pub fn inspect_conversation(
    store: &Store,
    conversation_id: &ConversationId,
    head_message_id: Option<&MessageId>,
    model_override: Option<ModelName>,
) -> Result<InspectionReport> {
    let model = resolve_conversation_model(store, conversation_id, model_override)?;
    let reasoning = conversation_reasoning(store, conversation_id)?;
    let tool_approval_mode = store.tool_approval_mode(conversation_id)?;
    let messages = store.load_message_tree(conversation_id)?;
    let tool_schemas = store.load_tool_schemas_for_head(conversation_id, head_message_id)?;
    let path = match head_message_id {
        Some(message_id) => store.load_path_to_message(conversation_id, message_id)?,
        None => Vec::new(),
    };
    let model_context = ContextBuilder::build_messages(store, conversation_id, head_message_id)?;
    let system_prompt = store.effective_system_prompt_for_head(conversation_id, head_message_id)?;
    let latest_compaction = store.latest_compaction(conversation_id)?;

    Ok(InspectionReport::new(
        conversation_id,
        head_message_id,
        model.as_str(),
        reasoning,
        system_prompt,
        tool_approval_mode,
        tool_schemas,
        messages,
        path,
        model_context,
        latest_compaction,
    ))
}
