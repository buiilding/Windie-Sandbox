//! Shared query, approval-continuation, and runtime-turn operations.

use super::models::prompt_cache_request;
use super::{
    BaseUrl, BifrostClient, ConversationId, ExecutionCursor, GatewayUrl, Message, MessageId,
    ModelName, ReasoningRequest, Result, RunCancellation, RuntimeEventSink, RuntimeExecution,
    RuntimeOutput, RuntimeSnapshot, Store, ToolCallTarget, ToolExecutionResult,
    ToolProviderRegistry, approve_tool_call_with_snapshot, deny_tool_call_for_run, llm,
    pending_tool_approvals_on_path, query_conversation_resolving_automatic_tools,
    query_conversation_resolving_automatic_tools_with_events, require_gateway_running,
    runtime_snapshot,
};

#[cfg(test)]
use super::conversation_reasoning;

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
    let cursor = ExecutionCursor::capture(store, conversation_id)?;
    let (model, reasoning, snapshot) = capture_runtime_snapshot(
        store,
        conversation_id,
        runtime.model_override.clone(),
        runtime.reasoning.clone(),
    )?;
    require_gateway_running(runtime.gateway_url).await?;
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
        RuntimeExecution::new(
            runtime.run_id,
            &runtime.cancellation,
            &snapshot,
            cursor,
            reasoning.as_ref(),
            prompt_cache.as_ref(),
        ),
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn query_runtime_turn_from_snapshot<O, E>(
    output: &O,
    events: &E,
    store: &mut Store,
    conversation_id: &ConversationId,
    cursor: ExecutionCursor,
    runtime: RuntimeTurnConfig<'_>,
    model: ModelName,
    reasoning: Option<ReasoningRequest>,
    snapshot: &RuntimeSnapshot,
) -> Result<Message>
where
    O: RuntimeOutput,
    E: RuntimeEventSink,
{
    require_gateway_running(runtime.gateway_url).await?;
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
        RuntimeExecution::new(
            runtime.run_id,
            &runtime.cancellation,
            snapshot,
            cursor,
            reasoning.as_ref(),
            prompt_cache.as_ref(),
        ),
    )
    .await
}

/// Provider/runtime inputs needed to execute one model-backed runtime turn.
///
/// Query, approval, and denial flows share these values. Grouping them keeps
/// call sites explicit without growing long parameter lists.
pub struct RuntimeTurnConfig<'a> {
    run_id: &'a str,
    cancellation: RunCancellation,
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
        run_id: &'a str,
        cancellation: RunCancellation,
        gateway_url: GatewayUrl,
        base_url: BaseUrl,
        model_override: Option<ModelName>,
        reasoning: Option<ReasoningRequest>,
        registry: &'a ToolProviderRegistry,
    ) -> Self {
        Self {
            run_id,
            cancellation,
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
    runtime.cancellation.check()?;
    let cursor = ExecutionCursor::capture(store, conversation_id)?;
    let (model, reasoning, snapshot) = capture_runtime_snapshot(
        store,
        conversation_id,
        runtime.model_override.clone(),
        runtime.reasoning.clone(),
    )?;
    query_runtime_turn_from_snapshot(
        output,
        events,
        store,
        conversation_id,
        cursor,
        runtime,
        model,
        reasoning,
        &snapshot,
    )
    .await
}

pub(crate) fn capture_runtime_snapshot(
    store: &Store,
    conversation_id: &ConversationId,
    model_override: Option<ModelName>,
    reasoning_override: Option<ReasoningRequest>,
) -> Result<(ModelName, Option<ReasoningRequest>, RuntimeSnapshot)> {
    let configuration = store.load_runtime_configuration(conversation_id)?;
    let model = model_override.unwrap_or_else(|| ModelName::new(configuration.model));
    let reasoning = reasoning_override.or_else(|| {
        configuration
            .reasoning_effort
            .map(|effort| ReasoningRequest {
                effort: Some(effort),
                summary: None,
            })
    });
    let snapshot = RuntimeSnapshot {
        system_prompt: configuration.system_prompt,
        compaction: configuration.compaction,
        approval_mode: configuration.tool_approval_mode,
        attached_tools: configuration.attached_tools,
    };

    Ok((model, reasoning, snapshot))
}

/// Resolves the reasoning request for a runtime operation.
///
/// A caller-supplied request is a one-query override. When it is absent,
/// Windie uses the conversation-level persisted effort so CLI, API, and
/// inspector clients all flow through the same primitive.
#[cfg(test)]
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
    target: &ToolCallTarget,
    run_id: &str,
    cancellation: &RunCancellation,
) -> Result<ToolExecutionResult> {
    let registry = ToolProviderRegistry::new();
    let snapshot = runtime_snapshot(store, conversation_id)?;
    approve_tool_call_with_snapshot(
        store,
        conversation_id,
        target,
        &registry,
        run_id,
        cancellation,
        &snapshot,
    )
    .await
    .map(|(result, _)| result)
}

