//! Shared CLI/API operation layer.
//!
//! This module owns the orchestration that should be identical across clients:
//! loading inspection snapshots, inserting messages, mutating conversation
//! state, and resolving explicit tool approvals. CLI and API code translate
//! inputs into these typed operations and translate returned values into their
//! own output formats.

use std::path::PathBuf;

use anyhow::Result;

use crate::context::{ContextBuilder, ContextParts};
use serde::Serialize;

use crate::conversation::{
    ConversationId, Message, MessageId, MessageMetadata, MessagePart, Role, ToolCallId, ToolSchema,
    ToolSchemaName, UnsavedImagePart, UnsavedMessagePart,
};
use crate::error;
use crate::gateway::{BifrostGateway, GatewayStart, GatewayStop, GatewayUrl};
use crate::image_input::{ImageInput, read_image_input, validate_image_input_bytes};
use crate::llm::{self, BaseUrl, BifrostClient, ModelInfo, ModelName, RuntimeLlm};
use crate::output::RuntimeOutput;
use crate::runtime::{
    approve_tool_call, deny_tool_call, pending_tool_approvals,
    query_conversation_once as runtime_query_conversation_once,
};
use crate::store::{Compaction, ConversationInfo, Store};
use crate::tool::{
    ProviderToolName, ToolApprovalRequest, ToolDefinition, ToolExecutionResult, ToolProviderId,
};
use crate::tool_provider::ToolProviderRegistry;

/// One ordered message part accepted by client-facing insert operations.
///
/// Text parts are stored directly. Path images are read through `image_input.rs`;
/// byte images arrive from local clients such as clipboard paste. Both image
/// forms are validated before storage copies bytes into SQLite.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageInputPart {
    Text(String),
    ImagePath(PathBuf),
    ImageBytes { mime_type: String, bytes: Vec<u8> },
}

/// Full message tree plus the selected active node.
pub struct ConversationTree {
    pub messages: Vec<Message>,
    pub active_message_id: Option<MessageId>,
}

#[derive(Debug, Serialize)]
/// Machine-readable snapshot of one conversation's current runtime state.
///
/// CLI JSON and API inspection both serialize this same operation-level shape.
/// It deliberately summarizes image bytes instead of exposing raw image data.
pub struct InspectionReport {
    conversation_id: String,
    active_message_id: Option<String>,
    model: String,
    system_prompt: Option<String>,
    tool_schemas: Vec<ToolSchema>,
    messages: Vec<InspectionMessage>,
    active_path: Vec<InspectionMessage>,
    model_context: Vec<InspectionMessage>,
    latest_compaction: Option<InspectionCompaction>,
}

