//! Session boundary.
//!
//! A session is one durable execution handle. It records what conversation head
//! Windie is advancing, what lifecycle state that execution is in, what
//! replayable events clients can inspect, and how live session tasks are
//! supervised.

mod control;
mod event;
mod id;
mod manager;
mod model;

pub use control::{SessionCancellation, SessionControl};
pub use event::{SessionEvent, SessionEventKind, SessionEventRecord};
pub use id::{SessionExecutionClaimId, SessionId, SessionInputId};
pub use manager::{SessionManager, SessionSubscription};
pub use model::{
    ClaimedSession, Session, SessionExecutionClaim, SessionExecutionOwner, SessionExecutionStart,
    SessionQueryResult, SessionResolution, SessionStatus,
};