/// Executes one approved pending tool call with a caller-owned provider registry.
pub async fn approve_tool_with_registry(
    store: &mut Store,
    conversation_id: &ConversationId,
    target: &ToolCallTarget,
    registry: &ToolProviderRegistry,
    run_id: &str,
    cancellation: &RunCancellation,
) -> Result<ToolExecutionResult> {
    let snapshot = runtime_snapshot(store, conversation_id)?;
    approve_tool_call_with_snapshot(
        store,
        conversation_id,
        target,
        registry,
        run_id,
        cancellation,
        &snapshot,
    )
    .await
    .map(|(result, _)| result)
}

/// Persists a rejected result for one pending tool call.
pub fn deny_tool(
    store: &mut Store,
    conversation_id: &ConversationId,
    target: &ToolCallTarget,
    run_id: &str,
    cancellation: &RunCancellation,
) -> Result<ToolExecutionResult> {
    cancellation.check()?;
    deny_tool_call_for_run(store, conversation_id, target, run_id).map(|(result, _)| result)
}

/// Executes one approved tool call, emits its persisted result, and continues
/// the runtime when no later approval is waiting.
///
/// This is the client-facing approval behavior: approval resolves one pending
/// call and lets Windie advance if the run path is ready. Multi-tool turns
/// stop after the stored result when the next requested call still needs manual
/// approval.
pub async fn approve_tool_turn<O, E>(
    output: &O,
    events: &E,
    store: &mut Store,
    conversation_id: &ConversationId,
    target: &ToolCallTarget,
    runtime: RuntimeTurnConfig<'_>,
) -> Result<Option<Message>>
where
    O: RuntimeOutput,
    E: RuntimeEventSink,
{
    runtime.cancellation.check()?;
    let (model, reasoning, snapshot) = capture_runtime_snapshot(
        store,
        conversation_id,
        runtime.model_override.clone(),
        runtime.reasoning.clone(),
    )?;
    let (_, message_id) = approve_tool_call_with_snapshot(
        store,
        conversation_id,
        target,
        runtime.registry,
        runtime.run_id,
        &runtime.cancellation,
        &snapshot,
    )
    .await?;
    events.tool_result_saved(&message_id)?;
    let continuation = RuntimeContinuation {
        model,
        reasoning,
        snapshot,
    };

    continue_after_tool_result(
        output,
        events,
        store,
        conversation_id,
        &message_id,
        runtime,
        continuation,
    )
    .await
}

/// Stores one denied tool result, emits it, and continues the runtime when
/// there are no later approvals waiting.
pub async fn deny_tool_turn<O, E>(
    output: &O,
    events: &E,
    store: &mut Store,
    conversation_id: &ConversationId,
    target: &ToolCallTarget,
    runtime: RuntimeTurnConfig<'_>,
) -> Result<Option<Message>>
where
    O: RuntimeOutput,
    E: RuntimeEventSink,
{
    runtime.cancellation.check()?;
    let (model, reasoning, snapshot) = capture_runtime_snapshot(
        store,
        conversation_id,
        runtime.model_override.clone(),
        runtime.reasoning.clone(),
    )?;
    let (_, message_id) = deny_tool_call_for_run(store, conversation_id, target, runtime.run_id)?;
    events.tool_result_saved(&message_id)?;
    let continuation = RuntimeContinuation {
        model,
        reasoning,
        snapshot,
    };

    continue_after_tool_result(
        output,
        events,
        store,
        conversation_id,
        &message_id,
        runtime,
        continuation,
    )
    .await
}

struct RuntimeContinuation {
    model: ModelName,
    reasoning: Option<ReasoningRequest>,
    snapshot: RuntimeSnapshot,
}

/// Continues after a stored tool result only when no manual approval remains.
async fn continue_after_tool_result<O, E>(
    output: &O,
    events: &E,
    store: &mut Store,
    conversation_id: &ConversationId,
    result_message_id: &MessageId,
    runtime: RuntimeTurnConfig<'_>,
    continuation: RuntimeContinuation,
) -> Result<Option<Message>>
where
    O: RuntimeOutput,
    E: RuntimeEventSink,
{
    let cursor = ExecutionCursor::at(result_message_id.clone());
    if !pending_tool_approvals_on_path(
        store,
        conversation_id,
        runtime.registry,
        &continuation.snapshot,
        &cursor,
    )?
    .is_empty()
    {
        return Ok(None);
    }

    query_runtime_turn_from_snapshot(
        output,
        events,
        store,
        conversation_id,
        cursor,
        runtime,
        continuation.model,
        continuation.reasoning,
        &continuation.snapshot,
    )
    .await
    .map(Some)
}