impl InspectionReport {
    /// Builds the serializable inspection report from loaded runtime data.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        conversation_id: &ConversationId,
        active_message_id: Option<&MessageId>,
        model: &str,
        system_prompt: Option<String>,
        tool_schemas: Vec<ToolSchema>,
        messages: Vec<Message>,
        active_path: Vec<Message>,
        model_context: Vec<Message>,
        latest_compaction: Option<Compaction>,
    ) -> Self {
        Self {
            conversation_id: conversation_id.as_str().to_string(),
            active_message_id: active_message_id.map(|id| id.as_str().to_string()),
            model: model.to_string(),
            system_prompt,
            tool_schemas,
            messages: inspection_messages(messages),
            active_path: inspection_messages(active_path),
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

/// Creates an empty persisted conversation.
pub fn create_conversation(store: &Store) -> Result<ConversationId> {
    store.create_conversation()
}

/// Lists persisted conversations without loading message bodies.
pub fn list_conversations(store: &Store) -> Result<Vec<ConversationInfo>> {
    store.list_conversations()
}

/// Returns whether the configured local Bifrost gateway is running.
pub async fn gateway_status(gateway_url: GatewayUrl) -> bool {
    BifrostGateway::new(gateway_url).is_running().await
}

/// Starts the configured local Bifrost gateway if it is not already running.
pub async fn start_gateway(gateway_url: GatewayUrl) -> Result<GatewayStart> {
    BifrostGateway::new(gateway_url).start().await
}

/// Stops the configured local Bifrost gateway when Windie can identify it.
pub async fn stop_gateway(gateway_url: GatewayUrl) -> Result<GatewayStop> {
    BifrostGateway::new(gateway_url).stop().await
}

/// Requires the configured local Bifrost gateway to be reachable.
pub async fn require_gateway_running(gateway_url: GatewayUrl) -> Result<()> {
    BifrostGateway::new(gateway_url).require_running().await
}

/// Lists models from the currently running Bifrost gateway.
///
/// This operation is intentionally read-only. It does not start, stop, restart,
/// or reconfigure Bifrost; users restart the gateway explicitly after changing
/// `.env`.
pub async fn list_models(gateway_url: GatewayUrl, base_url: BaseUrl) -> Result<Vec<ModelInfo>> {
    require_gateway_running(gateway_url).await?;

    llm::list_models(base_url).await
}

/// Loads the active path shown by the CLI and inspector.
pub fn active_path(store: &Store, conversation_id: &ConversationId) -> Result<Vec<Message>> {
    store.load_active_path(conversation_id)
}

/// Loads the full tree and active message pointer for inspection.
pub fn conversation_tree(
    store: &Store,
    conversation_id: &ConversationId,
) -> Result<ConversationTree> {
    let messages = store.load_message_tree(conversation_id)?;
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
    model: &ModelName,
) -> Result<InspectionReport> {
    let active_message_id = store.active_message_id(conversation_id)?;
    let messages = store.load_message_tree(conversation_id)?;
    let tool_schemas = store.load_tool_schemas(conversation_id)?;
    let context_parts = ContextBuilder::load_parts(store, conversation_id)?;
    let model_context = ContextBuilder::flatten(ContextParts {
        active_path: context_parts.active_path.clone(),
        system_prompt: context_parts.system_prompt.clone(),
        compaction: context_parts.compaction.clone(),
    });

    Ok(InspectionReport::new(
        conversation_id,
        active_message_id.as_ref(),
        model.as_str(),
        context_parts.system_prompt,
        tool_schemas,
        messages,
        context_parts.active_path,
        model_context,
        context_parts.compaction,
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

/// Lists provider tools that can be attached to conversations.
pub fn available_tools() -> Result<Vec<ToolDefinition>> {
    ToolProviderRegistry::new().list_available_tools()
}

/// Lists provider tools for one provider ID.
pub fn available_provider_tools(provider_id: &ToolProviderId) -> Result<Vec<ToolDefinition>> {
    ToolProviderRegistry::new().list_provider_tools(provider_id)
}

/// Attaches one available provider tool to a conversation.
pub fn attach_tool(
    store: &mut Store,
    conversation_id: &ConversationId,
    provider_id: &ToolProviderId,
    tool_name: &ProviderToolName,
) -> Result<ToolSchemaName> {
    let registry = ToolProviderRegistry::new();
    let definition = registry.find_tool(provider_id, tool_name)?.ok_or_else(|| {
        error::not_found(format!("tool does not exist: {provider_id}/{tool_name}"))
    })?;
    let attached_tool = definition.attached_tool();
    let schema_name = attached_tool.schema_name.clone();

    store.insert_attached_tool(conversation_id, &attached_tool)?;

    Ok(schema_name)
}

/// Inserts one conversation-level tool schema.
pub fn insert_tool_schema(
    store: &mut Store,
    conversation_id: &ConversationId,
    tool_schema: &ToolSchema,
) -> Result<()> {
    store.insert_tool_schema(conversation_id, tool_schema)
}

/// Updates one conversation-level tool schema.
pub fn update_tool_schema(
    store: &mut Store,
    conversation_id: &ConversationId,
    current_name: &ToolSchemaName,
    tool_schema: &ToolSchema,
) -> Result<()> {
    store.update_tool_schema(conversation_id, current_name, tool_schema)
}

/// Removes one conversation-level tool schema.
pub fn remove_tool_schema(
    store: &mut Store,
    conversation_id: &ConversationId,
    name: &ToolSchemaName,
) -> Result<()> {
    store.remove_tool_schema(conversation_id, name)
}

/// Detaches one model-facing tool schema from a conversation.
pub fn detach_tool(
    store: &mut Store,
    conversation_id: &ConversationId,
    schema_name: &ToolSchemaName,
) -> Result<()> {
    remove_tool_schema(store, conversation_id, schema_name)
}

/// Deletes one conversation and all data owned by it.
pub fn remove_conversation(store: &mut Store, conversation_id: &ConversationId) -> Result<()> {
    store.remove_conversation(conversation_id)
}

/// Removes one message according to the store's current tree-removal policy.
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

/// Copies a conversation through one checkpoint into a new conversation.
pub fn fork_conversation(
    store: &mut Store,
    conversation_id: &ConversationId,
    message_id: &MessageId,
) -> Result<ConversationId> {
    store.fork_conversation_at_message(conversation_id, message_id)
}

/// Lists pending active-path tool calls that need user approval.
pub fn list_tool_approvals(
    store: &Store,
    conversation_id: &ConversationId,
) -> Result<Vec<ToolApprovalRequest>> {
    pending_tool_approvals(store, conversation_id)
}

/// Runs one explicit model query turn for a conversation.
///
/// This operation is the shared CLI/API entrypoint for one model request. The
/// runtime query primitive prepares the active path before provider context is
/// built. This operation does not loop through tool calls; callers compose
/// approval, denial, and later query turns as separate explicit operations.
pub async fn query_conversation_once<O, L>(
    output: &O,
    llm: &L,
    store: &mut Store,
    conversation_id: &ConversationId,
) -> Result<Message>
where
    O: RuntimeOutput,
    L: RuntimeLlm,
{
    runtime_query_conversation_once(output, llm, store, conversation_id).await
}

/// Runs the shared CLI/API query sequence for one model request.
///
/// Clients pass runtime settings in, but this operation owns the repeated
/// sequence: require the local gateway, construct the OpenAI-compatible Bifrost
/// client, then run one runtime query turn. Runtime still owns active-path
/// preparation and tool-result validation inside `query_conversation_once`.
pub async fn query_conversation<O>(
    output: &O,
    store: &mut Store,
    conversation_id: &ConversationId,
    gateway_url: GatewayUrl,
    base_url: BaseUrl,
    model: ModelName,
) -> Result<Message>
where
    O: RuntimeOutput,
{
    require_gateway_running(gateway_url).await?;
    let llm = BifrostClient::new(base_url, model);

    query_conversation_once(output, &llm, store, conversation_id).await
}

/// Executes one approved pending tool call and persists its result.
pub async fn approve_tool(
    store: &mut Store,
    conversation_id: &ConversationId,
    tool_call_id: &ToolCallId,
) -> Result<ToolExecutionResult> {
    approve_tool_call(store, conversation_id, tool_call_id).await
}

/// Persists a rejected result for one pending tool call.
pub fn deny_tool(
    store: &mut Store,
    conversation_id: &ConversationId,
    tool_call_id: &ToolCallId,
) -> Result<ToolExecutionResult> {
    deny_tool_call(store, conversation_id, tool_call_id)
}

/// Loaded version of one insert part.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::{MessageMetadata, ToolCall};
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn inserts_text_message() {
        let mut store = Store::open_memory().unwrap();
        let conversation_id = create_conversation(&store).unwrap();

        let message_id = insert_message(
            &mut store,
            &conversation_id,
            Role::User,
            &[MessageInputPart::Text("hello".to_string())],
        )
        .unwrap();

        let messages = active_path(&store, &conversation_id).unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].id.as_ref(), Some(&message_id));
        assert_eq!(messages[0].content, "hello");
    }

    #[test]
    fn rejects_direct_tool_message_insert() {
        let mut store = Store::open_memory().unwrap();
        let conversation_id = create_conversation(&store).unwrap();

        let error = insert_message(
            &mut store,
            &conversation_id,
            Role::Tool,
            &[MessageInputPart::Text("tool output".to_string())],
        )
        .unwrap_err();

        assert_eq!(
            error.to_string(),
            "role: tool messages must be created through approve or deny"
        );
    }

    #[test]
    fn inserts_multi_part_message() {
        let mut store = Store::open_memory().unwrap();
        let conversation_id = create_conversation(&store).unwrap();
        let image_path = temp_image_path("png");
        fs::write(
            &image_path,
            [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a],
        )
        .unwrap();

        insert_message(
            &mut store,
            &conversation_id,
            Role::User,
            &[
                MessageInputPart::Text("first".to_string()),
                MessageInputPart::ImagePath(image_path.clone()),
            ],
        )
        .unwrap();

        let messages = active_path(&store, &conversation_id).unwrap();
        assert_eq!(messages[0].content, "first");
        assert_eq!(messages[0].parts.len(), 2);
        fs::remove_file(image_path).unwrap();
    }

    #[test]
    fn inserts_loaded_image_bytes() {
        let mut store = Store::open_memory().unwrap();
        let conversation_id = create_conversation(&store).unwrap();

        insert_message(
            &mut store,
            &conversation_id,
            Role::User,
            &[
                MessageInputPart::Text("clipboard".to_string()),
                MessageInputPart::ImageBytes {
                    mime_type: "image/png".to_string(),
                    bytes: vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a],
                },
            ],
        )
        .unwrap();

        let messages = active_path(&store, &conversation_id).unwrap();
        assert_eq!(messages[0].content, "clipboard");
        assert_eq!(messages[0].parts.len(), 2);
    }

    #[test]
    fn inspection_snapshot_includes_runtime_state() {
        let mut store = Store::open_memory().unwrap();
        let conversation_id = create_conversation(&store).unwrap();
        set_system_prompt(&mut store, &conversation_id, "You are concise.").unwrap();
        let user_id = insert_message(
            &mut store,
            &conversation_id,
            Role::User,
            &[MessageInputPart::Text("hello".to_string())],
        )
        .unwrap();
        let tool_schema = ToolSchema {
            name: ToolSchemaName::new("run_shell"),
            description: "Run a shell command".to_string(),
            parameters: serde_json::json!({"type":"object"}),
        };
        insert_tool_schema(&mut store, &conversation_id, &tool_schema).unwrap();
        store
            .save_compaction(&conversation_id, &user_id, "hello happened")
            .unwrap();

        let report =
            inspect_conversation(&store, &conversation_id, &ModelName::new("openai/test")).unwrap();
        let value = serde_json::to_value(report).unwrap();

        assert_eq!(value["conversation_id"], conversation_id.as_str());
        assert_eq!(value["active_message_id"], user_id.as_str());
        assert_eq!(value["system_prompt"], "You are concise.");
        assert_eq!(value["tool_schemas"][0]["name"], "run_shell");
        assert_eq!(value["messages"][0]["id"], user_id.as_str());
        assert_eq!(value["active_path"][0]["id"], user_id.as_str());
        assert_eq!(value["model_context"][0]["role"], "system");
        assert_eq!(value["latest_compaction"]["content"], "hello happened");
    }

    #[test]
    fn attaches_available_provider_tool() {
        let mut store = Store::open_memory().unwrap();
        let conversation_id = create_conversation(&store).unwrap();
        let registry = ToolProviderRegistry::new();
        let run_shell = registry
            .find_tool(
                &ToolProviderId::new("windie"),
                &ProviderToolName::new("run_shell"),
            )
            .unwrap()
            .unwrap();

        let schema_name = attach_tool(
            &mut store,
            &conversation_id,
            &ToolProviderId::new("windie"),
            &ProviderToolName::new("run_shell"),
        )
        .unwrap();
        let attached_tools = store.load_attached_tools(&conversation_id).unwrap();

        assert_eq!(run_shell.schema_name, schema_name);
        assert_eq!(schema_name.as_str(), "run_shell");
        assert_eq!(attached_tools.len(), 1);
        assert_eq!(attached_tools[0].provider.provider_id.as_str(), "windie");
        assert_eq!(attached_tools[0].provider.tool_name.as_str(), "run_shell");
    }

    #[test]
    fn shared_operations_match_direct_store_state() {
        let mut store = Store::open_memory().unwrap();
        let conversation_id = create_conversation(&store).unwrap();
        let first_id = insert_message(
            &mut store,
            &conversation_id,
            Role::User,
            &[MessageInputPart::Text("first".to_string())],
        )
        .unwrap();
        let second_id = insert_message(
            &mut store,
            &conversation_id,
            Role::Assistant,
            &[MessageInputPart::Text("second".to_string())],
        )
        .unwrap();

        activate_message(&mut store, &conversation_id, &first_id).unwrap();
        update_message(&mut store, &conversation_id, &second_id, "second updated").unwrap();
        activate_message(&mut store, &conversation_id, &second_id).unwrap();

        let path = store.load_active_path(&conversation_id).unwrap();
        assert_eq!(path.len(), 2);
        assert_eq!(path[1].content, "second updated");
        assert_eq!(
            store.active_message_id(&conversation_id).unwrap().as_ref(),
            Some(&second_id)
        );
    }

    #[test]
    fn deny_tool_persists_tool_result() {
        let mut store = Store::open_memory().unwrap();
        let conversation_id = create_conversation(&store).unwrap();
        let user_id = store
            .insert_message(&conversation_id, None, Role::User, "run command", None)
            .unwrap();
        store
            .insert_message(
                &conversation_id,
                Some(&user_id),
                Role::Assistant,
                "",
                Some(&MessageMetadata {
                    tool_calls: vec![ToolCall::function(
                        "call_123",
                        "run_shell",
                        r#"{"command":"printf no"}"#,
                    )],
                    ..Default::default()
                }),
            )
            .unwrap();

        let result = deny_tool(&mut store, &conversation_id, &ToolCallId::new("call_123")).unwrap();
        let messages = store.load_active_path(&conversation_id).unwrap();

        assert!(!result.success);
        assert_eq!(messages.last().unwrap().role, Role::Tool);
        assert_eq!(
            messages
                .last()
                .unwrap()
                .metadata
                .as_ref()
                .and_then(|metadata| metadata.tool_call_id.as_ref())
                .map(ToolCallId::as_str),
            Some("call_123")
        );
    }

    #[tokio::test]
    async fn approve_tool_persists_tool_result() {
        let mut store = Store::open_memory().unwrap();
        let conversation_id = create_conversation(&store).unwrap();
        attach_tool(
            &mut store,
            &conversation_id,
            &ToolProviderId::new("windie"),
            &ProviderToolName::new("run_shell"),
        )
        .unwrap();
        let user_id = store
            .insert_message(&conversation_id, None, Role::User, "run command", None)
            .unwrap();
        store
            .insert_message(
                &conversation_id,
                Some(&user_id),
                Role::Assistant,
                "",
                Some(&MessageMetadata {
                    tool_calls: vec![ToolCall::function(
                        "call_approve",
                        "run_shell",
                        r#"{"command":"printf ok"}"#,
                    )],
                    ..Default::default()
                }),
            )
            .unwrap();

        let approvals = list_tool_approvals(&store, &conversation_id).unwrap();
        assert_eq!(approvals.len(), 1);

        let result = approve_tool(
            &mut store,
            &conversation_id,
            &ToolCallId::new("call_approve"),
        )
        .await
        .unwrap();
        let messages = store.load_active_path(&conversation_id).unwrap();

        assert!(result.success);
        assert!(result.content.contains("ok"));
        assert_eq!(messages.last().unwrap().role, Role::Tool);
        assert_eq!(
            messages
                .last()
                .unwrap()
                .metadata
                .as_ref()
                .and_then(|metadata| metadata.tool_call_id.as_ref())
                .map(ToolCallId::as_str),
            Some("call_approve")
        );
    }

    fn temp_image_path(extension: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let counter = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);

        std::env::temp_dir().join(format!(
            "windie-operation-{}-{nanos}-{counter}.{extension}",
            std::process::id()
        ))
    }
}
