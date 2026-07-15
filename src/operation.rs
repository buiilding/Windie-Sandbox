//! Shared CLI/API operation layer.
//!
//! This module owns the orchestration that should be identical across clients:
//! loading inspection snapshots, inserting messages, mutating conversation
//! state, and resolving explicit tool approvals. CLI and API code translate
//! inputs into these typed operations and translate returned values into their
//! own output formats.

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::Result;

use crate::context::ContextBuilder;
use serde::Serialize;

use crate::conversation::{
    ConversationId, Message, MessageId, MessageMetadata, MessagePart, Role, ToolCallId,
    UnsavedImagePart, UnsavedMessagePart,
};
use crate::error;
use crate::gateway::{BifrostGateway, GatewayStart, GatewayStop, GatewayUrl};
use crate::image_input::{ImageInput, read_image_input, validate_image_input_bytes};
use crate::llm::{
    self, BaseUrl, BifrostClient, InputTokenCount, ModelInfo, ModelName, ModelParameter,
    ModelParameterOption, PromptCacheRequest, ReasoningRequest,
};
use crate::output::{RuntimeOutput, TerminalOutput};
use crate::runtime::{
    PendingToolExecution, RuntimeEventSink, RuntimeInput, RuntimeModelRequest, RuntimeOutcome,
    advance_until_blocked as runtime_advance_until_blocked, deny_pending_tool_call,
    execute_pending_tool_call, load_pending_tool_call_at_head, pending_approvals_at_head,
    prepare_pending_tool_execution, store_pending_tool_result_at_head,
};
use crate::session::{Session, SessionEvent, SessionId, SessionStatus};
use crate::store::{Compaction, ConversationInfo, Store};
use crate::tool::{
    ProviderToolName, ToolApprovalMode, ToolApprovalRequest, ToolDefinition, ToolProviderId,
    ToolSchema, ToolSchemaName,
};
use crate::tool_provider::ToolProviderRegistry;
use crate::wakeup::{ContinueWakeup, Wakeup};

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

const SYNTHETIC_INPUT_TOKEN_COUNT_MESSAGE: &str = ".";

/// Full durable message tree.
pub struct ConversationTree {
    pub messages: Vec<Message>,
}

#[derive(Debug, Clone, Serialize)]
/// One session-owned pending approval surfaced to clients.
pub struct SessionToolApprovalRequest {
    pub session_id: SessionId,
    pub conversation_id: ConversationId,
    pub session_status: SessionStatus,
    pub head_message_id: Option<MessageId>,
    pub approval: ToolApprovalRequest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Action a session manager should take for a session-targeted wakeup.
pub enum SessionResumeAction {
    ApproveTool(ToolCallId),
    DenyTool(ToolCallId),
    Stop,
}

#[derive(Debug, Clone)]
/// Session and action resolved from a wakeup that targets an existing session.
pub struct SessionResume {
    pub session: Session,
    pub action: SessionResumeAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Source of a pre-query input-token count.
pub enum InputTokenCountSource {
    PrequeryInput,
    PrequerySyntheticInput,
}

impl InputTokenCountSource {
    /// Returns the stable API/UI label for this count source.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PrequeryInput => "prequery_input",
            Self::PrequerySyntheticInput => "prequery_synthetic_input",
        }
    }
}

/// Read-only model-facing payload pieces prepared for input-token counting.
///
/// Loading these pieces is separate from the async Bifrost request so API
/// handlers can release SQLite state before awaiting network I/O.
pub struct InputTokenCountContext {
    model_messages: Vec<Message>,
    tool_schemas: Vec<ToolSchema>,
    source: InputTokenCountSource,
}

impl InputTokenCountContext {
    /// Returns whether the count uses real context input or synthetic input.
    pub fn source(&self) -> InputTokenCountSource {
        self.source
    }
}

