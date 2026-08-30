//! Runtime flow coordination.
//!
//! This module exposes the runtime boundary while implementation details live
//! in turn and tool-execution modules.

pub(crate) mod context;
mod retry;
mod tool_execution;
mod turn;
pub(crate) mod wakeup;

pub(crate) use tool_execution::{
    PendingToolExecution, deny_pending_tool_call, execute_pending_tool_call_with_catalog,
    load_pending_tool_call_at_head, prepare_pending_tool_execution,
};
pub(crate) use turn::{advance_until_blocked, pending_approvals_at_head, prepare_head_turn};

#[cfg(test)]
pub(crate) use tool_execution::{
    PendingToolCall, active_tool_execution, execute_pending_tool_call,
};
#[cfg(test)]
pub(crate) use turn::advance_turn;

use anyhow::Result;

use crate::conversation::{
    ConversationId, MessageId, MessageMetadata, ToolCallId, UnsavedMessagePart,
};
use crate::llm::{PromptCacheRequest, ReasoningRequest};
use crate::plugin::PluginCatalog;
use crate::store::Store;
use crate::tool::ToolProviderRegistry;

#[cfg(test)]
use crate::llm::RuntimeLlm;
#[cfg(test)]
use crate::output::RuntimeOutput;
/// Required persistence boundary for every runtime-produced message.
///
/// Production session runners implement this contract through the atomic
/// session transaction. Requiring both methods prevents a caller from silently
/// falling back to a second persistence model.
pub(crate) trait RuntimeMessagePersistence {
    fn save_assistant_message(
        &self,
        store: &mut Store,
        conversation_id: &ConversationId,
        parent_message_id: Option<&MessageId>,
        content: &str,
        metadata: Option<&MessageMetadata>,
    ) -> Result<MessageId>;

    fn save_tool_result(
        &self,
        store: &mut Store,
        conversation_id: &ConversationId,
        parent_message_id: &MessageId,
        tool_call_id: &ToolCallId,
        content: &str,
        parts: &[UnsavedMessagePart],
    ) -> Result<MessageId>;
}

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
    pub(crate) plugin_catalog: Option<&'a PluginCatalog>,
    pub(crate) model_request: RuntimeModelRequest<'a>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RuntimeOutcome {
    Completed { head_message_id: Option<MessageId> },
    WaitingForApproval { head_message_id: MessageId },
}

#[cfg(test)]
mod tests;
