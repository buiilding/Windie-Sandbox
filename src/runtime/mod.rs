//! Runtime flow coordination.
//!
//! Coordinates runtime flows across output, store, context, and LLM components.
//! Tree-wide: system prompt and tool schemas are conversation-wide, same for any head.

use std::collections::HashSet;

use anyhow::Result;

use serde_json::Value;

use crate::context::{ContextBuilder, ModelContext};
use crate::conversation::{ConversationId, Message, MessageId, Role, ToolCall, ToolCallId};
use crate::error;
use crate::llm::{LlmStreamEvent, PromptCacheRequest, ReasoningRequest, RuntimeLlm};
use crate::output::RuntimeOutput;
use crate::store::Store;
use crate::tool::{
    AttachedTool, PolicyDecision, ToolApprovalRequest, ToolExecutionResult, ToolPolicy,
    ToolProviderKind, ToolSchemaName,
};
use crate::tool_provider::{
    ATTACH_PROVIDER_TOOL_NAME, BUILTIN_PROVIDER_ID, LIST_PROVIDERS_TOOL_NAME, ToolProviderRegistry,
};

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

    let model_context = build_model_context(
        store,
        input.conversation_id,
        head_message_id.as_ref(),
        input.tools,
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
    let attached_tool =
        load_attached_tool_for_call(store, input.conversation_id, &tool_call, input.tools)?;
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
        let attached_tool = load_attached_tool_for_call(store, conversation_id, &tool_call, tools)?;
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
    let attached_tool =
        load_attached_tool_for_call(store, conversation_id, &pending.tool_call, tools)?;
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
            execute_provider_tool_call(
                store,
                conversation_id,
                &pending,
                attached_tool.as_ref(),
                tools,
            )
            .await?
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
    let attached_tool =
        load_attached_tool_for_call(store, conversation_id, &pending.tool_call, registry)?;
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
    store: &mut Store,
    conversation_id: &ConversationId,
    pending: &PendingToolCall,
    attached_tool: &AttachedTool,
    registry: &ToolProviderRegistry,
) -> Result<ToolExecutionResult> {
    if attached_tool.provider.kind == ToolProviderKind::Builtin {
        return execute_builtin_tool_call(store, conversation_id, pending, attached_tool, registry);
    }

    registry.call_tool(attached_tool, &pending.tool_call).await
}

async fn execute_provider_tool_call(
    store: &mut Store,
    conversation_id: &ConversationId,
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

    execute_pending_tool_call(store, conversation_id, pending, attached_tool, registry).await
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
    registry: &ToolProviderRegistry,
) -> Result<Option<AttachedTool>> {
    let schema_name = ToolSchemaName::new(tool_call.name());
    if let Some(attached_tool) = store.load_attached_tool(conversation_id, &schema_name)? {
        return Ok(Some(attached_tool));
    }

    Ok(registry
        .builtin_tool(&schema_name)
        .map(|definition| definition.attached_tool()))
}

fn attached_tool_can_execute(
    store: &Store,
    registry: &ToolProviderRegistry,
    attached_tool: Option<&AttachedTool>,
) -> bool {
    attached_tool.is_some_and(|attached_tool| {
        if attached_tool.provider.kind == ToolProviderKind::Builtin {
            return registry.can_execute(attached_tool);
        }

        store
            .provider_is_enabled(&attached_tool.provider.provider_id)
            .unwrap_or(false)
            && registry.can_execute(attached_tool)
    })
}

/// Builds runtime model context and adds Windie's implicit control tools.
///
/// Built-in tools are intentionally added only on the model-facing runtime
/// path. They do not enter conversation inspection or conversation tool-schema
/// persistence, so clients cannot detach or mistake them for providers.
fn build_model_context(
    store: &Store,
    conversation_id: &ConversationId,
    head_message_id: Option<&MessageId>,
    registry: &ToolProviderRegistry,
) -> Result<ModelContext> {
    let mut context = ContextBuilder::build_model_context(store, conversation_id, head_message_id)?;
    let mut names = context
        .tool_schemas
        .iter()
        .map(|tool| tool.name.as_str().to_string())
        .collect::<HashSet<_>>();

    for definition in registry.builtin_tools() {
        if names.insert(definition.schema_name.as_str().to_string()) {
            context
                .tool_schemas
                .push(definition.attached_tool().schema());
        }
    }

    Ok(context)
}

