//! Runtime flow coordination.
//!
//! Coordinates runtime flows across output, store, context, and LLM components.
//! One-shot query primitives live here so CLI and future UI clients can reuse
//! the same execution path.

use std::collections::HashSet;

use anyhow::Result;

use crate::context::ContextBuilder;
use crate::conversation::{
    ConversationId, Message, MessageId, Role, ToolCall, ToolCallId, ToolSchemaName,
};
use crate::error;
use crate::llm::RuntimeLlm;
use crate::output::RuntimeOutput;
use crate::policy::{PolicyDecision, ToolPolicy};
use crate::store::Store;
use crate::tool::{AttachedTool, ToolApprovalRequest, ToolExecutionResult};
use crate::tool_provider::ToolProviderRegistry;

/// Receives durable runtime state changes during a query flow.
///
/// Runtime emits these events only after data has been persisted. HTTP clients
/// can stream them to a UI, while CLI callers use the no-op sink and keep the
/// existing blocking behavior.
pub(crate) trait RuntimeEventSink {
    fn assistant_message_saved(&self, _message_id: &MessageId) {}
    fn tool_result_saved(&self, _message_id: &MessageId) {}
}

/// Runtime event sink used by existing blocking callers.
pub(crate) struct NoopRuntimeEventSink;

impl RuntimeEventSink for NoopRuntimeEventSink {}

/// Runs one assistant inference turn and persists the assistant message.
///
/// This is intentionally one model request. The function prepares the active
/// path before building provider context so callers cannot accidentally query
/// while tool results are still pending. If the assistant returns tool-call
/// metadata, Windie stores that assistant message, records failed results for
/// policy-denied calls, and stops before any approval-required tool execution.
/// Callers compose approval steps explicitly with `pending_tool_approvals`,
/// `approve_tool_call` or `deny_tool_call`, and then another query turn.
#[cfg(test)]
pub(crate) async fn query_conversation_once<O, L>(
    output: &O,
    llm: &L,
    store: &mut Store,
    conversation_id: &ConversationId,
) -> Result<Message>
where
    O: RuntimeOutput,
    L: RuntimeLlm,
{
    let registry = ToolProviderRegistry::new();
    let events = NoopRuntimeEventSink;

    query_conversation_once_with_registry_and_events(
        output,
        llm,
        store,
        conversation_id,
        &registry,
        &events,
    )
    .await
}

/// Runs one assistant inference turn and emits persisted message events.
pub(crate) async fn query_conversation_once_with_registry_and_events<O, L, E>(
    output: &O,
    llm: &L,
    store: &mut Store,
    conversation_id: &ConversationId,
    registry: &ToolProviderRegistry,
    events: &E,
) -> Result<Message>
where
    O: RuntimeOutput,
    L: RuntimeLlm,
    E: RuntimeEventSink,
{
    prepare_query_turn_with_registry_and_events(store, conversation_id, registry, events)?;

    let parent_message_id = store.active_message_id(conversation_id)?;
    let model_messages = ContextBuilder::build(store, conversation_id)?;
    let tool_schemas = store.load_tool_schemas(conversation_id)?;

    output.start_assistant_message();
    let assistant_response = llm
        .stream(&model_messages, &tool_schemas, |text| {
            output.assistant_delta(text)
        })
        .await?;
    output.end_assistant_message();
    output.assistant_tool_calls(&assistant_response.metadata.tool_calls);

    let metadata = if assistant_response.metadata.is_empty() {
        None
    } else {
        Some(assistant_response.metadata)
    };
    let assistant_message_id = store.insert_message(
        conversation_id,
        parent_message_id.as_ref(),
        Role::Assistant,
        &assistant_response.content,
        metadata.as_ref(),
    )?;
    events.assistant_message_saved(&assistant_message_id);
    store_policy_denied_tool_results(store, conversation_id, registry, events)?;

    Ok(Message {
        id: Some(assistant_message_id),
        parent_message_id,
        role: Role::Assistant,
        content: assistant_response.content,
        parts: Vec::new(),
        metadata,
    })
}

