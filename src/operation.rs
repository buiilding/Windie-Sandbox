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

use crate::context::{ContextBuilder, ContextParts};
use serde::Serialize;

use crate::conversation::{
    ConversationId, Message, MessageId, MessageMetadata, MessagePart, Role, ToolCallId, ToolSchema,
    ToolSchemaName, UnsavedImagePart, UnsavedMessagePart,
};
use crate::error;
use crate::gateway::{BifrostGateway, GatewayStart, GatewayStop, GatewayUrl};
use crate::image_input::{ImageInput, read_image_input, validate_image_input_bytes};
use crate::llm::{
    self, BaseUrl, BifrostClient, InputTokenCount, InputTokenCountOutcome, ModelInfo, ModelName,
    ModelParameter, ModelParameterOption, PromptCacheRequest, ReasoningRequest,
};
use crate::output::RuntimeOutput;
use crate::runtime::{
    PendingToolExecution, RuntimeEventSink, RuntimeModelRequest, approve_tool_call,
    approve_tool_call_with_registry, deny_pending_tool_call, deny_tool_call,
    execute_pending_tool_call, load_pending_tool_call, pending_tool_approvals,
    pending_tool_approvals_with_registry, prepare_pending_tool_execution,
    query_conversation_resolving_automatic_tools,
    query_conversation_resolving_automatic_tools_with_events, store_pending_tool_result,
};
use crate::store::{Compaction, ConversationInfo, Store};
use crate::tool::{
    ProviderToolName, ToolApprovalMode, ToolApprovalRequest, ToolDefinition, ToolExecutionResult,
    ToolProviderId,
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

const SYNTHETIC_INPUT_TOKEN_COUNT_MESSAGE: &str = ".";

/// Full message tree plus the selected active node.
pub struct ConversationTree {
    pub messages: Vec<Message>,
    pub active_message_id: Option<MessageId>,
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

#[derive(Debug, Clone, PartialEq)]
/// Client-facing outcome for a pre-query input-token count request.
pub enum InputTokenCountResult {
    Count(InputTokenCount),
    Unsupported,
    EmptyContext,
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
/// Machine-readable snapshot of one conversation's current runtime state.
///
/// CLI JSON and API inspection both serialize this same operation-level shape.
/// It deliberately summarizes image bytes instead of exposing raw image data.
pub struct InspectionReport {
    conversation_id: String,
    active_message_id: Option<String>,
    model: String,
    reasoning: Option<ReasoningRequest>,
    system_prompt: Option<String>,
    tool_approval_mode: ToolApprovalMode,
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
        reasoning: Option<ReasoningRequest>,
        system_prompt: Option<String>,
        tool_approval_mode: ToolApprovalMode,
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
            reasoning,
            system_prompt,
            tool_approval_mode,
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

    let Some(parameters) = llm::model_parameters(base_url, model).await? else {
        return Ok(ModelRuntimeParameters {
            model: model.as_str().to_string(),
            supports_reasoning: false,
            supports_prompt_caching: false,
            reasoning: None,
            raw: serde_json::Value::Null,
        });
    };
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
    let parameters = llm::model_parameters(base_url, model)
        .await
        .ok()
        .flatten()?;
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
) -> Result<Option<InputTokenCountContext>> {
    let mut model_messages = ContextBuilder::build(store, conversation_id)?;
    let tool_schemas = store.load_tool_schemas(conversation_id)?;
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
) -> Result<InputTokenCountResult> {
    let Some(context) = context else {
        return Ok(InputTokenCountResult::EmptyContext);
    };
    require_gateway_running(gateway_url).await?;

    let client = BifrostClient::new(base_url, model.clone());
    match client
        .count_input_tokens(&context.model_messages, &context.tool_schemas)
        .await
    {
        Ok(InputTokenCountOutcome::Count(count)) => Ok(InputTokenCountResult::Count(count)),
        Ok(InputTokenCountOutcome::Unsupported) => Ok(InputTokenCountResult::Unsupported),
        Err(error) => Err(error),
    }
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
    model_override: Option<ModelName>,
) -> Result<InspectionReport> {
    let model = resolve_conversation_model(store, conversation_id, model_override)?;
    let reasoning = conversation_reasoning(store, conversation_id)?;
    let active_message_id = store.active_message_id(conversation_id)?;
    let tool_approval_mode = store.tool_approval_mode(conversation_id)?;
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
        reasoning,
        context_parts.system_prompt,
        tool_approval_mode,
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

/// Lists pending active-path tool calls through a caller-owned registry.
pub fn list_tool_approvals_with_registry(
    store: &Store,
    conversation_id: &ConversationId,
    registry: &ToolProviderRegistry,
) -> Result<Vec<ToolApprovalRequest>> {
    pending_tool_approvals_with_registry(store, conversation_id, registry)
}

/// Runs the shared CLI/API query sequence for the next assistant response.
///
/// Clients pass runtime settings in, but this operation owns the repeated
/// sequence: require the local gateway, construct the OpenAI-compatible Bifrost
/// client, then let runtime auto-resolve denied or auto-approved tools until it
/// reaches a normal assistant message or a manual approval boundary.
pub async fn query_conversation<O>(
    output: &O,
    store: &mut Store,
    conversation_id: &ConversationId,
    gateway_url: GatewayUrl,
    base_url: BaseUrl,
    model_override: Option<ModelName>,
    reasoning: Option<ReasoningRequest>,
) -> Result<Message>
where
    O: RuntimeOutput,
{
    require_gateway_running(gateway_url).await?;
    let model = resolve_conversation_model(store, conversation_id, model_override)?;
    let reasoning = resolve_reasoning_request(store, conversation_id, reasoning)?;
    let reasoning = llm::reasoning_request_for_model(&model, reasoning);
    let prompt_cache = prompt_cache_request(base_url.clone(), &model, conversation_id).await;
    let llm = BifrostClient::new(base_url, model);
    let registry = ToolProviderRegistry::new();

    query_conversation_resolving_automatic_tools(
        output,
        &llm,
        store,
        conversation_id,
        &registry,
        reasoning.as_ref(),
        prompt_cache.as_ref(),
    )
    .await
}

/// Runs the shared query sequence with a caller-owned provider registry.
///
/// Long-lived clients such as the API server use this path so auto-approved MCP
/// calls reuse the same registry/session behavior as explicit approvals.
pub async fn query_conversation_with_registry<O>(
    output: &O,
    store: &mut Store,
    conversation_id: &ConversationId,
    runtime: RuntimeTurnConfig<'_>,
) -> Result<Message>
where
    O: RuntimeOutput,
{
    require_gateway_running(runtime.gateway_url).await?;
    let model = resolve_conversation_model(store, conversation_id, runtime.model_override)?;
    let reasoning = resolve_reasoning_request(store, conversation_id, runtime.reasoning)?;
    let reasoning = llm::reasoning_request_for_model(&model, reasoning);
    let prompt_cache =
        prompt_cache_request(runtime.base_url.clone(), &model, conversation_id).await;
    let llm = BifrostClient::new(runtime.base_url, model);

    query_conversation_resolving_automatic_tools(
        output,
        &llm,
        store,
        conversation_id,
        runtime.registry,
        reasoning.as_ref(),
        prompt_cache.as_ref(),
    )
    .await
}

/// Provider/runtime inputs needed to execute one model-backed runtime turn.
///
/// Query, approval, and denial flows share these values. Grouping them keeps
/// call sites explicit without growing long parameter lists.
pub struct RuntimeTurnConfig<'a> {
    gateway_url: GatewayUrl,
    base_url: BaseUrl,
    model_override: Option<ModelName>,
    reasoning: Option<ReasoningRequest>,
    registry: &'a ToolProviderRegistry,
}

impl<'a> RuntimeTurnConfig<'a> {
    /// Groups the gateway, Bifrost endpoint, optional model override, and
    /// provider registry.
    pub fn new(
        gateway_url: GatewayUrl,
        base_url: BaseUrl,
        model_override: Option<ModelName>,
        reasoning: Option<ReasoningRequest>,
        registry: &'a ToolProviderRegistry,
    ) -> Self {
        Self {
            gateway_url,
            base_url,
            model_override,
            reasoning,
            registry,
        }
    }
}

/// Runs one streamed runtime query turn while emitting durable runtime events.
///
/// The API streaming route uses this path to notify clients after assistant
/// messages and tool results have been persisted. Existing blocking callers use
/// `query_conversation_with_registry`, which keeps the same runtime flow with a
/// no-op event sink.
pub async fn query_runtime_turn<O, E>(
    output: &O,
    events: &E,
    store: &mut Store,
    conversation_id: &ConversationId,
    runtime: RuntimeTurnConfig<'_>,
) -> Result<Message>
where
    O: RuntimeOutput,
    E: RuntimeEventSink,
{
    require_gateway_running(runtime.gateway_url).await?;
    let model = resolve_conversation_model(store, conversation_id, runtime.model_override)?;
    let reasoning = resolve_reasoning_request(store, conversation_id, runtime.reasoning)?;
    let reasoning = llm::reasoning_request_for_model(&model, reasoning);
    let prompt_cache =
        prompt_cache_request(runtime.base_url.clone(), &model, conversation_id).await;
    let llm = BifrostClient::new(runtime.base_url, model);

    query_conversation_resolving_automatic_tools_with_events(
        output,
        &llm,
        store,
        conversation_id,
        runtime.registry,
        events,
        RuntimeModelRequest::new(reasoning.as_ref(), prompt_cache.as_ref()),
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

/// Executes one approved pending tool call and persists its result.
pub async fn approve_tool(
    store: &mut Store,
    conversation_id: &ConversationId,
    tool_call_id: &ToolCallId,
) -> Result<ToolExecutionResult> {
    approve_tool_call(store, conversation_id, tool_call_id).await
}

/// Executes one approved pending tool call with a caller-owned provider registry.
pub async fn approve_tool_with_registry(
    store: &mut Store,
    conversation_id: &ConversationId,
    tool_call_id: &ToolCallId,
    registry: &ToolProviderRegistry,
) -> Result<ToolExecutionResult> {
    approve_tool_call_with_registry(store, conversation_id, tool_call_id, registry).await
}

/// Persists a rejected result for one pending tool call.
pub fn deny_tool(
    store: &mut Store,
    conversation_id: &ConversationId,
    tool_call_id: &ToolCallId,
) -> Result<ToolExecutionResult> {
    deny_tool_call(store, conversation_id, tool_call_id)
}

/// Executes one approved tool call, emits its persisted result, and continues
/// the runtime when no later approval is waiting.
///
/// This is the client-facing approval behavior: approval resolves one pending
/// call and lets Windie advance if the active path is ready. Multi-tool turns
/// stop after the stored result when the next requested call still needs manual
/// approval.
pub async fn approve_tool_turn<O, E>(
    output: &O,
    events: &E,
    store: &mut Store,
    conversation_id: &ConversationId,
    tool_call_id: &ToolCallId,
    runtime: RuntimeTurnConfig<'_>,
) -> Result<Option<Message>>
where
    O: RuntimeOutput,
    E: RuntimeEventSink,
{
    let pending = load_pending_tool_call(store, conversation_id, tool_call_id)?;
    let execution =
        prepare_pending_tool_execution(store, conversation_id, &pending, runtime.registry)?;
    let result = match execution {
        PendingToolExecution::Finished(result) => result,
        PendingToolExecution::Execute(attached_tool) => {
            execute_pending_tool_call(&pending, &attached_tool, runtime.registry).await?
        }
    };
    let message_id = store_pending_tool_result(store, conversation_id, &pending, &result)?;
    events.tool_result_saved(&message_id);

    continue_after_tool_result(output, events, store, conversation_id, runtime).await
}

/// Stores one denied tool result, emits it, and continues the runtime when
/// there are no later approvals waiting.
pub async fn deny_tool_turn<O, E>(
    output: &O,
    events: &E,
    store: &mut Store,
    conversation_id: &ConversationId,
    tool_call_id: &ToolCallId,
    runtime: RuntimeTurnConfig<'_>,
) -> Result<Option<Message>>
where
    O: RuntimeOutput,
    E: RuntimeEventSink,
{
    let pending = load_pending_tool_call(store, conversation_id, tool_call_id)?;
    let result = deny_pending_tool_call(&pending);
    let message_id = store_pending_tool_result(store, conversation_id, &pending, &result)?;
    events.tool_result_saved(&message_id);

    continue_after_tool_result(output, events, store, conversation_id, runtime).await
}

/// Continues after a stored tool result only when no manual approval remains.
async fn continue_after_tool_result<O, E>(
    output: &O,
    events: &E,
    store: &mut Store,
    conversation_id: &ConversationId,
    runtime: RuntimeTurnConfig<'_>,
) -> Result<Option<Message>>
where
    O: RuntimeOutput,
    E: RuntimeEventSink,
{
    if !pending_tool_approvals_with_registry(store, conversation_id, runtime.registry)?.is_empty() {
        return Ok(None);
    }

    query_runtime_turn(output, events, store, conversation_id, runtime)
        .await
        .map(Some)
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
#[path = "operation_tests.rs"]
mod tests;
