//! Wakeup input types.
//!
//! A wakeup is the reason Windie becomes active. Current wakeups are explicit
//! client actions such as query, continue, approve, deny, and stop. Future OS
//! wakeups such as schedules, file events, browser events, and system events
//! should enter through this same typed boundary before operation code creates
//! or resumes a session.

use crate::conversation::{ConversationId, MessageId, ToolCallId};
use crate::llm::{ModelName, ReasoningRequest};
use crate::session::SessionId;

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
/// Reason Windie should start or resume runtime activity.
pub enum Wakeup {
    Query(QueryWakeup),
    Continue(ContinueWakeup),
    ApproveTool(ToolDecisionWakeup),
    DenyTool(ToolDecisionWakeup),
    Stop(StopWakeup),
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
/// User query wakeup that should advance a conversation from a selected head.
pub struct QueryWakeup {
    pub conversation_id: ConversationId,
    pub head_message_id: Option<MessageId>,
    pub model: Option<ModelName>,
    pub reasoning: Option<ReasoningRequest>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Explicit continuation wakeup for an existing conversation head.
pub struct ContinueWakeup {
    pub conversation_id: ConversationId,
    pub head_message_id: Option<MessageId>,
    pub model: Option<ModelName>,
    pub reasoning: Option<ReasoningRequest>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
/// Tool approval or denial wakeup targeting one waiting session.
pub struct ToolDecisionWakeup {
    pub session_id: SessionId,
    pub tool_call_id: ToolCallId,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
/// Stop wakeup targeting one observable session.
pub struct StopWakeup {
    pub session_id: SessionId,
}
