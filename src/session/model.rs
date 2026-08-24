//! Durable session row and lifecycle status types.

use serde::{Deserialize, Serialize};

use crate::conversation::{ConversationId, MessageId};
use crate::llm::ReasoningRequest;

use super::{SessionExecutionClaimId, SessionId};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// User-selectable interval between autonomous idle wakeups.
///
/// This is a closed set instead of an arbitrary duration so the Inspector can
/// communicate the cost and frequency of autonomous work clearly. The value
/// is persisted with the session and is the source of truth for scheduling.
pub enum IdleWakeupInterval {
    FifteenMinutes,
    #[default]
    ThirtyMinutes,
    OneHour,
    TwoHours,
}

impl IdleWakeupInterval {
    /// Returns the stable SQLite representation for this interval.
    pub fn as_storage(self) -> &'static str {
        match self {
            Self::FifteenMinutes => "fifteen_minutes",
            Self::ThirtyMinutes => "thirty_minutes",
            Self::OneHour => "one_hour",
            Self::TwoHours => "two_hours",
        }
    }

    /// Decodes one persisted interval value.
    pub fn from_storage(value: &str) -> Option<Self> {
        match value {
            "fifteen_minutes" => Some(Self::FifteenMinutes),
            "thirty_minutes" => Some(Self::ThirtyMinutes),
            "one_hour" => Some(Self::OneHour),
            "two_hours" => Some(Self::TwoHours),
            _ => None,
        }
    }

    /// Returns the duration used by the durable scheduler in milliseconds.
    pub fn milliseconds(self) -> i64 {
        match self {
            Self::FifteenMinutes => 15 * 60 * 1_000,
            Self::ThirtyMinutes => 30 * 60 * 1_000,
            Self::OneHour => 60 * 60 * 1_000,
            Self::TwoHours => 2 * 60 * 60 * 1_000,
        }
    }
}

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
/// Durable kind of client currently executing a session.
///
/// This is intentionally separate from a session's lifecycle status. The
/// status says what the session is doing; the owner kind lets restart recovery
/// distinguish an interrupted API task from a CLI process that is still
/// running independently.
pub enum SessionExecutionOwner {
    Api,
    Cli,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Durable state condition required before a new execution may claim a session.
///
/// API and CLI callers use this one typed condition instead of choosing among
/// several claim functions with subtly different state checks.
pub enum SessionExecutionStart {
    /// Starts any session that is neither running nor waiting for approval.
    Runnable,
    /// Starts a runnable session only while it still points at this head.
    RunnableAtHead(Option<MessageId>),
    /// Resumes a session that is paused for an approval decision.
    WaitingForApproval,
    /// Starts one enabled session after its user-activity and wakeup cooldowns
    /// have both elapsed for the persisted interval that was observed by the
    /// scheduler.
    IdleWakeup {
        eligible_before: i64,
        interval: IdleWakeupInterval,
    },
    /// Starts one explicit user-requested wakeup and atomically records that
    /// request as user activity, postponing the next autonomous wakeup.
    ManualWakeup,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Exclusive durable claim held by exactly one session execution attempt.
///
/// `owner` describes the client surface for recovery and diagnostics. `id` is
/// the actual fencing token checked by every run-owned database write.
pub struct SessionExecutionClaim {
    pub id: SessionExecutionClaimId,
    pub owner: SessionExecutionOwner,
}

impl SessionExecutionClaim {
    /// Creates a new claim for one API- or CLI-owned execution attempt.
    pub fn fresh(owner: SessionExecutionOwner) -> Self {
        Self {
            id: SessionExecutionClaimId::fresh(),
            owner,
        }
    }
}

#[derive(Debug, Clone)]
/// Session state returned together with the unique claim that made it runnable.
pub struct ClaimedSession {
    pub session: Session,
    pub claim: SessionExecutionClaim,
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
    /// Whether this session should autonomously wake after the user is idle.
    pub keep_awake: bool,
    /// Durable cadence used when this session autonomously wakes.
    pub idle_wakeup_interval: IdleWakeupInterval,
    /// Latest explicit user interaction with this session, in Unix milliseconds.
    pub last_user_activity_at: i64,
    /// Completion time of the most recent idle wakeup, in Unix milliseconds.
    pub last_idle_wakeup_completed_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl Session {
    /// Returns the next autonomous wakeup time, if this session is enabled.
    ///
    /// The newer of explicit user activity and the most recently completed
    /// autonomous wakeup is the cooldown boundary. This matches the scheduler
    /// and keeps the Inspector's visible timer authoritative.
    pub fn next_idle_wakeup_at(&self) -> Option<i64> {
        self.keep_awake.then(|| {
            self.last_user_activity_at
                .max(self.last_idle_wakeup_completed_at.unwrap_or(i64::MIN))
                .saturating_add(self.idle_wakeup_interval.milliseconds())
        })
    }
}

#[derive(Debug, Clone)]
/// Backend-owned resolution of one conversation head to a durable session branch.
pub enum SessionResolution {
    /// Exactly one session currently ends at the requested head.
    Existing(Box<Session>),
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