/// Runs assistant turns while policy can resolve tool calls automatically.
///
/// Manual approval remains an explicit boundary: when policy asks for approval,
/// this function returns the assistant tool-call message that created the
/// pending approval. When policy denies or allows a pending call, runtime stores
/// the required `role: tool` result and continues toward the next assistant
/// response.
pub(crate) async fn query_conversation_resolving_automatic_tools<O, L>(
    output: &O,
    llm: &L,
    store: &mut Store,
    conversation_id: &ConversationId,
    registry: &ToolProviderRegistry,
) -> Result<Message>
where
    O: RuntimeOutput,
    L: RuntimeLlm,
{
    let events = NoopRuntimeEventSink;

    query_conversation_resolving_automatic_tools_with_events(
        output,
        llm,
        store,
        conversation_id,
        registry,
        &events,
    )
    .await
}

/// Runs assistant turns while emitting durable persisted-message events.
pub(crate) async fn query_conversation_resolving_automatic_tools_with_events<O, L, E>(
    output: &O,
    llm: &L,
    store: &mut Store,
    conversation_id: &ConversationId,
    registry: &ToolProviderRegistry,
    events: &E,
) -> Result<Message>
where
    O: RuntimeOutput,
    L: RuntimeLlm,
    E: RuntimeEventSink,
{
    let mut last_assistant_message = None;

    loop {
        match resolve_next_automatic_tool_call_with_registry_and_events(
            store,
            conversation_id,
            registry,
            events,
        )
        .await?
        {
            AutomaticToolResolution::Resolved => {}
            AutomaticToolResolution::WaitingForApproval => {
                if let Some(message) = last_assistant_message {
                    return Ok(message);
                }

                return query_conversation_once_with_registry_and_events(
                    output,
                    llm,
                    store,
                    conversation_id,
                    registry,
                    events,
                )
                .await;
            }
            AutomaticToolResolution::Idle => {
                let message = query_conversation_once_with_registry_and_events(
                    output,
                    llm,
                    store,
                    conversation_id,
                    registry,
                    events,
                )
                .await?;
                let has_tool_calls = message
                    .metadata
                    .as_ref()
                    .is_some_and(|metadata| !metadata.tool_calls.is_empty());

                if !has_tool_calls {
                    return Ok(message);
                }

                last_assistant_message = Some(message);
            }
        }
    }
}

/// Prepares the active path for a model query.
///
/// Policy-denied tool calls have no user decision to wait for, so Windie records
/// failed tool results for them before checking whether any approval-required
/// calls still block the query.
pub(crate) fn prepare_query_turn(
    store: &mut Store,
    conversation_id: &ConversationId,
) -> Result<()> {
    let registry = ToolProviderRegistry::new();

    prepare_query_turn_with_registry(store, conversation_id, &registry)
}

/// Prepares the active path for a model query using a caller-owned provider
/// registry.
pub(crate) fn prepare_query_turn_with_registry(
    store: &mut Store,
    conversation_id: &ConversationId,
    registry: &ToolProviderRegistry,
) -> Result<()> {
    let events = NoopRuntimeEventSink;

    prepare_query_turn_with_registry_and_events(store, conversation_id, registry, &events)
}

/// Prepares a model query and emits events for policy-denied tool results.
pub(crate) fn prepare_query_turn_with_registry_and_events<E>(
    store: &mut Store,
    conversation_id: &ConversationId,
    registry: &ToolProviderRegistry,
    events: &E,
) -> Result<()>
where
    E: RuntimeEventSink,
{
    store_policy_denied_tool_results(store, conversation_id, registry, events)?;
    validate_query_availability(store, conversation_id)
}

/// Rejects model queries while the active path is waiting for tool results.
///
/// OpenAI-compatible tool calls require the assistant tool-call message to be
/// followed by every requested `role: tool` result before the next assistant
/// turn. Windie's model context is the active path, so this check keeps that
/// path valid before any provider request is sent.
pub(crate) fn validate_query_availability(
    store: &Store,
    conversation_id: &ConversationId,
) -> Result<()> {
    let messages = store.load_active_path(conversation_id)?;
    let Some(execution) = active_tool_execution(&messages) else {
        return Ok(());
    };
    let Some(tool_call) = execution.next_pending_tool_call() else {
        return Ok(());
    };

    Err(error::invalid_request(format!(
        "tool call requires result before query: {}",
        tool_call.id
    )))
}

