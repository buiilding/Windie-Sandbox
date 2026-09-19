//! Storage-independent progression of an assistant's ordered tool calls.
//!
//! Adapters supply a canonical selected path and loaded permission facts. This
//! module never loads storage, executes a provider, or grants device authority.

use std::collections::HashSet;

use crate::conversation::{Message, MessageId, Role, ToolCall, ToolCallId};
use crate::tool::{
    AttachedTool, PolicyDecision, ToolApprovalMode, ToolExecutionResult, ToolPolicy,
};

/// The next unresolved call and the exact parent for its durable result.
pub(crate) struct PendingToolCall {
    pub(crate) result_parent_message_id: MessageId,
    pub(crate) tool_call: ToolCall,
}

/// Loaded assistant group. Ordering follows metadata order, never tool names or
/// provider call IDs sorted independently of the request.
pub(crate) struct ActiveToolExecution {
    pub(crate) assistant_message_id: MessageId,
    pub(crate) result_parent_message_id: MessageId,
    requested_tool_calls: Vec<ToolCall>,
    resolved_tool_call_ids: HashSet<String>,
}

impl ActiveToolExecution {
    pub(crate) fn next_pending_tool_call(&self) -> Option<&ToolCall> {
        self.requested_tool_calls
            .iter()
            .find(|call| !self.resolved_tool_call_ids.contains(call.id.as_str()))
    }

    pub(crate) fn has_requested_tool_call(&self, id: &ToolCallId) -> bool {
        self.requested_tool_calls.iter().any(|call| &call.id == id)
    }

    pub(crate) fn has_tool_result(&self, id: &ToolCallId) -> bool {
        self.resolved_tool_call_ids.contains(id.as_str())
    }
}

/// Finds the same active tool group used by local approval and execution.
pub(crate) fn active_tool_execution(messages: &[Message]) -> Option<ActiveToolExecution> {
    let (assistant_index, assistant) = messages.iter().enumerate().rev().find(|(_, message)| {
        message.role == Role::Assistant && assistant_requires_tools(message)
    })?;
    let assistant_message_id = assistant.id.as_ref()?.clone();
    let requested_tool_calls = assistant.metadata.as_ref()?.tool_calls.clone();
    let requested_ids = requested_tool_calls
        .iter()
        .map(|call| call.id.as_str())
        .collect::<HashSet<_>>();
    let mut result_parent_message_id = assistant_message_id.clone();
    let mut resolved_tool_call_ids = HashSet::new();
    for message in &messages[assistant_index + 1..] {
        if message.role != Role::Tool {
            break;
        }
        let Some(message_id) = &message.id else {
            continue;
        };
        let Some(id) = message
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.tool_call_id.as_ref())
        else {
            continue;
        };
        if requested_ids.contains(id.as_str()) {
            resolved_tool_call_ids.insert(id.as_str().to_owned());
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

/// A saved model response ends a turn only when it requested no tools.
pub(crate) fn assistant_requires_tools(message: &Message) -> bool {
    message
        .metadata
        .as_ref()
        .is_some_and(|metadata| !metadata.tool_calls.is_empty())
}

/// Decisions shared by immediate local execution and durable hosted dispatch.
pub(crate) enum ToolAction {
    PersistDenied(ToolExecutionResult),
    RequestApproval { reason: String },
    Execute,
}

/// Applies the existing policy to loaded facts. Approval never overrides an
/// absent attachment or unavailable executor; callers revalidate on dispatch.
pub(crate) fn next_tool_action(
    call: &ToolCall,
    attached: Option<&AttachedTool>,
    executable: bool,
    approval_mode: ToolApprovalMode,
) -> ToolAction {
    match ToolPolicy.decide(call, attached, executable, approval_mode) {
        PolicyDecision::Allow => ToolAction::Execute,
        PolicyDecision::Ask { reason } => ToolAction::RequestApproval { reason },
        PolicyDecision::Deny { reason } => ToolAction::PersistDenied(ToolExecutionResult::failure(
            call.id.clone(),
            call.name(),
            reason,
        )),
    }
}
