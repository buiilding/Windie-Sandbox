//! Durable session row and lifecycle status types.

use serde::{Deserialize, Serialize};

use crate::conversation::{ConversationId, MessageId};
use crate::llm::ReasoningRequest;

use super::SessionId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Durable lifecycle state for one session.
pub enum SessionStatus {
    Ready,
    Running,
    WaitingForApproval,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Durable identity of the client process currently executing a session.
///
/// This is intentionally separate from a session's lifecycle status. The
/// status says what the session is doing; the owner lets restart recovery
/// distinguish an interrupted API task from a CLI process that is still
/// running independently.
pub enum SessionExecutionOwner {
    Api,
    Cli,
}

impl SessionExecutionOwner {
    /// Returns the stable SQLite representation of this execution owner.
    pub fn as_storage(self) -> &'static str {
        match self {
            Self::Api => "api",
            Self::Cli => "cli",
        }
    }

    /// Decodes one SQLite execution-owner value.
    pub fn from_storage(value: &str) -> Option<Self> {
        match value {
            "api" => Some(Self::Api),
            "cli" => Some(Self::Cli),
            _ => None,
        }
    }
}

impl SessionStatus {
    /// Converts storage text into the typed status.
    pub fn from_storage(value: &str) -> Option<Self> {
        match value {
            "ready" => Some(Self::Ready),
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
            Self::Ready => "ready",
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

#[derive(Debug, Clone)]
/// Backend-owned resolution of one conversation head to a durable session branch.
pub enum SessionResolution {
    /// Exactly one session currently ends at the requested head.
    Existing(Session),
    /// No session currently ends at the requested head.
    NoSessionAtHead,
    /// More than one session currently ends at the requested head.
    Ambiguous(Vec<Session>),
}

#[derive(Debug, Clone)]
/// Result of accepting one user query into a session.
pub struct SessionQueryResult {
    pub session: Session,
    pub queued: bool,
    pub input_id: Option<super::SessionInputId>,
    pub queue_depth: usize,
}