/// Lists the next pending active-path tool call requiring approval.
///
/// Windie exposes only the next pending tool call in assistant metadata
/// order. This preserves the linear active path shape that the model sees:
/// assistant tool-call message, first tool result, second tool result, and so
/// on.
pub(crate) fn pending_tool_approvals(
    store: &Store,
    conversation_id: &ConversationId,
) -> Result<Vec<ToolApprovalRequest>> {
    let registry = ToolProviderRegistry::new();

    pending_tool_approvals_with_registry(store, conversation_id, &registry)
}

/// Lists the next pending active-path tool call requiring approval using a
/// caller-owned provider registry.
pub(crate) fn pending_tool_approvals_with_registry(
    store: &Store,
    conversation_id: &ConversationId,
    registry: &ToolProviderRegistry,
) -> Result<Vec<ToolApprovalRequest>> {
    let messages = store.load_active_path(conversation_id)?;
    let Some(execution) = active_tool_execution(&messages) else {
        return Ok(Vec::new());
    };
    let Some(tool_call) = execution.next_pending_tool_call().cloned() else {
        return Ok(Vec::new());
    };
    let policy = ToolPolicy;
    let attached_tool = load_attached_tool_for_call(store, conversation_id, &tool_call)?;
    let approval_mode = store.tool_approval_mode(conversation_id)?;

    if let PolicyDecision::Ask { reason } = policy.decide(
        &tool_call,
        attached_tool.as_ref(),
        attached_tool_can_execute(registry, attached_tool.as_ref()),
        approval_mode,
    ) {
        return Ok(vec![ToolApprovalRequest {
            assistant_message_id: execution.assistant_message_id,
            tool_call,
            reason,
        }]);
    }

    Ok(Vec::new())
}

/// Stores failed tool results for pending calls that policy refuses to run.
///
/// Policy denial is not a user decision, but the model-facing protocol still
/// requires a `role: tool` result before the next assistant turn. This helper
/// advances through denied calls in metadata order and stops at the first call
/// that needs explicit approval.
fn store_policy_denied_tool_results(
    store: &mut Store,
    conversation_id: &ConversationId,
    registry: &ToolProviderRegistry,
    events: &impl RuntimeEventSink,
) -> Result<()> {
    let policy = ToolPolicy;

    loop {
        let messages = store.load_active_path(conversation_id)?;
        let Some(execution) = active_tool_execution(&messages) else {
            return Ok(());
        };
        let Some(tool_call) = execution.next_pending_tool_call().cloned() else {
            return Ok(());
        };
        let attached_tool = load_attached_tool_for_call(store, conversation_id, &tool_call)?;
        let approval_mode = store.tool_approval_mode(conversation_id)?;

        let PolicyDecision::Deny { reason } = policy.decide(
            &tool_call,
            attached_tool.as_ref(),
            attached_tool_can_execute(registry, attached_tool.as_ref()),
            approval_mode,
        ) else {
            return Ok(());
        };
        let pending = PendingToolCall {
            result_parent_message_id: execution.result_parent_message_id,
            tool_call,
        };
        let result = ToolExecutionResult::failure(
            pending.tool_call.id.clone(),
            pending.tool_call.name(),
            reason,
        );
        let message_id = store_pending_tool_result(store, conversation_id, &pending, &result)?;
        events.tool_result_saved(&message_id);
    }
}

/// Result of trying to resolve one pending tool call without user input.
enum AutomaticToolResolution {
    Idle,
    WaitingForApproval,
    Resolved,
}

