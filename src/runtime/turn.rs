//! Runtime turn orchestration.
//!
//! This module prepares model context, advances assistant turns, and coordinates
//! automatic tool resolution until the session completes or needs approval.

use std::collections::HashSet;

use anyhow::Result;

use crate::context::{ContextBuilder, ModelContext};
use crate::conversation::{ConversationId, Message, MessageId, Role};
use crate::error;
use crate::llm::RuntimeLlm;
use crate::output::RuntimeOutput;
use crate::store::Store;
use crate::tool::{PolicyDecision, ToolApprovalRequest, ToolExecutionResult, ToolPolicy};
use crate::tool_provider::ToolProviderRegistry;

use super::retry::stream_with_retry;
use super::tool_execution::{
    AutomaticToolResolution, PendingToolCall, active_tool_execution, attached_tool_can_execute,
    load_attached_tool_for_call, resolve_next_automatic_tool_call_at_head,
    store_pending_tool_result_at_head,
};
use super::{RuntimeEventSink, RuntimeInput, RuntimeOutcome};

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

    let assistant_response =
        stream_with_retry(output, llm, &model_context, input.model_request).await?;
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

pub(crate) fn load_path_at_head(
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

/// Builds runtime model context and adds Windie's implicit control tools.
///
/// Built-in tools are intentionally added only on the model-facing runtime
/// path. They do not enter conversation inspection or conversation tool-schema
/// persistence, so clients cannot detach or mistake them for providers.
pub(crate) fn build_model_context(
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
