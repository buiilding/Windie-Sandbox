//! Runtime flow coordination.
//!
//! Coordinates runtime flows across output, store, context, and LLM components.
//! Tree-wide: system prompt and tool schemas are conversation-wide, same for any head.

use std::collections::HashSet;

use anyhow::Result;

use crate::context::ContextBuilder;
use crate::conversation::{ConversationId, Message, MessageId, Role, ToolCall, ToolCallId};
use crate::error;
use crate::llm::{LlmStreamEvent, PromptCacheRequest, ReasoningRequest, RuntimeLlm};
use crate::output::RuntimeOutput;
use crate::store::Store;
use crate::tool::{
    AttachedTool, PolicyDecision, ToolApprovalRequest, ToolExecutionResult, ToolPolicy,
    ToolSchemaName,
};
use crate::tool_provider::ToolProviderRegistry;

pub(crate) trait RuntimeEventSink {
    fn assistant_message_saved(&self, _message_id: &MessageId) {}
    fn tool_result_saved(&self, _message_id: &MessageId) {}
}

pub(crate) struct NoopRuntimeEventSink;

impl RuntimeEventSink for NoopRuntimeEventSink {}

#[derive(Clone, Copy)]
pub(crate) struct RuntimeModelRequest<'a> {
    reasoning: Option<&'a ReasoningRequest>,
    prompt_cache: Option<&'a PromptCacheRequest>,
}

impl<'a> RuntimeModelRequest<'a> {
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
pub(crate) struct RuntimeInput<'a> {
    pub(crate) conversation_id: &'a ConversationId,
    pub(crate) head_message_id: Option<&'a MessageId>,
    pub(crate) tools: &'a ToolProviderRegistry,
    pub(crate) model_request: RuntimeModelRequest<'a>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RuntimeOutcome {
    Completed { head_message_id: Option<MessageId> },
    WaitingForApproval { head_message_id: MessageId },
}

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

    let model_context = ContextBuilder::build_model_context(
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
/// Tree-wide: tool lookup is conversation-wide.
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
    let attached_tool = load_attached_tool_for_call(store, input.conversation_id, &tool_call)?;
    let approval_mode = store.tool_approval_mode(input.conversation_id)?;

    if let PolicyDecision::Ask { reason } = policy.decide(
        &tool_call,
        attached_tool.as_ref(),
        attached_tool_can_execute(store, input.tools, attached_tool.as_ref()),
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
        let attached_tool = load_attached_tool_for_call(store, conversation_id, &tool_call)?;
        let approval_mode = store.tool_approval_mode(conversation_id)?;

        let PolicyDecision::Deny { reason } = policy.decide(
            &tool_call,
            attached_tool.as_ref(),
            attached_tool_can_execute(store, tools, attached_tool.as_ref()),
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

enum AutomaticToolResolution {
    Idle,
    WaitingForApproval,
    Resolved,
}

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
    let attached_tool = load_attached_tool_for_call(store, conversation_id, &pending.tool_call)?;
    let approval_mode = store.tool_approval_mode(conversation_id)?;
    let result = match policy.decide(
        &pending.tool_call,
        attached_tool.as_ref(),
        attached_tool_can_execute(store, tools, attached_tool.as_ref()),
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

pub(crate) struct PendingToolCall {
    pub(crate) result_parent_message_id: MessageId,
    pub(crate) tool_call: ToolCall,
}

pub(crate) enum PendingToolExecution {
    Finished(ToolExecutionResult),
    Execute(AttachedTool),
}

struct ActiveToolExecution {
    assistant_message_id: MessageId,
    result_parent_message_id: MessageId,
    requested_tool_calls: Vec<ToolCall>,
    resolved_tool_call_ids: HashSet<String>,
}

impl ActiveToolExecution {
    fn next_pending_tool_call(&self) -> Option<&ToolCall> {
        self.requested_tool_calls
            .iter()
            .find(|tool_call| !self.resolved_tool_call_ids.contains(tool_call.id.as_str()))
    }

    fn has_requested_tool_call(&self, tool_call_id: &ToolCallId) -> bool {
        self.requested_tool_calls
            .iter()
            .any(|tool_call| &tool_call.id == tool_call_id)
    }

    fn has_tool_result(&self, tool_call_id: &ToolCallId) -> bool {
        self.resolved_tool_call_ids.contains(tool_call_id.as_str())
    }
}

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

/// Tree-wide: tool lookup ignores head, same tool set for any branch.
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
        attached_tool_can_execute(store, registry, attached_tool.as_ref()),
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

pub(crate) async fn execute_pending_tool_call(
    pending: &PendingToolCall,
    attached_tool: &AttachedTool,
    registry: &ToolProviderRegistry,
) -> Result<ToolExecutionResult> {
    registry.call_tool(attached_tool, &pending.tool_call).await
}

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

pub(crate) fn deny_pending_tool_call(pending: &PendingToolCall) -> ToolExecutionResult {
    ToolExecutionResult::failure(
        pending.tool_call.id.clone(),
        pending.tool_call.name(),
        "tool call rejected by user",
    )
}

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

fn load_attached_tool_for_call(
    store: &Store,
    conversation_id: &ConversationId,
    tool_call: &ToolCall,
) -> Result<Option<AttachedTool>> {
    store.load_attached_tool(conversation_id, &ToolSchemaName::new(tool_call.name()))
}

fn attached_tool_can_execute(
    store: &Store,
    registry: &ToolProviderRegistry,
    attached_tool: Option<&AttachedTool>,
) -> bool {
    attached_tool.is_some_and(|attached_tool| {
        store
            .provider_is_enabled(&attached_tool.provider.provider_id)
            .unwrap_or(false)
            && registry.can_execute(attached_tool)
    })
}

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
mod tests;