/// Executes one Windie-owned control tool and returns its compact model result.
fn execute_builtin_tool_call(
    store: &mut Store,
    conversation_id: &ConversationId,
    pending: &PendingToolCall,
    attached_tool: &AttachedTool,
    registry: &ToolProviderRegistry,
) -> Result<ToolExecutionResult> {
    if attached_tool.provider.provider_id.as_str() != BUILTIN_PROVIDER_ID {
        return Ok(ToolExecutionResult::failure(
            pending.tool_call.id.clone(),
            pending.tool_call.name(),
            "unknown built-in tool",
        ));
    }

    match attached_tool.provider.tool_name.as_str() {
        LIST_PROVIDERS_TOOL_NAME => Ok(ToolExecutionResult {
            tool_call_id: pending.tool_call.id.clone(),
            tool_name: pending.tool_call.name().to_string(),
            content: list_attachable_providers(store, registry)?,
            parts: Vec::new(),
            success: true,
        }),
        ATTACH_PROVIDER_TOOL_NAME => {
            let arguments = match serde_json::from_str::<Value>(pending.tool_call.arguments()) {
                Ok(arguments) => arguments,
                Err(error) => {
                    return Ok(ToolExecutionResult::failure(
                        pending.tool_call.id.clone(),
                        pending.tool_call.name(),
                        format!("invalid tool arguments: {error}"),
                    ));
                }
            };
            let Some(provider_id) = arguments.get("provider_id").and_then(Value::as_str) else {
                return Ok(ToolExecutionResult::failure(
                    pending.tool_call.id.clone(),
                    pending.tool_call.name(),
                    "provider_id is required",
                ));
            };

            let attachment = attach_provider_to_conversation(
                store,
                conversation_id,
                &crate::tool::ToolProviderId::new(provider_id),
                registry,
            );

            let Err(error) = attachment else {
                return Ok(ToolExecutionResult {
                    tool_call_id: pending.tool_call.id.clone(),
                    tool_name: pending.tool_call.name().to_string(),
                    content: "provider attached".to_string(),
                    parts: Vec::new(),
                    success: true,
                });
            };

            Ok(ToolExecutionResult::failure(
                pending.tool_call.id.clone(),
                pending.tool_call.name(),
                error.to_string(),
            ))
        }
        _ => Ok(ToolExecutionResult::failure(
            pending.tool_call.id.clone(),
            pending.tool_call.name(),
            "unknown built-in tool",
        )),
    }
}

/// Formats the attachable provider list exactly as model-facing plain text.
fn list_attachable_providers(store: &Store, registry: &ToolProviderRegistry) -> Result<String> {
    let mut lines = vec!["provider_id, description".to_string()];
    for manifest in registry.provider_manifests() {
        if !store.provider_is_enabled(&manifest.provider_id)? {
            continue;
        }
        let Some(status) = registry.provider_status(&manifest.provider_id) else {
            continue;
        };
        if status.available {
            lines.push(format!(
                "{}, {}",
                manifest.provider_id.as_str(),
                manifest.description
            ));
        }
    }

    Ok(lines.join("\n"))
}

/// Validates and attaches every tool from one enabled, healthy provider.
fn attach_provider_to_conversation(
    store: &mut Store,
    conversation_id: &ConversationId,
    provider_id: &crate::tool::ToolProviderId,
    registry: &ToolProviderRegistry,
) -> Result<()> {
    if registry.provider_manifest(provider_id).is_none() {
        return Err(error::not_found(format!(
            "provider does not exist: {provider_id}"
        )));
    }
    if !store.provider_is_enabled(provider_id)? {
        return Err(error::invalid_request(format!(
            "provider is not installed, enabled, and healthy: {provider_id}"
        )));
    }
    let Some(status) = registry.provider_status(provider_id) else {
        return Err(error::not_found(format!(
            "provider does not exist: {provider_id}"
        )));
    };
    if !status.available {
        return Err(error::invalid_request(format!(
            "provider is not healthy: {provider_id}"
        )));
    }

    let existing_names = store
        .load_attached_tools(conversation_id)?
        .into_iter()
        .map(|tool| tool.schema_name)
        .collect::<HashSet<_>>();
    let new_tools = registry
        .list_provider_tools(provider_id)?
        .into_iter()
        .filter(|tool| !existing_names.contains(&tool.schema_name))
        .map(|tool| tool.attached_tool())
        .collect::<Vec<_>>();
    store.insert_attached_tools(conversation_id, &new_tools)
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
