//! Runtime flow coordination.
//!
//! Coordinates runtime flows across output, store, context, and LLM components.
//! Session-owned execution advances from explicit message heads so clients do not
//! query from the mutable conversation active path.

use std::collections::HashSet;

use anyhow::Result;

use crate::context::ContextBuilder;
use crate::conversation::{
    ConversationId, Message, MessageId, Role, ToolCall, ToolCallId, ToolSchemaName,
};
use crate::error;
use crate::llm::{LlmStreamEvent, PromptCacheRequest, ReasoningRequest, RuntimeLlm};
use crate::output::RuntimeOutput;
use crate::policy::{PolicyDecision, ToolPolicy};
use crate::store::Store;
use crate::tool::{AttachedTool, ToolApprovalRequest, ToolExecutionResult};
use crate::tool_provider::ToolProviderRegistry;

/// Receives durable runtime state changes during run execution.
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

#[derive(Clone, Copy)]
/// Optional model-request controls used for one provider turn.
///
/// Runtime does not interpret these controls. It only carries them from the
/// operation layer to the LLM boundary so provider-specific serialization stays
/// in `llm.rs`.
pub(crate) struct RuntimeModelRequest<'a> {
    reasoning: Option<&'a ReasoningRequest>,
    prompt_cache: Option<&'a PromptCacheRequest>,
}

impl<'a> RuntimeModelRequest<'a> {
    /// Groups optional reasoning and prompt-cache controls for one query.
    pub(crate) fn new(
        reasoning: Option<&'a ReasoningRequest>,
        prompt_cache: Option<&'a PromptCacheRequest>,
    ) -> Self {
        Self {
            reasoning,
            prompt_cache,
        }
    }
}

#[derive(Clone, Copy)]
/// Inputs for one explicit-head runtime execution.
///
/// The head is captured by run admission. Runtime never reads the
/// conversation's active UI selection when this input is used.
pub(crate) struct RuntimeInput<'a> {
    pub(crate) conversation_id: &'a ConversationId,
    pub(crate) head_message_id: Option<&'a MessageId>,
    pub(crate) tools: &'a ToolProviderRegistry,
    pub(crate) model_request: RuntimeModelRequest<'a>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Result of advancing a runtime session.
pub(crate) enum RuntimeOutcome {
    Completed { head_message_id: Option<MessageId> },
    WaitingForApproval { head_message_id: MessageId },
}

/// Sessions one assistant inference turn from an explicit message head.
pub(crate) async fn advance_turn<O, L, E>(
    output: &O,
    llm: &L,
    store: &mut Store,
    input: RuntimeInput<'_>,
    events: &E,
) -> Result<Message>
where
    O: RuntimeOutput,
    L: RuntimeLlm,
    E: RuntimeEventSink,
{
    let mut head_message_id = input.head_message_id.cloned();
    prepare_head_turn(
        store,
        input.conversation_id,
        &mut head_message_id,
        input.tools,
        events,
    )?;

    let model_context = ContextBuilder::build_model_context_to_head(
        store,
        input.conversation_id,
        head_message_id.as_ref(),
    )?;

    output.start_assistant_message();
    let assistant_response = llm
        .stream(
            &model_context.messages,
            &model_context.tool_schemas,
            input.model_request.reasoning,
            input.model_request.prompt_cache,
            |event| match event {
                LlmStreamEvent::AssistantDelta(text) => output.assistant_delta(text),
                LlmStreamEvent::ReasoningDelta(text) => output.reasoning_delta(text),
                LlmStreamEvent::ToolCallDelta {
                    index,
                    id,
                    name,
                    arguments_delta,
                } => output.tool_call_delta(index, id, name, arguments_delta),
            },
        )
        .await?;
    output.end_assistant_message();
    output.assistant_tool_calls(&assistant_response.metadata.tool_calls);

    let metadata = if assistant_response.metadata.is_empty() {
        None
    } else {
        Some(assistant_response.metadata)
    };
    let assistant_message_id = store.insert_run_message(
        input.conversation_id,
        head_message_id.as_ref(),
        Role::Assistant,
        &assistant_response.content,
        metadata.as_ref(),
    )?;
    events.assistant_message_saved(&assistant_message_id);
    head_message_id = Some(assistant_message_id.clone());
    store_policy_denied_tool_results_at_head(
        store,
        input.conversation_id,
        &mut head_message_id,
        input.tools,
        events,
    )?;

    Ok(Message {
        id: Some(assistant_message_id),
        parent_message_id: input.head_message_id.cloned(),
        role: Role::Assistant,
        content: assistant_response.content,
        parts: Vec::new(),
        metadata,
    })
}

