//! Wakeup input types.
//!
//! A wakeup is an event that resumes runtime work. Sessions are created as
//! selectable branches inside a conversation and started explicitly, so the
//! current wakeups are session-targeted approval decisions. Future OS wakeups
//! such as schedules, file events, browser events, and system events should
//! enter through this typed boundary before operation code resumes a session.

use crate::conversation::ToolCallId;
use crate::session::SessionId;

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
/// Reason Windie should resume runtime activity on a durable session.
pub enum Wakeup {
    ApproveTool(ToolDecisionWakeup),
    DenyTool(ToolDecisionWakeup),
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
/// Tool approval or denial wakeup targeting one waiting session.
pub struct ToolDecisionWakeup {
    pub session_id: SessionId,
    pub tool_call_id: ToolCallId,
}
