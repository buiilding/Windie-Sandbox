//! Replayable session event types.

use serde::{Deserialize, Serialize};

use super::SessionId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Stable event kinds used by persistence, SSE names, and aggregate filters.
pub enum SessionEventKind {
    InputQueued,
    InputStarted,
    AssistantDelta,
    ReasoningDelta,
    ToolCallDelta,
    AssistantAttemptReset,
    AssistantMessageSaved,
    ToolResultSaved,
    WaitingForApproval,
    Completed,
    Failed,
    Cancelled,
}

impl SessionEventKind {
    /// Returns the storage and SSE name for this event kind.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InputQueued => "input_queued",
            Self::InputStarted => "input_started",
            Self::AssistantDelta => "assistant_delta",
            Self::ReasoningDelta => "reasoning_delta",
            Self::ToolCallDelta => "tool_call_delta",
            Self::AssistantAttemptReset => "assistant_attempt_reset",
            Self::AssistantMessageSaved => "assistant_message_saved",
            Self::ToolResultSaved => "tool_result_saved",
            Self::WaitingForApproval => "waiting_for_approval",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
/// Durable event emitted by one session.
pub enum SessionEvent {
    InputQueued {
        input_id: String,
        queue_depth: usize,
    },
    InputStarted {
        input_id: String,
        message_id: String,
    },
    AssistantDelta {
        text: String,
    },
    ReasoningDelta {
        text: String,
    },
    ToolCallDelta {
        index: u16,
        id: Option<String>,
        name: Option<String>,
        arguments_delta: Option<String>,
    },
    AssistantAttemptReset,
    AssistantMessageSaved {
        message_id: String,
    },
    ToolResultSaved {
        message_id: String,
    },
    WaitingForApproval,
    Completed {
        message_id: Option<String>,
    },
    Failed {
        error: String,
        causes: Vec<String>,
    },
    Cancelled,
}

impl SessionEvent {
    /// Returns the stable kind shared by persistence and client transports.
    pub const fn kind(&self) -> SessionEventKind {
        match self {
            Self::InputQueued { .. } => SessionEventKind::InputQueued,
            Self::InputStarted { .. } => SessionEventKind::InputStarted,
            Self::AssistantDelta { .. } => SessionEventKind::AssistantDelta,
            Self::ReasoningDelta { .. } => SessionEventKind::ReasoningDelta,
            Self::ToolCallDelta { .. } => SessionEventKind::ToolCallDelta,
            Self::AssistantAttemptReset => SessionEventKind::AssistantAttemptReset,
            Self::AssistantMessageSaved { .. } => SessionEventKind::AssistantMessageSaved,
            Self::ToolResultSaved { .. } => SessionEventKind::ToolResultSaved,
            Self::WaitingForApproval => SessionEventKind::WaitingForApproval,
            Self::Completed { .. } => SessionEventKind::Completed,
            Self::Failed { .. } => SessionEventKind::Failed,
            Self::Cancelled => SessionEventKind::Cancelled,
        }
    }

    /// Returns the SSE event name matching the JSON `type`.
    pub fn event_name(&self) -> &'static str {
        self.kind().as_str()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// One persisted event record with a database-wide monotonic cursor.
pub struct SessionEventRecord {
    pub id: i64,
    pub session_id: SessionId,
    pub event: SessionEvent,
    pub created_at: i64,
}
