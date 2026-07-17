//! Read-only operation snapshots for CLI JSON, API, and developer inspection.

use super::*;

/// Maximum preview characters exposed per path leaf in the inspection JSON.
const PATH_LEAF_PREVIEW_MAX_CHARS: usize = 80;

/// Full durable message tree.
pub struct ConversationTree {
    pub messages: Vec<Message>,
}

/// One root-to-leaf path through the conversation tree.
#[derive(Debug, Serialize)]
pub struct InspectionPath {
    message_ids: Vec<String>,
    leaf_message_id: String,
    depth: usize,
    leaf_preview: String,
}

impl InspectionPath {
    fn from_root_to_leaf(path: &[MessageId], leaf: &Message) -> Self {
        let message_ids = path
            .iter()
            .map(|id| id.as_str().to_string())
            .collect::<Vec<_>>();
        let leaf_message_id = message_ids.last().cloned().unwrap_or_default();
        let depth = message_ids.len().saturating_sub(1);
        let leaf_preview = preview_for_message(leaf);

        Self {
            message_ids,
            leaf_message_id,
            depth,
            leaf_preview,
        }
    }
}

fn preview_for_message(message: &Message) -> String {
    let raw = message
        .parts
        .iter()
        .find_map(|part| match part {
            MessagePart::Text(text) => Some(text.as_str()),
            MessagePart::Image(_) => None,
        })
        .unwrap_or(message.content.as_str());
    raw.chars().take(PATH_LEAF_PREVIEW_MAX_CHARS).collect()
}

#[derive(Debug, Serialize)]
/// Machine-readable snapshot of one conversation's current sessiontime state.
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
    paths: Vec<InspectionPath>,
    latest_compaction: Option<InspectionCompaction>,
}

impl InspectionReport {
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
        paths: Vec<InspectionPath>,
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
            paths,
            latest_compaction: latest_compaction.map(InspectionCompaction::from_compaction),
        }
    }
}

#[derive(Debug, Serialize)]
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
enum InspectionMessagePart {
    Text { text: String },
    Image {
        asset_id: String,
        mime_type: String,
        byte_count: usize,
    },
}

#[derive(Debug, Serialize)]
struct InspectionCompaction {
    id: String,
    conversation_id: String,
    through_message_id: String,
    content: String,
    created_at: i64,
}

impl InspectionCompaction {
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
/// Tree-wide: system prompt and tool schemas are conversation-wide, same for any head.
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
    let tool_schemas = store.load_tool_schemas(conversation_id)?;
    let path = match head_message_id {
        Some(message_id) => store.load_path_to_message(conversation_id, message_id)?,
        None => Vec::new(),
    };
    let model_context = ContextBuilder::build_messages(store, conversation_id, head_message_id)?;
    let system_prompt = store.system_prompt(conversation_id)?;
    let latest_compaction = store.latest_compaction(conversation_id)?;
    let paths = build_inspection_paths(store, conversation_id, &messages)?;

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
        paths,
        latest_compaction,
    ))
}

fn build_inspection_paths(
    store: &Store,
    conversation_id: &ConversationId,
    messages: &[Message],
) -> Result<Vec<InspectionPath>> {
    let raw_paths = store.root_to_leaf_paths(conversation_id)?;
    if raw_paths.is_empty() {
        return Ok(Vec::new());
    }

    let mut by_id: HashMap<&str, &Message> = HashMap::with_capacity(messages.len());
    for message in messages {
        if let Some(id) = message.id.as_ref() {
            by_id.insert(id.as_str(), message);
        }
    }

    let mut inspection_paths = Vec::with_capacity(raw_paths.len());
    for raw_path in raw_paths {
        let Some(leaf_id) = raw_path.last() else {
            continue;
        };
        let Some(leaf_message) = by_id.get(leaf_id.as_str()) else {
            continue;
        };
        inspection_paths.push(InspectionPath::from_root_to_leaf(&raw_path, leaf_message));
    }

    Ok(inspection_paths)
}
