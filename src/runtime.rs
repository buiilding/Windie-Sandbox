//! Runtime flow coordination.
//!
//! Coordinates runtime flows across output, store, context, and LLM components.
//! One-shot query primitives live here so CLI and future UI clients can reuse
//! the same execution path.

use std::collections::HashSet;

use anyhow::{Context, Result, anyhow};

use crate::context::ContextBuilder;
use crate::conversation::{ConversationId, Message, MessageId, MessageMetadata, Role, ToolCallId};
use crate::llm::RuntimeLlm;
use crate::output::RuntimeOutput;
use crate::policy::{PolicyDecision, ToolPolicy};
use crate::shell::ShellExecutor;
use crate::store::Store;
use crate::tool::{ToolApprovalRequest, ToolExecutionResult};

/// Runs one assistant inference turn and persists the assistant message.
///
/// This is intentionally one model request. If the assistant returns tool-call
/// metadata, Windie stores that assistant message and stops. Callers compose the
/// next steps explicitly with `pending_tool_approvals`, `approve_tool_call` or
/// `deny_tool_call`, and then another call to this function.
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
    let parent_message_id = store
        .active_message_id(conversation_id)
        .context("failed to load active message")?;
    let model_messages =
        ContextBuilder::build(store, conversation_id).context("failed to build model context")?;
    let tool_schemas = store
        .load_tool_schemas(conversation_id)
        .context("failed to load tool schemas")?;

    output.start_assistant_message();
    let assistant_response = llm
        .stream(&model_messages, &tool_schemas, |text| {
            output.assistant_delta(text)
        })
        .await
        .context("failed to stream assistant response")?;
    output.end_assistant_message();
    output.assistant_tool_calls(&assistant_response.metadata.tool_calls);

    let metadata = if assistant_response.metadata.is_empty() {
        None
    } else {
        Some(assistant_response.metadata)
    };
    let assistant_message_id = store
        .insert_message(
            conversation_id,
            parent_message_id.as_ref(),
            Role::Assistant,
            &assistant_response.content,
            metadata.as_ref(),
        )
        .context("failed to save assistant message")?;

    Ok(Message {
        id: Some(assistant_message_id),
        parent_message_id,
        role: Role::Assistant,
        content: assistant_response.content,
        parts: Vec::new(),
        metadata,
    })
}

/// Lists unresolved active-path tool calls that require explicit user approval.
///
/// A tool call is unresolved when an assistant message requested it and no
/// active-path `role: tool` message references the same tool-call ID. Runtime
/// queries only see the active path, so approvals use the same boundary.
pub(crate) fn pending_tool_approvals(
    store: &Store,
    conversation_id: &ConversationId,
) -> Result<Vec<ToolApprovalRequest>> {
    let messages = store
        .load_active_path(conversation_id)
        .context("failed to load active path for approvals")?;
    let resolved_tool_call_ids = resolved_tool_call_ids(&messages);
    let policy = ToolPolicy;
    let mut approvals = Vec::new();

    for message in messages {
        if message.role != Role::Assistant {
            continue;
        }
        let Some(assistant_message_id) = message.id else {
            continue;
        };
        let Some(metadata) = message.metadata else {
            continue;
        };

        for tool_call in metadata.tool_calls {
            if resolved_tool_call_ids.contains(tool_call.id.as_str()) {
                continue;
            }
            if let PolicyDecision::Ask { reason } = policy.decide(&tool_call) {
                approvals.push(ToolApprovalRequest {
                    assistant_message_id: assistant_message_id.clone(),
                    tool_call,
                    reason,
                });
            }
        }
    }

    Ok(approvals)
}

/// Executes one approved pending tool call and stores its result.
pub(crate) async fn approve_tool_call(
    store: &mut Store,
    conversation_id: &ConversationId,
    tool_call_id: &ToolCallId,
) -> Result<ToolExecutionResult> {
    let pending = find_unresolved_tool_call(store, conversation_id, tool_call_id)?;
    let policy = ToolPolicy;
    let shell = ShellExecutor::default();
    let result = match policy.decide(&pending.tool_call) {
        PolicyDecision::Deny { reason } => ToolExecutionResult::failure(
            pending.tool_call.id.clone(),
            pending.tool_call.name(),
            reason,
        ),
        PolicyDecision::Ask { .. } => shell.execute_tool_call(&pending.tool_call).await,
    };

    store_tool_result(
        store,
        conversation_id,
        &pending.assistant_message_id,
        &result,
    )?;

    Ok(result)
}

/// Stores an explicit rejection for one pending tool call.
pub(crate) fn deny_tool_call(
    store: &mut Store,
    conversation_id: &ConversationId,
    tool_call_id: &ToolCallId,
) -> Result<ToolExecutionResult> {
    let pending = find_unresolved_tool_call(store, conversation_id, tool_call_id)?;
    let result = ToolExecutionResult::failure(
        pending.tool_call.id.clone(),
        pending.tool_call.name(),
        "tool call rejected by user",
    );

    store_tool_result(
        store,
        conversation_id,
        &pending.assistant_message_id,
        &result,
    )?;

    Ok(result)
}

/// Finds one unresolved tool call by provider tool-call ID.
fn find_unresolved_tool_call(
    store: &Store,
    conversation_id: &ConversationId,
    tool_call_id: &ToolCallId,
) -> Result<ToolApprovalRequest> {
    let messages = store
        .load_active_path(conversation_id)
        .context("failed to load active path for tool approval")?;
    let resolved_tool_call_ids = resolved_tool_call_ids(&messages);
    if resolved_tool_call_ids.contains(tool_call_id.as_str()) {
        return Err(anyhow!("tool call already has a result: {tool_call_id}"));
    }
    let policy = ToolPolicy;

    for message in messages {
        if message.role != Role::Assistant {
            continue;
        }
        let Some(assistant_message_id) = message.id else {
            continue;
        };
        let Some(metadata) = message.metadata else {
            continue;
        };

        for tool_call in metadata.tool_calls {
            if &tool_call.id != tool_call_id {
                continue;
            }
            let reason = match policy.decide(&tool_call) {
                PolicyDecision::Ask { reason } | PolicyDecision::Deny { reason } => reason,
            };

            return Ok(ToolApprovalRequest {
                assistant_message_id,
                tool_call,
                reason,
            });
        }
    }

    Err(anyhow!("pending tool call does not exist: {tool_call_id}"))
}

/// Saves one tool execution result as a `role: tool` child message.
fn store_tool_result(
    store: &mut Store,
    conversation_id: &ConversationId,
    parent_message_id: &MessageId,
    result: &ToolExecutionResult,
) -> Result<MessageId> {
    let metadata = MessageMetadata {
        tool_call_id: Some(result.tool_call_id.clone()),
        ..Default::default()
    };

    store
        .insert_message(
            conversation_id,
            Some(parent_message_id),
            Role::Tool,
            &result.content,
            Some(&metadata),
        )
        .context("failed to save tool result")
}

/// Collects tool-call IDs that already have stored tool result messages.
fn resolved_tool_call_ids(messages: &[Message]) -> HashSet<String> {
    messages
        .iter()
        .filter(|message| message.role == Role::Tool)
        .filter_map(|message| message.metadata.as_ref())
        .filter_map(|metadata| metadata.tool_call_id.as_ref())
        .map(|id| id.as_str().to_string())
        .collect()
}

#[allow(dead_code)]
#[cfg(test)]
#[path = "runtime_tests.rs"]
mod tests;
