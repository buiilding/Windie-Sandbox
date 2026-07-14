//! Session domain types.
//!
//! A session is one durable execution handle. It records what conversation head
//! Windie is advancing, what lifecycle state that execution is in, and what
//! replayable events clients can inspect. This module only defines the typed
//! protocol and storage shape; live task supervision lives in
//! `session_manager.rs`.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::conversation::{ConversationId, MessageId};
use crate::llm::ReasoningRequest;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
/// Stable identifier for one backend-owned runtime session.
pub struct SessionId(String);

impl SessionId {
    /// Creates a fresh session ID.
    pub fn fresh() -> Self {
        Self(Uuid::new_v4().to_string())
    }

    /// Wraps raw ID text from API or storage.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Exposes the ID at persistence and protocol boundaries.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SessionId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Durable lifecycle state for one session.
pub enum SessionStatus {
    Running,
    WaitingForApproval,
    Completed,
    Failed,
    Cancelled,
}

impl SessionStatus {
    /// Converts storage text into the typed status.
    pub fn from_storage(value: &str) -> Option<Self> {
        match value {
            "running" => Some(Self::Running),
            "waiting_for_approval" => Some(Self::WaitingForApproval),
            "completed" => Some(Self::Completed),
            "failed" => Some(Self::Failed),
            "cancelled" => Some(Self::Cancelled),
            _ => None,
        }
    }

    /// Returns the stable storage representation.
    pub fn as_storage(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::WaitingForApproval => "waiting_for_approval",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

impl std::fmt::Display for SessionStatus {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_storage())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Stored metadata for one runtime session.
pub struct Session {
    pub id: SessionId,
    pub conversation_id: ConversationId,
    pub start_head_message_id: Option<MessageId>,
    pub current_head_message_id: Option<MessageId>,
    pub status: SessionStatus,
    pub model: String,
    pub reasoning: Option<ReasoningRequest>,
    pub error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
/// Durable event emitted by one session.
pub enum SessionEvent {
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
    /// Returns the SSE event name matching the JSON `type`.
    pub fn event_name(&self) -> &'static str {
        match self {
            Self::AssistantDelta { .. } => "assistant_delta",
            Self::ReasoningDelta { .. } => "reasoning_delta",
            Self::ToolCallDelta { .. } => "tool_call_delta",
            Self::AssistantMessageSaved { .. } => "assistant_message_saved",
            Self::ToolResultSaved { .. } => "tool_result_saved",
            Self::WaitingForApproval => "waiting_for_approval",
            Self::Completed { .. } => "completed",
            Self::Failed { .. } => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// One persisted event record with a monotonic session-local cursor.
pub struct SessionEventRecord {
    pub id: i64,
    pub session_id: SessionId,
    pub event: SessionEvent,
    pub created_at: i64,
}