/// Resolves one pending tool call and emits an event for the stored result.
///
/// Denied calls become failed `role: tool` results. Auto-approved attached
/// calls execute through the provider registry and then become normal tool
/// results. Approval-required calls are left untouched so clients can show the
/// approval request.
async fn resolve_next_automatic_tool_call_with_registry_and_events(
    store: &mut Store,
    conversation_id: &ConversationId,
    registry: &ToolProviderRegistry,
    events: &impl RuntimeEventSink,
) -> Result<AutomaticToolResolution> {
    let messages = store.load_active_path(conversation_id)?;
    let Some(execution) = active_tool_execution(&messages) else {
        return Ok(AutomaticToolResolution::Idle);
    };
    let Some(tool_call) = execution.next_pending_tool_call().cloned() else {
        return Ok(AutomaticToolResolution::Idle);
    };

    let pending = PendingToolCall {
        result_parent_message_id: execution.result_parent_message_id,
        tool_call,
    };
    let policy = ToolPolicy;
    let attached_tool = load_attached_tool_for_call(store, conversation_id, &pending.tool_call)?;
    let approval_mode = store.tool_approval_mode(conversation_id)?;
    let result = match policy.decide(
        &pending.tool_call,
        attached_tool.as_ref(),
        attached_tool_can_execute(registry, attached_tool.as_ref()),
        approval_mode,
    ) {
        PolicyDecision::Deny { reason } => ToolExecutionResult::failure(
            pending.tool_call.id.clone(),
            pending.tool_call.name(),
            reason,
        ),
        PolicyDecision::Allow => {
            execute_provider_tool_call(&pending, attached_tool.as_ref(), registry).await?
        }
        PolicyDecision::Ask { .. } => return Ok(AutomaticToolResolution::WaitingForApproval),
    };

    let message_id = store_pending_tool_result(store, conversation_id, &pending, &result)?;
    events.tool_result_saved(&message_id);

    Ok(AutomaticToolResolution::Resolved)
}

/// One pending tool call plus the message that should parent its result.
pub(crate) struct PendingToolCall {
    pub(crate) result_parent_message_id: MessageId,
    pub(crate) tool_call: ToolCall,
}

/// Prepared result of policy evaluation for one pending tool call.
pub(crate) enum PendingToolExecution {
    Finished(ToolExecutionResult),
    Execute(AttachedTool),
}

/// Active-path state for the latest assistant tool execution.
struct ActiveToolExecution {
    assistant_message_id: MessageId,
    result_parent_message_id: MessageId,
    requested_tool_calls: Vec<ToolCall>,
    resolved_tool_call_ids: HashSet<String>,
}

impl ActiveToolExecution {
    /// Returns the first requested tool call that has no active-path result.
    fn next_pending_tool_call(&self) -> Option<&ToolCall> {
        self.requested_tool_calls
            .iter()
            .find(|tool_call| !self.resolved_tool_call_ids.contains(tool_call.id.as_str()))
    }

    /// Returns whether this assistant requested the given provider tool-call ID.
    fn has_requested_tool_call(&self, tool_call_id: &ToolCallId) -> bool {
        self.requested_tool_calls
            .iter()
            .any(|tool_call| &tool_call.id == tool_call_id)
    }

    /// Returns whether the given provider tool-call ID already has a result.
    fn has_tool_result(&self, tool_call_id: &ToolCallId) -> bool {
        self.resolved_tool_call_ids.contains(tool_call_id.as_str())
    }
}