#[derive(Debug, Serialize)]
/// Normalized model-parameter metadata used by developer clients.
///
/// Bifrost returns a richer raw parameter schema. Windie extracts only the
/// effort selector needed for runtime query controls and preserves the raw
/// response for inspection/debugging.
pub struct ModelRuntimeParameters {
    model: String,
    supports_reasoning: bool,
    supports_prompt_caching: bool,
    reasoning: Option<ReasoningParameter>,
    raw: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
/// Effort selector derived from Bifrost model parameters.
pub struct ReasoningParameter {
    source: ReasoningParameterSource,
    options: Vec<ModelParameterOption>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
/// Bifrost parameter source used to build a normalized effort selector.
pub enum ReasoningParameterSource {
    ReasoningEffort,
    OutputConfigEffort,
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

/// Loads model-parameter metadata for one selected model.
///
/// This keeps Bifrost as the source of model capability truth. Windie only
/// normalizes Bifrost's effort parameter into the small shape the inspector
/// needs to render the reasoning dropdown.
pub async fn model_runtime_parameters(
    gateway_url: GatewayUrl,
    base_url: BaseUrl,
    model: &ModelName,
) -> Result<ModelRuntimeParameters> {
    require_gateway_running(gateway_url).await?;

    let parameters = llm::model_parameters(base_url, model).await?;
    let reasoning = reasoning_parameter(&parameters.model_parameters);

    Ok(ModelRuntimeParameters {
        model: model.as_str().to_string(),
        supports_reasoning: parameters.supports_reasoning.unwrap_or(false) || reasoning.is_some(),
        supports_prompt_caching: parameters.supports_prompt_caching.unwrap_or(false),
        reasoning,
        raw: parameters.raw,
    })
}

/// Extracts an effort selector from Bifrost model-parameter metadata.
fn reasoning_parameter(parameters: &[ModelParameter]) -> Option<ReasoningParameter> {
    parameters
        .iter()
        .find(|parameter| parameter.id == "reasoning_effort" && !parameter.options.is_empty())
        .map(|parameter| ReasoningParameter {
            source: ReasoningParameterSource::ReasoningEffort,
            options: parameter.options.clone(),
        })
        .or_else(|| {
            parameters
                .iter()
                .find(|parameter| {
                    parameter.id == "output_config"
                        && parameter.accessor_key.as_deref() == Some("effort")
                        && !parameter.options.is_empty()
                })
                .map(|parameter| ReasoningParameter {
                    source: ReasoningParameterSource::OutputConfigEffort,
                    options: parameter.options.clone(),
                })
        })
}

/// Builds an optional provider prompt-cache request for one conversation turn.
///
/// Bifrost owns model capability metadata. Windie asks for that metadata before
/// a query and only creates a cache hint when the selected model explicitly
/// reports prompt-cache support. Metadata lookup failure is treated as
/// unsupported so prompt caching remains additive and does not block normal
/// queries for custom or older Bifrost model entries.
async fn prompt_cache_request(
    base_url: BaseUrl,
    model: &ModelName,
    conversation_id: &ConversationId,
) -> Option<PromptCacheRequest> {
    let parameters = llm::model_parameters(base_url, model).await.ok()?;
    if !parameters.supports_prompt_caching.unwrap_or(false) {
        return None;
    }

    Some(conversation_prompt_cache_request(conversation_id))
}

/// Creates the stable prompt-cache identity for one Windie conversation.
fn conversation_prompt_cache_request(conversation_id: &ConversationId) -> PromptCacheRequest {
    PromptCacheRequest {
        key: format!("windie:{}", conversation_id.as_str()),
        retention: Some("24h".to_string()),
    }
}

/// Builds the current model-facing input-token context for one conversation.
///
/// This is a read-only preview operation. It builds the same flattened context
/// and attached tool schema list used by query execution, but it does not run
/// query preparation because that path can persist automatic tool results.
/// Bifrost requires at least one Responses input item before it can count tool
/// schema tokens, so a tool-only setup uses a tiny synthetic system message
/// that is never persisted and never sent during a real query.
pub fn conversation_input_token_context(
    store: &Store,
    conversation_id: &ConversationId,
    head_message_id: Option<&MessageId>,
) -> Result<Option<InputTokenCountContext>> {
    let model_context =
        ContextBuilder::build_model_context(store, conversation_id, head_message_id)?;
    let mut model_messages = model_context.messages;
    let tool_schemas = model_context.tool_schemas;
    let source = if model_messages.is_empty() {
        if tool_schemas.is_empty() {
            return Ok(None);
        }
        model_messages.push(synthetic_input_token_count_message());
        InputTokenCountSource::PrequerySyntheticInput
    } else {
        InputTokenCountSource::PrequeryInput
    };

    Ok(Some(InputTokenCountContext {
        model_messages,
        tool_schemas,
        source,
    }))
}

/// Builds the tiny provider input needed to count a tool-only setup.
fn synthetic_input_token_count_message() -> Message {
    Message {
        id: None,
        parent_message_id: None,
        role: Role::System,
        content: SYNTHETIC_INPUT_TOKEN_COUNT_MESSAGE.to_string(),
        parts: Vec::new(),
        metadata: None,
    }
}

/// Counts prepared model-facing input tokens through Bifrost.
pub async fn count_input_tokens_for_context(
    gateway_url: GatewayUrl,
    base_url: BaseUrl,
    model: &ModelName,
    context: Option<InputTokenCountContext>,
) -> Result<Option<InputTokenCount>> {
    let Some(context) = context else {
        return Ok(None);
    };
    require_gateway_running(gateway_url).await?;

    let client = BifrostClient::new(base_url, model.clone());
    client
        .count_input_tokens(&context.model_messages, &context.tool_schemas)
        .await
        .map(Some)
}

/// Loads the full tree for inspection.
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

/// Inserts one message below an explicit parent message.
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
pub fn set_tool_approval_mode(
    store: &mut Store,
    conversation_id: &ConversationId,
    mode: ToolApprovalMode,
) -> Result<()> {
    store.set_tool_approval_mode(conversation_id, mode)
}

/// Lists provider tools that can be attached to conversations.
pub fn available_tools() -> Result<Vec<ToolDefinition>> {
    let registry = ToolProviderRegistry::new();

    available_tools_with_registry(&registry)
}

/// Lists provider tools through a caller-owned registry.
pub fn available_tools_with_registry(
    registry: &ToolProviderRegistry,
) -> Result<Vec<ToolDefinition>> {
    registry.list_available_tools()
}

/// Lists provider tools for one provider ID.
pub fn available_provider_tools(provider_id: &ToolProviderId) -> Result<Vec<ToolDefinition>> {
    let registry = ToolProviderRegistry::new();

    available_provider_tools_with_registry(&registry, provider_id)
}

/// Lists one provider's tools through a caller-owned registry.
pub fn available_provider_tools_with_registry(
    registry: &ToolProviderRegistry,
    provider_id: &ToolProviderId,
) -> Result<Vec<ToolDefinition>> {
    registry.list_provider_tools(provider_id)
}

/// Attaches one available provider tool to a conversation.
pub fn attach_tool(
    store: &mut Store,
    conversation_id: &ConversationId,
    provider_id: &ToolProviderId,
    tool_name: &ProviderToolName,
) -> Result<ToolSchemaName> {
    let registry = ToolProviderRegistry::new();

    attach_tool_with_registry(store, conversation_id, provider_id, tool_name, &registry)
}

/// Attaches one available provider tool using a caller-owned registry.
pub fn attach_tool_with_registry(
    store: &mut Store,
    conversation_id: &ConversationId,
    provider_id: &ToolProviderId,
    tool_name: &ProviderToolName,
    registry: &ToolProviderRegistry,
) -> Result<ToolSchemaName> {
    let definition = registry.find_tool(provider_id, tool_name)?.ok_or_else(|| {
        error::not_found(format!("tool does not exist: {provider_id}/{tool_name}"))
    })?;
    let attached_tool = definition.attached_tool();
    let schema_name = attached_tool.schema_name.clone();

    store.insert_attached_tool(conversation_id, &attached_tool)?;

    Ok(schema_name)
}

/// Attaches one available provider tool to an explicit conversation path.
pub fn attach_tool_with_registry_at_head(
    store: &mut Store,
    conversation_id: &ConversationId,
    head_message_id: Option<&MessageId>,
    provider_id: &ToolProviderId,
    tool_name: &ProviderToolName,
    registry: &ToolProviderRegistry,
) -> Result<ToolSchemaName> {
    let definition = registry.find_tool(provider_id, tool_name)?.ok_or_else(|| {
        error::not_found(format!("tool does not exist: {provider_id}/{tool_name}"))
    })?;
    let attached_tool = definition.attached_tool();
    let schema_name = attached_tool.schema_name.clone();

    store.insert_attached_tool_at_head(conversation_id, head_message_id, &attached_tool)?;

    Ok(schema_name)
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// One requested provider tool attachment in a batch operation.
pub struct ToolAttachmentInput {
    pub provider_id: ToolProviderId,
    pub tool_name: ProviderToolName,
}

impl ToolAttachmentInput {
    /// Builds a typed attachment request from provider identity parts.
    pub fn new(provider_id: ToolProviderId, tool_name: ProviderToolName) -> Self {
        Self {
            provider_id,
            tool_name,
        }
    }
}

/// Attaches multiple available provider tools using a caller-owned registry.
///
/// The provider catalog is loaded at most once per provider ID in the request,
/// so provider-level UI actions do not restart an MCP server for each tool.
/// Storage remains strict: duplicate schema names fail the batch insert.
pub fn attach_tools_with_registry(
    store: &mut Store,
    conversation_id: &ConversationId,
    requests: &[ToolAttachmentInput],
    registry: &ToolProviderRegistry,
) -> Result<Vec<ToolSchemaName>> {
    let mut provider_catalogs: HashMap<ToolProviderId, HashMap<ProviderToolName, ToolDefinition>> =
        HashMap::new();

    for request in requests {
        if provider_catalogs.contains_key(&request.provider_id) {
            continue;
        }
        let provider_tools = registry.list_provider_tools(&request.provider_id)?;
        provider_catalogs.insert(
            request.provider_id.clone(),
            provider_tools
                .into_iter()
                .map(|definition| (definition.provider.tool_name.clone(), definition))
                .collect(),
        );
    }

    let mut attached_tools = Vec::with_capacity(requests.len());
    let mut schema_names = Vec::with_capacity(requests.len());
    for request in requests {
        let definition = provider_catalogs
            .get(&request.provider_id)
            .and_then(|provider_tools| provider_tools.get(&request.tool_name))
            .ok_or_else(|| {
                error::not_found(format!(
                    "tool does not exist: {}/{}",
                    request.provider_id, request.tool_name
                ))
            })?;
        let attached_tool = definition.attached_tool();
        schema_names.push(attached_tool.schema_name.clone());
        attached_tools.push(attached_tool);
    }

    store.insert_attached_tools(conversation_id, &attached_tools)?;

    Ok(schema_names)
}

/// Attaches multiple available provider tools to an explicit conversation path.
pub fn attach_tools_with_registry_at_head(
    store: &mut Store,
    conversation_id: &ConversationId,
    head_message_id: Option<&MessageId>,
    requests: &[ToolAttachmentInput],
    registry: &ToolProviderRegistry,
) -> Result<Vec<ToolSchemaName>> {
    let mut provider_catalogs: HashMap<ToolProviderId, HashMap<ProviderToolName, ToolDefinition>> =
        HashMap::new();

    for request in requests {
        if provider_catalogs.contains_key(&request.provider_id) {
            continue;
        }
        let provider_tools = registry.list_provider_tools(&request.provider_id)?;
        provider_catalogs.insert(
            request.provider_id.clone(),
            provider_tools
                .into_iter()
                .map(|definition| (definition.provider.tool_name.clone(), definition))
                .collect(),
        );
    }

    let mut attached_tools = Vec::with_capacity(requests.len());
    let mut schema_names = Vec::with_capacity(requests.len());
    for request in requests {
        let definition = provider_catalogs
            .get(&request.provider_id)
            .and_then(|provider_tools| provider_tools.get(&request.tool_name))
            .ok_or_else(|| {
                error::not_found(format!(
                    "tool does not exist: {}/{}",
                    request.provider_id, request.tool_name
                ))
            })?;
        let attached_tool = definition.attached_tool();
        schema_names.push(attached_tool.schema_name.clone());
        attached_tools.push(attached_tool);
    }

    store.insert_attached_tools_at_head(conversation_id, head_message_id, &attached_tools)?;

    Ok(schema_names)
}

/// Inserts one tool schema at the active conversation path.
pub fn insert_tool_schema(
    store: &mut Store,
    conversation_id: &ConversationId,
    tool_schema: &ToolSchema,
) -> Result<()> {
    store.insert_tool_schema(conversation_id, tool_schema)
}

/// Inserts one tool schema at an explicit conversation path.
pub fn insert_tool_schema_at_head(
    store: &mut Store,
    conversation_id: &ConversationId,
    head_message_id: Option<&MessageId>,
    tool_schema: &ToolSchema,
) -> Result<()> {
    store.insert_tool_schema_at_head(conversation_id, head_message_id, tool_schema)
}

/// Updates one tool schema at the active conversation path.
pub fn update_tool_schema(
    store: &mut Store,
    conversation_id: &ConversationId,
    current_name: &ToolSchemaName,
    tool_schema: &ToolSchema,
) -> Result<()> {
    store.update_tool_schema(conversation_id, current_name, tool_schema)
}

/// Updates one tool schema at an explicit conversation path.
pub fn update_tool_schema_at_head(
    store: &mut Store,
    conversation_id: &ConversationId,
    head_message_id: Option<&MessageId>,
    current_name: &ToolSchemaName,
    tool_schema: &ToolSchema,
) -> Result<()> {
    store.update_tool_schema_at_head(conversation_id, head_message_id, current_name, tool_schema)
}

/// Removes one tool schema at the active conversation path.
pub fn remove_tool_schema(
    store: &mut Store,
    conversation_id: &ConversationId,
    name: &ToolSchemaName,
) -> Result<()> {
    store.remove_tool_schema(conversation_id, name)
}

/// Removes one tool schema at an explicit conversation path.
pub fn remove_tool_schema_at_head(
    store: &mut Store,
    conversation_id: &ConversationId,
    head_message_id: Option<&MessageId>,
    name: &ToolSchemaName,
) -> Result<()> {
    store.remove_tool_schema_at_head(conversation_id, head_message_id, name)
}

/// Detaches one model-facing tool schema from a conversation.
pub fn detach_tool(
    store: &mut Store,
    conversation_id: &ConversationId,
    schema_name: &ToolSchemaName,
) -> Result<()> {
    remove_tool_schema(store, conversation_id, schema_name)
}

/// Detaches one model-facing tool schema from an explicit conversation path.
pub fn detach_tool_at_head(
    store: &mut Store,
    conversation_id: &ConversationId,
    head_message_id: Option<&MessageId>,
    schema_name: &ToolSchemaName,
) -> Result<()> {
    remove_tool_schema_at_head(store, conversation_id, head_message_id, schema_name)
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

/// Lists pending tool calls at a session-owned message head.
pub fn list_session_tool_approvals_with_registry(
    store: &Store,
    conversation_id: &ConversationId,
    head_message_id: Option<&MessageId>,
    registry: &ToolProviderRegistry,
) -> Result<Vec<ToolApprovalRequest>> {
    pending_approvals_at_head(
        store,
        RuntimeInput {
            conversation_id,
            head_message_id,
            tools: registry,
            model_request: RuntimeModelRequest::new(None, None),
        },
    )
}

/// Lists pending approval requests for one session.
pub fn list_session_approvals_with_registry(
    store: &Store,
    session: &Session,
    registry: &ToolProviderRegistry,
) -> Result<Vec<SessionToolApprovalRequest>> {
    let approvals = list_session_tool_approvals_with_registry(
        store,
        &session.conversation_id,
        session.current_head_message_id.as_ref(),
        registry,
    )?;

    Ok(approvals
        .into_iter()
        .map(|approval| SessionToolApprovalRequest {
            session_id: session.id.clone(),
            conversation_id: session.conversation_id.clone(),
            session_status: session.status,
            head_message_id: session.current_head_message_id.clone(),
            approval,
        })
        .collect())
}

/// Lists pending session-owned approval requests for a conversation.
pub fn list_conversation_session_approvals_with_registry(
    store: &Store,
    conversation_id: &ConversationId,
    registry: &ToolProviderRegistry,
) -> Result<Vec<SessionToolApprovalRequest>> {
    let mut approvals = Vec::new();

    for session in store.list_conversation_sessions(conversation_id)? {
        if session.status != SessionStatus::WaitingForApproval {
            continue;
        }
        approvals.extend(list_session_approvals_with_registry(
            store, &session, registry,
        )?);
    }

    Ok(approvals)
}

/// Provider/runtime inputs needed to execute a run.
///
/// Long-lived API execution and blocking CLI calls both pass through this
/// struct so gateway, Bifrost endpoint, model override, reasoning, and tool
/// executor access stay explicit.
pub struct RuntimeDependencies<'a> {
    gateway_url: GatewayUrl,
    base_url: BaseUrl,
    model_override: Option<ModelName>,
    reasoning: Option<ReasoningRequest>,
    tools: &'a ToolProviderRegistry,
}

impl<'a> RuntimeDependencies<'a> {
    /// Groups provider/runtime dependencies for one session.
    pub fn new(
        gateway_url: GatewayUrl,
        base_url: BaseUrl,
        model_override: Option<ModelName>,
        reasoning: Option<ReasoningRequest>,
        tools: &'a ToolProviderRegistry,
    ) -> Self {
        Self {
            gateway_url,
            base_url,
            model_override,
            reasoning,
            tools,
        }
    }
}

/// Creates a durable session from a wakeup and captures the head/model used.
pub fn start_session_from_wakeup(store: &mut Store, wakeup: ContinueWakeup) -> Result<Session> {
    let head_message_id = wakeup.head_message_id;
    let model = match wakeup.model {
        Some(model) => model,
        None => conversation_model(store, &wakeup.conversation_id)?,
    };
    let session_id = SessionId::fresh();

    store.create_session(
        &session_id,
        &wakeup.conversation_id,
        head_message_id.as_ref(),
        model.as_str(),
        wakeup.reasoning.as_ref(),
    )
}

/// Resolves a session-targeted wakeup into the persisted session and action.
///
/// Conversation wakeups create new sessions through `start_session_from_wakeup`.
/// This helper is only for wakeups that target an already durable session.
pub fn resume_session_from_wakeup(store: &Store, wakeup: Wakeup) -> Result<Option<SessionResume>> {
    let (session_id, action) = match wakeup {
        Wakeup::ApproveTool(decision) => (
            decision.session_id,
            SessionResumeAction::ApproveTool(decision.tool_call_id),
        ),
        Wakeup::DenyTool(decision) => (
            decision.session_id,
            SessionResumeAction::DenyTool(decision.tool_call_id),
        ),
        Wakeup::Stop(stop) => (stop.session_id, SessionResumeAction::Stop),
        Wakeup::Query(_) | Wakeup::Continue(_) => {
            anyhow::bail!("conversation wakeups create sessions instead of resuming them")
        }
    };
    let session = store.load_session(&session_id)?;

    if action != SessionResumeAction::Stop && session.status != SessionStatus::WaitingForApproval {
        return Ok(None);
    }

    Ok(Some(SessionResume { session, action }))
}

/// Persists the terminal status/head and final event for a session outcome.
pub fn finish_session(
    store: &mut Store,
    session_id: &SessionId,
    outcome: RuntimeOutcome,
) -> Result<crate::session::SessionEventRecord> {
    match outcome {
        RuntimeOutcome::Completed { head_message_id } => {
            store.update_session_head(session_id, head_message_id.as_ref())?;
            store.update_session_status(session_id, SessionStatus::Completed, None)?;
            store.append_session_event(
                session_id,
                SessionEvent::Completed {
                    message_id: head_message_id.map(|id| id.as_str().to_string()),
                },
            )
        }
        RuntimeOutcome::WaitingForApproval { head_message_id } => {
            store.update_session_head(session_id, Some(&head_message_id))?;
            store.update_session_status(session_id, SessionStatus::WaitingForApproval, None)?;
            store.append_session_event(session_id, SessionEvent::WaitingForApproval)
        }
    }
}

/// Persists a failed session status and replayable failure event.
pub fn record_session_failure(
    store: &mut Store,
    session_id: &SessionId,
    error: &anyhow::Error,
) -> Result<crate::session::SessionEventRecord> {
    let causes = error.chain().map(ToString::to_string).collect::<Vec<_>>();
    let message = error
        .chain()
        .last()
        .map(ToString::to_string)
        .unwrap_or_else(|| error.to_string());

    store.update_session_status(session_id, SessionStatus::Failed, Some(&message))?;
    store.append_session_event(
        session_id,
        SessionEvent::Failed {
            error: message,
            causes,
        },
    )
}

/// Starts and advances a CLI-owned session from a conversation wakeup.
pub async fn start_cli_session(
    conversation_id: ConversationId,
    head_message_id: Option<MessageId>,
    model: Option<ModelName>,
    gateway_url: GatewayUrl,
    base_url: BaseUrl,
) -> Result<()> {
    let mut store = Store::open()?;
    let session = start_session_from_wakeup(
        &mut store,
        ContinueWakeup {
            conversation_id,
            head_message_id,
            model,
            reasoning: None,
        },
    )?;
    let output = TerminalOutput;

    output.created_session(&session.id);
    continue_cli_session(&mut store, &session.id, gateway_url, base_url).await
}

/// Executes one approved CLI session-owned tool call and continues the session.
pub async fn approve_cli_session_tool(
    session_id: SessionId,
    tool_call_id: ToolCallId,
    gateway_url: GatewayUrl,
    base_url: BaseUrl,
) -> Result<()> {
    let mut store = Store::open()?;
    let session = store.load_session(&session_id)?;
    let registry = ToolProviderRegistry::new();
    let runtime = RuntimeDependencies::new(
        gateway_url,
        base_url,
        Some(ModelName::new(session.model)),
        session.reasoning,
        &registry,
    );
    let cli_output = CliSessionOutput::new(session_id.clone());
    let events = CliSessionEvents::new(session_id.clone());
    let outcome = approve_session_tool(
        &cli_output,
        &events,
        &mut store,
        &session.conversation_id,
        session.current_head_message_id.as_ref(),
        &tool_call_id,
        runtime,
    )
    .await?;

    finish_session(&mut store, &session_id, outcome)?;
    Ok(())
}

/// Stores one denied CLI session-owned tool result and continues the session.
pub async fn deny_cli_session_tool(
    session_id: SessionId,
    tool_call_id: ToolCallId,
    gateway_url: GatewayUrl,
    base_url: BaseUrl,
) -> Result<()> {
    let mut store = Store::open()?;
    let session = store.load_session(&session_id)?;
    let registry = ToolProviderRegistry::new();
    let runtime = RuntimeDependencies::new(
        gateway_url,
        base_url,
        Some(ModelName::new(session.model)),
        session.reasoning,
        &registry,
    );
    let cli_output = CliSessionOutput::new(session_id.clone());
    let events = CliSessionEvents::new(session_id.clone());
    let outcome = deny_session_tool(
        &cli_output,
        &events,
        &mut store,
        &session.conversation_id,
        session.current_head_message_id.as_ref(),
        &tool_call_id,
        runtime,
    )
    .await?;

    finish_session(&mut store, &session_id, outcome)?;
    Ok(())
}

/// Cancels one persisted CLI session and returns the updated state.
pub fn cancel_session(session_id: &SessionId) -> Result<Session> {
    let mut store = Store::open()?;

    store.update_session_status(session_id, SessionStatus::Cancelled, None)?;
    store.load_session(session_id)
}

/// Continues a CLI-owned session until it completes or reaches approval.
async fn continue_cli_session(
    store: &mut Store,
    session_id: &SessionId,
    gateway_url: GatewayUrl,
    base_url: BaseUrl,
) -> Result<()> {
    let session = store.load_session(session_id)?;
    store.update_session_status(session_id, SessionStatus::Running, None)?;
    let registry = ToolProviderRegistry::new();
    let runtime = RuntimeDependencies::new(
        gateway_url,
        base_url,
        Some(ModelName::new(session.model)),
        session.reasoning,
        &registry,
    );
    let cli_output = CliSessionOutput::new(session_id.clone());
    let events = CliSessionEvents::new(session_id.clone());
    let outcome = advance_session_until_blocked(
        &cli_output,
        &events,
        store,
        &session.conversation_id,
        session.current_head_message_id.as_ref(),
        runtime,
    )
    .await?;

    finish_session(store, session_id, outcome)?;
    Ok(())
}

/// CLI runtime output that prints to the terminal and appends replayable events.
struct CliSessionOutput {
    session_id: SessionId,
    terminal: TerminalOutput,
}

impl CliSessionOutput {
    fn new(session_id: SessionId) -> Self {
        Self {
            session_id,
            terminal: TerminalOutput,
        }
    }

    fn record(&self, event: SessionEvent) -> Result<()> {
        let mut store = Store::open()?;
        store.append_session_event(&self.session_id, event)?;

        Ok(())
    }
}

impl RuntimeOutput for CliSessionOutput {
    fn start_assistant_message(&self) {
        self.terminal.start_assistant_message();
    }

    fn assistant_delta(&self, text: &str) -> Result<()> {
        self.record(SessionEvent::AssistantDelta {
            text: text.to_string(),
        })?;
        self.terminal.assistant_delta(text)
    }

    fn reasoning_delta(&self, text: &str) -> Result<()> {
        self.record(SessionEvent::ReasoningDelta {
            text: text.to_string(),
        })
    }

    fn tool_call_delta(
        &self,
        index: u16,
        id: Option<&str>,
        name: Option<&str>,
        arguments_delta: Option<&str>,
    ) -> Result<()> {
        self.record(SessionEvent::ToolCallDelta {
            index,
            id: id.map(str::to_string),
            name: name.map(str::to_string),
            arguments_delta: arguments_delta.map(str::to_string),
        })
    }

    fn end_assistant_message(&self) {
        self.terminal.end_assistant_message();
    }

    fn assistant_tool_calls(&self, tool_calls: &[crate::conversation::ToolCall]) {
        self.terminal.assistant_tool_calls(tool_calls);
    }
}

/// CLI runtime sink for durable message events.
struct CliSessionEvents {
    session_id: SessionId,
}

impl CliSessionEvents {
    fn new(session_id: SessionId) -> Self {
        Self { session_id }
    }

    fn record(&self, event: SessionEvent) {
        match Store::open()
            .and_then(|mut store| store.append_session_event(&self.session_id, event))
        {
            Ok(_) => {}
            Err(error) => eprintln!("failed to append runtime event: {error}"),
        }
    }
}

impl RuntimeEventSink for CliSessionEvents {
    fn assistant_message_saved(&self, message_id: &MessageId) {
        self.record(SessionEvent::AssistantMessageSaved {
            message_id: message_id.as_str().to_string(),
        });
    }

    fn tool_result_saved(&self, message_id: &MessageId) {
        self.record(SessionEvent::ToolResultSaved {
            message_id: message_id.as_str().to_string(),
        });
    }
}

/// Advances one backend-owned execution until it completes or waits for approval.
pub async fn advance_session_until_blocked<O, E>(
    output: &O,
    events: &E,
    store: &mut Store,
    conversation_id: &ConversationId,
    head_message_id: Option<&MessageId>,
    runtime: RuntimeDependencies<'_>,
) -> Result<RuntimeOutcome>
where
    O: RuntimeOutput,
    E: RuntimeEventSink,
{
    require_gateway_running(runtime.gateway_url).await?;
    let model = resolve_conversation_model(store, conversation_id, runtime.model_override)?;
    let reasoning = resolve_reasoning_request(store, conversation_id, runtime.reasoning)?;
    let reasoning = reasoning_request_for_model(&model, reasoning);
    let prompt_cache =
        prompt_cache_request(runtime.base_url.clone(), &model, conversation_id).await;
    let llm = BifrostClient::new(runtime.base_url, model);

    runtime_advance_until_blocked(
        output,
        &llm,
        store,
        RuntimeInput {
            conversation_id,
            head_message_id,
            tools: runtime.tools,
            model_request: RuntimeModelRequest::new(reasoning.as_ref(), prompt_cache.as_ref()),
        },
        events,
    )
    .await
}

/// Resolves the reasoning request for a runtime operation.
///
/// A caller-supplied request is a one-query override. When it is absent,
/// Windie uses the conversation-level persisted effort so CLI, API, and
/// inspector clients all flow through the same primitive.
fn resolve_reasoning_request(
    store: &Store,
    conversation_id: &ConversationId,
    reasoning_override: Option<ReasoningRequest>,
) -> Result<Option<ReasoningRequest>> {
    match reasoning_override {
        Some(reasoning) => Ok(Some(reasoning)),
        None => conversation_reasoning(store, conversation_id),
    }
}

/// Converts a client-selected reasoning setting into the request Windie should
/// send for one concrete model.
///
/// The UI only chooses a reasoning effort from Bifrost metadata. OpenAI
/// Responses models need an additional `summary` request before they stream
/// visible reasoning-summary deltas, so Windie adds that provider request
/// detail here instead of teaching every client about OpenAI-specific fields.
fn reasoning_request_for_model(
    model: &ModelName,
    reasoning: Option<ReasoningRequest>,
) -> Option<ReasoningRequest> {
    let mut reasoning = reasoning.filter(|reasoning| !reasoning.is_empty())?;

    if model.as_str().starts_with("openai/")
        && reasoning.effort.is_some()
        && reasoning.summary.is_none()
    {
        reasoning.summary = Some("auto".to_string());
    }

    Some(reasoning)
}

/// Executes one approved session-scoped tool call and continues that session.
pub async fn approve_session_tool<O, E>(
    output: &O,
    events: &E,
    store: &mut Store,
    conversation_id: &ConversationId,
    head_message_id: Option<&MessageId>,
    tool_call_id: &ToolCallId,
    runtime: RuntimeDependencies<'_>,
) -> Result<RuntimeOutcome>
where
    O: RuntimeOutput,
    E: RuntimeEventSink,
{
    let pending =
        load_pending_tool_call_at_head(store, conversation_id, head_message_id, tool_call_id)?;
    let execution = prepare_pending_tool_execution(
        store,
        conversation_id,
        head_message_id,
        &pending,
        runtime.tools,
    )?;
    let result = match execution {
        PendingToolExecution::Finished(result) => result,
        PendingToolExecution::Execute(attached_tool) => {
            execute_pending_tool_call(&pending, &attached_tool, runtime.tools).await?
        }
    };
    let message_id = store_pending_tool_result_at_head(store, conversation_id, &pending, &result)?;
    events.tool_result_saved(&message_id);

    continue_session_after_tool_result(output, events, store, conversation_id, &message_id, runtime)
        .await
}

/// Stores one denied session-scoped tool result and continues that session.
pub async fn deny_session_tool<O, E>(
    output: &O,
    events: &E,
    store: &mut Store,
    conversation_id: &ConversationId,
    head_message_id: Option<&MessageId>,
    tool_call_id: &ToolCallId,
    runtime: RuntimeDependencies<'_>,
) -> Result<RuntimeOutcome>
where
    O: RuntimeOutput,
    E: RuntimeEventSink,
{
    let pending =
        load_pending_tool_call_at_head(store, conversation_id, head_message_id, tool_call_id)?;
    let result = deny_pending_tool_call(&pending);
    let message_id = store_pending_tool_result_at_head(store, conversation_id, &pending, &result)?;
    events.tool_result_saved(&message_id);

    continue_session_after_tool_result(output, events, store, conversation_id, &message_id, runtime)
        .await
}

/// Continues a run after a stored tool result only when no manual approval remains.
async fn continue_session_after_tool_result<O, E>(
    output: &O,
    events: &E,
    store: &mut Store,
    conversation_id: &ConversationId,
    head_message_id: &MessageId,
    runtime: RuntimeDependencies<'_>,
) -> Result<RuntimeOutcome>
where
    O: RuntimeOutput,
    E: RuntimeEventSink,
{
    if !pending_approvals_at_head(
        store,
        RuntimeInput {
            conversation_id,
            head_message_id: Some(head_message_id),
            tools: runtime.tools,
            model_request: RuntimeModelRequest::new(None, None),
        },
    )?
    .is_empty()
    {
        return Ok(RuntimeOutcome::WaitingForApproval {
            head_message_id: head_message_id.clone(),
        });
    }

    advance_session_until_blocked(
        output,
        events,
        store,
        conversation_id,
        Some(head_message_id),
        runtime,
    )
    .await
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
    use crate::mcp::McpCommand;
    use crate::tool::{ToolAnnotations, ToolPermission, ToolProviderKind, ToolProviderRef};
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn inserts_text_message() {
        let mut store = Store::open_memory().unwrap();
        let conversation_id = create_conversation(&store, &ModelName::new("openai/test")).unwrap();

        let message_id = insert_message(
            &mut store,
            &conversation_id,
            None,
            Role::User,
            &[MessageInputPart::Text("hello".to_string())],
        )
        .unwrap();

        let messages = store
            .load_path_to_message(&conversation_id, &message_id)
            .unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].id.as_ref(), Some(&message_id));
        assert_eq!(messages[0].content, "hello");
    }

