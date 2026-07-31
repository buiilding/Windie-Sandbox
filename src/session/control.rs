//! Explicit control inputs for durable sessions.
//!
//! Controls change or terminate session execution. They are separate from
//! `Wakeup`, which represents an event that resumes runtime work.

use super::SessionId;

#[derive(Debug, Clone, PartialEq, Eq)]
/// Control input applied to one durable session.
pub enum SessionControl {
    Cancel(SessionCancellation),
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Request to cancel one session and terminate its active runtime task.
pub struct SessionCancellation {
    pub session_id: SessionId,
}
