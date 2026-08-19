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
    store_pending_tool_result_at_head,
};
pub(crate) use turn::{advance_until_blocked, pending_approvals_at_head, prepare_head_turn};

#[cfg(test)]
pub(crate) use tool_execution::{
    PendingToolCall, active_tool_execution, execute_pending_tool_call,
};
#[cfg(test)]
pub(crate) use turn::advance_turn;

use crate::conversation::{ConversationId, MessageId};
use crate::llm::{PromptCacheRequest, ReasoningRequest};
use crate::plugin::PluginCatalog;
use crate::tool::ToolProviderRegistry;

#[cfg(test)]
use crate::conversation::Role;
#[cfg(test)]
use crate::error;
#[cfg(test)]
use crate::llm::RuntimeLlm;
#[cfg(test)]
use crate::output::RuntimeOutput;
#[cfg(test)]
use crate::store::Store;
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