    #[test]
    fn builds_conversation_prompt_cache_request() {
        let conversation_id = ConversationId::new("conversation-id");

        let prompt_cache = conversation_prompt_cache_request(&conversation_id);

        assert_eq!(prompt_cache.key, "windie:conversation-id");
        assert_eq!(prompt_cache.retention.as_deref(), Some("24h"));
    }

    #[test]
    fn openai_reasoning_effort_requests_visible_summary() {
        let reasoning = reasoning_request_for_model(
            &ModelName::new("openai/gpt-5.5"),
            Some(ReasoningRequest {
                effort: Some("high".to_string()),
                summary: None,
            }),
        )
        .unwrap();

        assert_eq!(reasoning.effort.as_deref(), Some("high"));
        assert_eq!(reasoning.summary.as_deref(), Some("auto"));
    }

    #[test]
    fn openai_reasoning_preserves_explicit_summary() {
        let reasoning = reasoning_request_for_model(
            &ModelName::new("openai/gpt-5.5"),
            Some(ReasoningRequest {
                effort: Some("high".to_string()),
                summary: Some("detailed".to_string()),
            }),
        )
        .unwrap();

        assert_eq!(reasoning.effort.as_deref(), Some("high"));
        assert_eq!(reasoning.summary.as_deref(), Some("detailed"));
    }

