//! Wakeup input types.
//!
//! A wakeup is the reason Windie becomes active. Sessions are created as
//! selectable branches inside a conversation and started explicitly, so the
//! remaining wakeups are the session-targeted decisions: approve, deny, and
//! stop. Future OS wakeups such as schedules, file events, browser events, and
//! system events should enter through this same typed boundary before operation
//! code resumes a session.

use crate::conversation::ToolCallId;
use crate::session::SessionId;

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
/// Reason Windie should resume or stop runtime activity on a durable session.
pub enum Wakeup {
    ApproveTool(ToolDecisionWakeup),
    DenyTool(ToolDecisionWakeup),
    Stop(StopWakeup),
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