/// Sessions from an explicit head until completion or a manual approval boundary.
pub(crate) async fn advance_until_blocked<O, L, E>(
    output: &O,
    llm: &L,
    store: &mut Store,
    input: RuntimeInput<'_>,
    events: &E,
) -> Result<RuntimeOutcome>
where
    O: RuntimeOutput,
    L: RuntimeLlm,
    E: RuntimeEventSink,
{
    let mut head_message_id = input.head_message_id.cloned();

    loop {
        match resolve_next_automatic_tool_call_at_head(
            store,
            input.conversation_id,
            &mut head_message_id,
            input.tools,
            events,
        )
        .await?
        {
            AutomaticToolResolution::Resolved => {}
            AutomaticToolResolution::WaitingForApproval => {
                let Some(head_message_id) = head_message_id else {
                    return Ok(RuntimeOutcome::Completed {
                        head_message_id: None,
                    });
                };
                return Ok(RuntimeOutcome::WaitingForApproval { head_message_id });
            }
            AutomaticToolResolution::Idle => {
                let turn_input = RuntimeInput {
                    conversation_id: input.conversation_id,
                    head_message_id: head_message_id.as_ref(),
                    tools: input.tools,
                    model_request: input.model_request,
                };
                let message = advance_turn(output, llm, store, turn_input, events).await?;
                head_message_id = message.id.clone();
                let has_tool_calls = message
                    .metadata
                    .as_ref()
                    .is_some_and(|metadata| !metadata.tool_calls.is_empty());

                if !has_tool_calls {
                    return Ok(RuntimeOutcome::Completed { head_message_id });
                }
            }
        }
    }
}