/// Finds the latest assistant tool execution on the active path.
///
/// Only contiguous `role: tool` messages after that assistant are treated as
/// results for that execution. If all calls have results, callers may safely
/// query the model again and append the next assistant message.
fn active_tool_execution(messages: &[Message]) -> Option<ActiveToolExecution> {
    let (assistant_index, assistant) = messages.iter().enumerate().rev().find(|(_, message)| {
        message.role == Role::Assistant
            && message
                .metadata
                .as_ref()
                .is_some_and(|metadata| !metadata.tool_calls.is_empty())
    })?;
    let assistant_message_id = assistant.id.as_ref()?.clone();
    let requested_tool_calls = assistant.metadata.as_ref()?.tool_calls.clone();
    let requested_tool_call_ids = requested_tool_calls
        .iter()
        .map(|tool_call| tool_call.id.as_str().to_string())
        .collect::<HashSet<_>>();
    let mut result_parent_message_id = assistant_message_id.clone();
    let mut resolved_tool_call_ids = HashSet::new();

    for message in &messages[assistant_index + 1..] {
        if message.role != Role::Tool {
            break;
        }
        let Some(message_id) = message.id.as_ref() else {
            continue;
        };
        let Some(tool_call_id) = message
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.tool_call_id.as_ref())
        else {
            continue;
        };
        if requested_tool_call_ids.contains(tool_call_id.as_str()) {
            resolved_tool_call_ids.insert(tool_call_id.as_str().to_string());
            result_parent_message_id = message_id.clone();
        }
    }

    Some(ActiveToolExecution {
        assistant_message_id,
        result_parent_message_id,
        requested_tool_calls,
        resolved_tool_call_ids,
    })
}

/// Executes one approved pending tool call and stores its result.
pub(crate) async fn approve_tool_call(
    store: &mut Store,
    conversation_id: &ConversationId,
    tool_call_id: &ToolCallId,
) -> Result<ToolExecutionResult> {
    let registry = ToolProviderRegistry::new();

    approve_tool_call_with_registry(store, conversation_id, tool_call_id, &registry).await
}

/// Executes one approved pending tool call through a caller-owned provider
/// registry and stores its result.
///
/// Long-lived clients such as the API server pass a registry configured for
/// persistent MCP sessions. One-shot CLI commands use `approve_tool_call`,
/// which builds the default short-lived registry.
pub(crate) async fn approve_tool_call_with_registry(
    store: &mut Store,
    conversation_id: &ConversationId,
    tool_call_id: &ToolCallId,
    registry: &ToolProviderRegistry,
) -> Result<ToolExecutionResult> {
    let pending = load_pending_tool_call(store, conversation_id, tool_call_id)?;
    let execution = prepare_pending_tool_execution(store, conversation_id, &pending, registry)?;
    let result = match execution {
        PendingToolExecution::Finished(result) => result,
        PendingToolExecution::Execute(attached_tool) => {
            execute_pending_tool_call(&pending, &attached_tool, registry).await?
        }
    };
    store_pending_tool_result(store, conversation_id, &pending, &result)?;

    Ok(result)
}

/// Evaluates policy and provider availability for one pending tool call.
///
/// This stays synchronous so SQLite store references never cross an async
/// provider boundary. If policy denies the call, the returned execution is a
/// finished failed result. If policy allows or asks, the caller receives the
/// attached provider mapping needed for execution.
pub(crate) fn prepare_pending_tool_execution(
    store: &Store,
    conversation_id: &ConversationId,
    pending: &PendingToolCall,
    registry: &ToolProviderRegistry,
) -> Result<PendingToolExecution> {
    let policy = ToolPolicy;
    let attached_tool = load_attached_tool_for_call(store, conversation_id, &pending.tool_call)?;
    let approval_mode = store.tool_approval_mode(conversation_id)?;

    match policy.decide(
        &pending.tool_call,
        attached_tool.as_ref(),
        attached_tool_can_execute(registry, attached_tool.as_ref()),
        approval_mode,
    ) {
        PolicyDecision::Deny { reason } => Ok(PendingToolExecution::Finished(
            ToolExecutionResult::failure(
                pending.tool_call.id.clone(),
                pending.tool_call.name(),
                reason,
            ),
        )),
        PolicyDecision::Allow | PolicyDecision::Ask { .. } => {
            let Some(attached_tool) = attached_tool else {
                return Err(error::invalid_request(format!(
                    "Tool is not attached: {}",
                    pending.tool_call.name()
                )));
            };
            Ok(PendingToolExecution::Execute(attached_tool))
        }
    }
}

/// Executes one prepared pending tool call through its attached provider.
pub(crate) async fn execute_pending_tool_call(
    pending: &PendingToolCall,
    attached_tool: &AttachedTool,
    registry: &ToolProviderRegistry,
) -> Result<ToolExecutionResult> {
    registry.call_tool(attached_tool, &pending.tool_call).await
}