    #[test]
    fn anthropic_reasoning_does_not_request_openai_summary() {
        let reasoning = reasoning_request_for_model(
            &ModelName::new("anthropic/claude-fable-5"),
            Some(ReasoningRequest {
                effort: Some("high".to_string()),
                summary: None,
            }),
        )
        .unwrap();

        assert_eq!(reasoning.effort.as_deref(), Some("high"));
        assert_eq!(reasoning.summary, None);
    }

    #[test]
    fn empty_reasoning_request_stays_absent() {
        let reasoning = reasoning_request_for_model(
            &ModelName::new("openai/gpt-5.5"),
            Some(ReasoningRequest {
                effort: None,
                summary: None,
            }),
        );

        assert_eq!(reasoning, None);
    }

    #[test]
    fn persisted_reasoning_resolves_without_request_override() {
        let mut store = Store::open_memory().unwrap();
        let conversation_id = create_conversation(&store, &ModelName::new("openai/test")).unwrap();
        set_conversation_reasoning_effort(&mut store, &conversation_id, Some("medium")).unwrap();

        let reasoning = resolve_reasoning_request(&store, &conversation_id, None).unwrap();

        assert_eq!(reasoning.unwrap().effort.as_deref(), Some("medium"));
    }

    #[test]
    fn request_reasoning_overrides_persisted_reasoning() {
        let mut store = Store::open_memory().unwrap();
        let conversation_id = create_conversation(&store, &ModelName::new("openai/test")).unwrap();
        set_conversation_reasoning_effort(&mut store, &conversation_id, Some("medium")).unwrap();

        let reasoning = resolve_reasoning_request(
            &store,
            &conversation_id,
            Some(ReasoningRequest {
                effort: Some("high".to_string()),
                summary: None,
            }),
        )
        .unwrap();

        assert_eq!(reasoning.unwrap().effort.as_deref(), Some("high"));
    }

