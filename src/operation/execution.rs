//! Shared query, approval-continuation, and runtime-turn operations.

use super::models::prompt_cache_request;
use super::*;

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
pub(crate) fn resolve_reasoning_request(
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
    let (_, message_id) = approve_tool_call_with_registry_and_message(
        store,
        conversation_id,
        tool_call_id,
        runtime.registry,
    )
    .await?;
    events.tool_result_saved(&message_id);

    continue_after_tool_result(output, events, store, conversation_id, &message_id, runtime).await
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
    let (_, message_id) = deny_tool_call_with_message(store, conversation_id, tool_call_id)?;
    events.tool_result_saved(&message_id);

    continue_after_tool_result(output, events, store, conversation_id, &message_id, runtime).await
}

/// Continues after a stored tool result only when no manual approval remains.
async fn continue_after_tool_result<O, E>(
    output: &O,
    events: &E,
    store: &mut Store,
    conversation_id: &ConversationId,
    result_message_id: &MessageId,
    runtime: RuntimeTurnConfig<'_>,
) -> Result<Option<Message>>
where
    O: RuntimeOutput,
    E: RuntimeEventSink,
{
    if store.active_message_id(conversation_id)?.as_ref() != Some(result_message_id) {
        return Ok(None);
    }
    if !pending_tool_approvals_with_registry(store, conversation_id, runtime.registry)?.is_empty() {
        return Ok(None);
    }

    query_runtime_turn(output, events, store, conversation_id, runtime)
        .await
        .map(Some)
}