/// Executes one pending tool call through its attached provider mapping.
async fn execute_provider_tool_call(
    pending: &PendingToolCall,
    attached_tool: Option<&AttachedTool>,
    registry: &ToolProviderRegistry,
) -> Result<ToolExecutionResult> {
    let Some(attached_tool) = attached_tool else {
        return Err(error::invalid_request(format!(
            "Tool is not attached: {}",
            pending.tool_call.name()
        )));
    };

    execute_pending_tool_call(pending, attached_tool, registry).await
}

/// Stores an explicit rejection for one pending tool call.
pub(crate) fn deny_tool_call(
    store: &mut Store,
    conversation_id: &ConversationId,
    tool_call_id: &ToolCallId,
) -> Result<ToolExecutionResult> {
    let pending = load_pending_tool_call(store, conversation_id, tool_call_id)?;
    let result = deny_pending_tool_call(&pending);
    store_pending_tool_result(store, conversation_id, &pending, &result)?;

    Ok(result)
}

/// Builds the failed result for an explicit user denial.
pub(crate) fn deny_pending_tool_call(pending: &PendingToolCall) -> ToolExecutionResult {
    ToolExecutionResult::failure(
        pending.tool_call.id.clone(),
        pending.tool_call.name(),
        "tool call rejected by user",
    )
}

/// Finds one pending tool call by provider tool-call ID.
pub(crate) fn load_pending_tool_call(
    store: &Store,
    conversation_id: &ConversationId,
    tool_call_id: &ToolCallId,
) -> Result<PendingToolCall> {
    let messages = store.load_active_path(conversation_id)?;
    let Some(execution) = active_tool_execution(&messages) else {
        return Err(error::not_found(format!(
            "pending tool call does not exist: {tool_call_id}"
        )));
    };
    if execution.has_tool_result(tool_call_id) {
        return Err(error::invalid_request(format!(
            "tool call already has a result: {tool_call_id}"
        )));
    }
    let Some(next_tool_call) = execution.next_pending_tool_call().cloned() else {
        return Err(error::not_found(format!(
            "pending tool call does not exist: {tool_call_id}"
        )));
    };
    if next_tool_call.id != *tool_call_id {
        if execution.has_requested_tool_call(tool_call_id) {
            return Err(error::invalid_request(format!(
                "tool call must be resolved after previous tool call: {}",
                next_tool_call.id
            )));
        }

        return Err(error::not_found(format!(
            "pending tool call does not exist: {tool_call_id}"
        )));
    }

    Ok(PendingToolCall {
        result_parent_message_id: execution.result_parent_message_id,
        tool_call: next_tool_call,
    })
}

/// Loads the attached tool matching one model-requested function name.
fn load_attached_tool_for_call(
    store: &Store,
    conversation_id: &ConversationId,
    tool_call: &ToolCall,
) -> Result<Option<AttachedTool>> {
    store.load_attached_tool(conversation_id, &ToolSchemaName::new(tool_call.name()))
}

/// Returns whether a loaded attached tool has an executor in the current
/// provider registry.
fn attached_tool_can_execute(
    registry: &ToolProviderRegistry,
    attached_tool: Option<&AttachedTool>,
) -> bool {
    attached_tool.is_some_and(|attached_tool| registry.can_execute(attached_tool))
}

/// Saves one tool execution result as a `role: tool` child message.
pub(crate) fn store_pending_tool_result(
    store: &mut Store,
    conversation_id: &ConversationId,
    pending: &PendingToolCall,
    result: &ToolExecutionResult,
) -> Result<MessageId> {
    if result.parts.is_empty() {
        store.insert_tool_result_message(
            conversation_id,
            &pending.result_parent_message_id,
            &result.tool_call_id,
            &result.content,
        )
    } else {
        store.insert_tool_result_message_with_parts(
            conversation_id,
            &pending.result_parent_message_id,
            &result.tool_call_id,
            &result.content,
            &result.parts,
        )
    }
}

#[allow(dead_code)]
#[cfg(test)]
#[path = "runtime_tests.rs"]
mod tests;