    #[test]
    fn rejects_direct_tool_message_insert() {
        let mut store = Store::open_memory().unwrap();
        let conversation_id = create_conversation(&store, &ModelName::new("openai/test")).unwrap();

        let error = insert_message(
            &mut store,
            &conversation_id,
            None,
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
        let conversation_id = create_conversation(&store, &ModelName::new("openai/test")).unwrap();
        let image_path = temp_image_path("png");
        fs::write(
            &image_path,
            [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a],
        )
        .unwrap();

        insert_message(
            &mut store,
            &conversation_id,
            None,
            Role::User,
            &[
                MessageInputPart::Text("first".to_string()),
                MessageInputPart::ImagePath(image_path.clone()),
            ],
        )
        .unwrap();

        let messages = store.load_messages(&conversation_id).unwrap();
        assert_eq!(messages[0].content, "first");
        assert_eq!(messages[0].parts.len(), 2);
        fs::remove_file(image_path).unwrap();
    }

    #[test]
    fn inserts_loaded_image_bytes() {
        let mut store = Store::open_memory().unwrap();
        let conversation_id = create_conversation(&store, &ModelName::new("openai/test")).unwrap();

        insert_message(
            &mut store,
            &conversation_id,
            None,
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

        let messages = store.load_messages(&conversation_id).unwrap();
        assert_eq!(messages[0].content, "clipboard");
        assert_eq!(messages[0].parts.len(), 2);
    }

    #[test]
    fn input_token_context_uses_synthetic_input_for_tool_only_setup() {
        let mut store = Store::open_memory().unwrap();
        let conversation_id = create_conversation(&store, &ModelName::new("openai/test")).unwrap();
        insert_tool_schema(
            &mut store,
            &conversation_id,
            &ToolSchema {
                name: ToolSchemaName::new("run_shell"),
                description: "Run a shell command".to_string(),
                parameters: serde_json::json!({"type":"object"}),
            },
        )
        .unwrap();

        let context = conversation_input_token_context(&store, &conversation_id, None)
            .unwrap()
            .unwrap();

        assert_eq!(
            context.source(),
            InputTokenCountSource::PrequerySyntheticInput
        );
        assert_eq!(context.model_messages.len(), 1);
        assert_eq!(context.model_messages[0].role, Role::System);
        assert_eq!(
            context.model_messages[0].content,
            SYNTHETIC_INPUT_TOKEN_COUNT_MESSAGE
        );
        assert_eq!(context.tool_schemas.len(), 1);
    }

    #[test]
    fn inspection_snapshot_includes_runtime_state() {
        let mut store = Store::open_memory().unwrap();
        let conversation_id = create_conversation(&store, &ModelName::new("openai/test")).unwrap();
        set_conversation_model(
            &mut store,
            &conversation_id,
            &ModelName::new("anthropic/test"),
        )
        .unwrap();
        set_conversation_reasoning_effort(&mut store, &conversation_id, Some("high")).unwrap();
        set_system_prompt(&mut store, &conversation_id, "You are concise.").unwrap();
        let user_id = insert_message(
            &mut store,
            &conversation_id,
            None,
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

        let report = inspect_conversation(&store, &conversation_id, Some(&user_id), None).unwrap();
        let value = serde_json::to_value(report).unwrap();

        assert_eq!(value["conversation_id"], conversation_id.as_str());
        assert_eq!(value["head_message_id"], user_id.as_str());
        assert_eq!(value["model"], "anthropic/test");
        assert_eq!(value["reasoning"]["effort"], "high");
        assert_eq!(value["system_prompt"], "You are concise.");
        assert_eq!(value["tool_schemas"][0]["name"], "run_shell");
        assert_eq!(value["messages"][0]["role"], "system");
        assert_eq!(value["messages"][1]["id"], user_id.as_str());
        assert_eq!(value["path"][0]["id"], user_id.as_str());
        assert_eq!(value["model_context"][0]["role"], "system");
        assert_eq!(value["latest_compaction"]["content"], "hello happened");
    }

    #[test]
    fn attaches_available_provider_tool() {
        let mut store = Store::open_memory().unwrap();
        let conversation_id = create_conversation(&store, &ModelName::new("openai/test")).unwrap();
        let registry = registry_with_cached_test_tool();
        let read_file = registry
            .find_tool(
                &ToolProviderId::new("desktop-commander"),
                &ProviderToolName::new("read_file"),
            )
            .unwrap()
            .unwrap();

        let schema_name = attach_tool_with_registry(
            &mut store,
            &conversation_id,
            &ToolProviderId::new("desktop-commander"),
            &ProviderToolName::new("read_file"),
            &registry,
        )
        .unwrap();
        let attached_tools = store.load_attached_tools(&conversation_id).unwrap();

        assert_eq!(read_file.schema_name, schema_name);
        assert_eq!(schema_name.as_str(), "desktop_commander__read_file");
        assert_eq!(attached_tools.len(), 1);
        assert_eq!(
            attached_tools[0].provider.provider_id.as_str(),
            "desktop-commander"
        );
        assert_eq!(attached_tools[0].provider.tool_name.as_str(), "read_file");
    }

    #[test]
    fn batch_attaches_available_provider_tools() {
        let mut store = Store::open_memory().unwrap();
        let conversation_id = create_conversation(&store, &ModelName::new("openai/test")).unwrap();

        let registry = registry_with_cached_test_tool();
        let schema_names = attach_tools_with_registry(
            &mut store,
            &conversation_id,
            &[ToolAttachmentInput::new(
                ToolProviderId::new("desktop-commander"),
                ProviderToolName::new("read_file"),
            )],
            &registry,
        )
        .unwrap();
        let attached_tools = store.load_attached_tools(&conversation_id).unwrap();

        assert_eq!(schema_names.len(), 1);
        assert_eq!(schema_names[0].as_str(), "desktop_commander__read_file");
        assert_eq!(attached_tools.len(), 1);
        assert_eq!(
            attached_tools[0].provider.provider_id.as_str(),
            "desktop-commander"
        );
        assert_eq!(attached_tools[0].provider.tool_name.as_str(), "read_file");
    }

    #[test]
    fn shared_operations_match_direct_store_state() {
        let mut store = Store::open_memory().unwrap();
        let conversation_id = create_conversation(&store, &ModelName::new("openai/test")).unwrap();
        let first_id = insert_message(
            &mut store,
            &conversation_id,
            None,
            Role::User,
            &[MessageInputPart::Text("first".to_string())],
        )
        .unwrap();
        let second_id = insert_message(
            &mut store,
            &conversation_id,
            Some(&first_id),
            Role::Assistant,
            &[MessageInputPart::Text("second".to_string())],
        )
        .unwrap();

        update_message(&mut store, &conversation_id, &second_id, "second updated").unwrap();

        let path = store
            .load_path_to_message(&conversation_id, &second_id)
            .unwrap();
        assert_eq!(path.len(), 2);
        assert_eq!(path[1].content, "second updated");
    }

    #[test]
    fn start_session_from_wakeup_captures_requested_head() {
        let mut store = Store::open_memory().unwrap();
        let conversation_id = create_conversation(&store, &ModelName::new("openai/test")).unwrap();
        let user_id = insert_message(
            &mut store,
            &conversation_id,
            None,
            Role::User,
            &[MessageInputPart::Text("hello".to_string())],
        )
        .unwrap();

        let session = start_session_from_wakeup(
            &mut store,
            crate::wakeup::ContinueWakeup {
                conversation_id: conversation_id.clone(),
                head_message_id: Some(user_id.clone()),
                model: None,
                reasoning: None,
            },
        )
        .unwrap();

        assert_eq!(session.conversation_id, conversation_id);
        assert_eq!(session.start_head_message_id.as_ref(), Some(&user_id));
        assert_eq!(session.current_head_message_id.as_ref(), Some(&user_id));
        assert_eq!(session.model, "openai/test");
        assert_eq!(session.status, SessionStatus::Running);
    }

    #[test]
    fn resume_session_from_wakeup_resolves_waiting_approval() {
        let mut store = Store::open_memory().unwrap();
        let conversation_id = create_conversation(&store, &ModelName::new("openai/test")).unwrap();
        let head_message_id = insert_message(
            &mut store,
            &conversation_id,
            None,
            Role::Assistant,
            &[MessageInputPart::Text("tool call pending".to_string())],
        )
        .unwrap();
        let session_id = SessionId::fresh();
        store
            .create_session(
                &session_id,
                &conversation_id,
                Some(&head_message_id),
                "openai/test",
                None,
            )
            .unwrap();
        store
            .update_session_status(&session_id, SessionStatus::WaitingForApproval, None)
            .unwrap();

        let resume = resume_session_from_wakeup(
            &store,
            crate::wakeup::Wakeup::ApproveTool(crate::wakeup::ToolDecisionWakeup {
                session_id: session_id.clone(),
                tool_call_id: ToolCallId::new("call_1"),
            }),
        )
        .unwrap()
        .unwrap();

        assert_eq!(resume.session.id, session_id);
        assert_eq!(
            resume.action,
            SessionResumeAction::ApproveTool(ToolCallId::new("call_1"))
        );
    }

    #[test]
    fn resume_session_from_wakeup_ignores_non_waiting_approval() {
        let mut store = Store::open_memory().unwrap();
        let conversation_id = create_conversation(&store, &ModelName::new("openai/test")).unwrap();
        let head_message_id = insert_message(
            &mut store,
            &conversation_id,
            None,
            Role::Assistant,
            &[MessageInputPart::Text("complete".to_string())],
        )
        .unwrap();
        let session_id = SessionId::fresh();
        store
            .create_session(
                &session_id,
                &conversation_id,
                Some(&head_message_id),
                "openai/test",
                None,
            )
            .unwrap();
        store
            .update_session_status(&session_id, SessionStatus::Completed, None)
            .unwrap();

        let resume = resume_session_from_wakeup(
            &store,
            crate::wakeup::Wakeup::DenyTool(crate::wakeup::ToolDecisionWakeup {
                session_id,
                tool_call_id: ToolCallId::new("call_1"),
            }),
        )
        .unwrap();

        assert!(resume.is_none());
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

    fn registry_with_cached_test_tool() -> ToolProviderRegistry {
        ToolProviderRegistry::with_test_mcp_provider(
            "desktop-commander",
            "desktop_commander",
            "Desktop Commander",
            McpCommand {
                program: "windie-test-unused-mcp-provider",
                args: &[],
                env: &[],
            },
            vec![desktop_commander_read_file_definition()],
        )
    }

    fn desktop_commander_read_file_definition() -> ToolDefinition {
        ToolDefinition {
            schema_name: ToolSchemaName::new("desktop_commander__read_file"),
            display_name: "Desktop Commander read_file".to_string(),
            description: "Read a file through Desktop Commander.".to_string(),
            parameters: serde_json::json!({"type":"object"}),
            provider: ToolProviderRef::new(
                ToolProviderId::new("desktop-commander"),
                ProviderToolName::new("read_file"),
                ToolProviderKind::Mcp,
            ),
            permissions: vec![ToolPermission::ExternalProcess],
            annotations: ToolAnnotations::default(),
        }
    }
}