/// Lists approval-required tool calls at an explicit runtime head.
pub(crate) fn pending_approvals_at_head(
    store: &Store,
    input: RuntimeInput<'_>,
) -> Result<Vec<ToolApprovalRequest>> {
    let messages = load_path_at_head(store, input.conversation_id, input.head_message_id)?;
    let Some(execution) = active_tool_execution(&messages) else {
        return Ok(Vec::new());
    };
    let Some(tool_call) = execution.next_pending_tool_call().cloned() else {
        return Ok(Vec::new());
    };
    let policy = ToolPolicy;
    let attached_tool = load_attached_tool_for_call(
        store,
        input.conversation_id,
        input.head_message_id,
        &tool_call,
    )?;
    let approval_mode = store.tool_approval_mode(input.conversation_id)?;

    if let PolicyDecision::Ask { reason } = policy.decide(
        &tool_call,
        attached_tool.as_ref(),
        attached_tool_can_execute(input.tools, attached_tool.as_ref()),
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

/// Prepares an explicit runtime head for a model request.
pub(crate) fn prepare_head_turn(
    store: &mut Store,
    conversation_id: &ConversationId,
    head_message_id: &mut Option<MessageId>,
    tools: &ToolProviderRegistry,
    events: &impl RuntimeEventSink,
) -> Result<()> {
    store_policy_denied_tool_results_at_head(
        store,
        conversation_id,
        head_message_id,
        tools,
        events,
    )?;
    validate_run_head_availability(store, conversation_id, head_message_id.as_ref())
}

/// Rejects provider queries while an explicit head is waiting for tool results.
fn validate_run_head_availability(
    store: &Store,
    conversation_id: &ConversationId,
    head_message_id: Option<&MessageId>,
) -> Result<()> {
    let messages = load_path_at_head(store, conversation_id, head_message_id)?;
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

/// Loads the root-to-head path for an explicit runtime head.
fn load_path_at_head(
    store: &Store,
    conversation_id: &ConversationId,
    head_message_id: Option<&MessageId>,
) -> Result<Vec<Message>> {
    match head_message_id {
        Some(message_id) => store.load_path_to_message(conversation_id, message_id),
        None => Ok(Vec::new()),
    }
}

/// Stores policy-denied tool results at an explicit runtime head.
fn store_policy_denied_tool_results_at_head(
    store: &mut Store,
    conversation_id: &ConversationId,
    head_message_id: &mut Option<MessageId>,
    tools: &ToolProviderRegistry,
    events: &impl RuntimeEventSink,
) -> Result<()> {
    let policy = ToolPolicy;

    loop {
        let messages = load_path_at_head(store, conversation_id, head_message_id.as_ref())?;
        let Some(execution) = active_tool_execution(&messages) else {
            return Ok(());
        };
        let Some(tool_call) = execution.next_pending_tool_call().cloned() else {
            return Ok(());
        };
        let attached_tool = load_attached_tool_for_call(
            store,
            conversation_id,
            head_message_id.as_ref(),
            &tool_call,
        )?;
        let approval_mode = store.tool_approval_mode(conversation_id)?;

        let PolicyDecision::Deny { reason } = policy.decide(
            &tool_call,
            attached_tool.as_ref(),
            attached_tool_can_execute(tools, attached_tool.as_ref()),
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
        let message_id =
            store_pending_tool_result_at_head(store, conversation_id, &pending, &result)?;
        *head_message_id = Some(message_id.clone());
        events.tool_result_saved(&message_id);
    }
}

/// Result of trying to resolve one pending tool call without user input.
enum AutomaticToolResolution {
    Idle,
    WaitingForApproval,
    Resolved,
}

/// Resolves one pending tool call at an explicit runtime head.
async fn resolve_next_automatic_tool_call_at_head(
    store: &mut Store,
    conversation_id: &ConversationId,
    head_message_id: &mut Option<MessageId>,
    tools: &ToolProviderRegistry,
    events: &impl RuntimeEventSink,
) -> Result<AutomaticToolResolution> {
    let messages = load_path_at_head(store, conversation_id, head_message_id.as_ref())?;
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
    let attached_tool = load_attached_tool_for_call(
        store,
        conversation_id,
        head_message_id.as_ref(),
        &pending.tool_call,
    )?;
    let approval_mode = store.tool_approval_mode(conversation_id)?;
    let result = match policy.decide(
        &pending.tool_call,
        attached_tool.as_ref(),
        attached_tool_can_execute(tools, attached_tool.as_ref()),
        approval_mode,
    ) {
        PolicyDecision::Deny { reason } => ToolExecutionResult::failure(
            pending.tool_call.id.clone(),
            pending.tool_call.name(),
            reason,
        ),
        PolicyDecision::Allow => {
            execute_provider_tool_call(&pending, attached_tool.as_ref(), tools).await?
        }
        PolicyDecision::Ask { .. } => return Ok(AutomaticToolResolution::WaitingForApproval),
    };

    let message_id = store_pending_tool_result_at_head(store, conversation_id, &pending, &result)?;
    *head_message_id = Some(message_id.clone());
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

/// Path state for the latest assistant tool execution.
struct ActiveToolExecution {
    assistant_message_id: MessageId,
    result_parent_message_id: MessageId,
    requested_tool_calls: Vec<ToolCall>,
    resolved_tool_call_ids: HashSet<String>,
}

impl ActiveToolExecution {
    /// Returns the first requested tool call that has no path result.
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

/// Finds the latest assistant tool execution on a message path.
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

/// Evaluates policy and provider availability for one pending tool call.
///
/// This stays synchronous so SQLite store references never cross an async
/// provider boundary. If policy denies the call, the returned execution is a
/// finished failed result. If policy allows or asks, the caller receives the
/// attached provider mapping needed for execution.
pub(crate) fn prepare_pending_tool_execution(
    store: &Store,
    conversation_id: &ConversationId,
    head_message_id: Option<&MessageId>,
    pending: &PendingToolCall,
    registry: &ToolProviderRegistry,
) -> Result<PendingToolExecution> {
    let policy = ToolPolicy;
    let attached_tool =
        load_attached_tool_for_call(store, conversation_id, head_message_id, &pending.tool_call)?;
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

/// Builds the failed result for an explicit user denial.
pub(crate) fn deny_pending_tool_call(pending: &PendingToolCall) -> ToolExecutionResult {
    ToolExecutionResult::failure(
        pending.tool_call.id.clone(),
        pending.tool_call.name(),
        "tool call rejected by user",
    )
}

/// Finds one pending tool call by provider tool-call ID at an explicit head.
pub(crate) fn load_pending_tool_call_at_head(
    store: &Store,
    conversation_id: &ConversationId,
    head_message_id: Option<&MessageId>,
    tool_call_id: &ToolCallId,
) -> Result<PendingToolCall> {
    let messages = load_path_at_head(store, conversation_id, head_message_id)?;
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
    head_message_id: Option<&MessageId>,
    tool_call: &ToolCall,
) -> Result<Option<AttachedTool>> {
    store.load_attached_tool_for_head(
        conversation_id,
        head_message_id,
        &ToolSchemaName::new(tool_call.name()),
    )
}

/// Returns whether a loaded attached tool has an executor in the current
/// provider registry.
fn attached_tool_can_execute(
    registry: &ToolProviderRegistry,
    attached_tool: Option<&AttachedTool>,
) -> bool {
    attached_tool.is_some_and(|attached_tool| registry.can_execute(attached_tool))
}

/// Saves one session-owned tool execution result without changing UI selection.
pub(crate) fn store_pending_tool_result_at_head(
    store: &mut Store,
    conversation_id: &ConversationId,
    pending: &PendingToolCall,
    result: &ToolExecutionResult,
) -> Result<MessageId> {
    if result.parts.is_empty() {
        store.insert_run_tool_result_message(
            conversation_id,
            &pending.result_parent_message_id,
            &result.tool_call_id,
            &result.content,
        )
    } else {
        store.insert_run_tool_result_message_with_parts(
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
