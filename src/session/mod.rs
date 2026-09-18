//! Session boundary.
//!
//! A session is one durable execution handle. It records what conversation head
//! Windie is advancing, what lifecycle state that execution is in, what
//! replayable events clients can inspect, and how live session tasks are
//! supervised.

mod control;
mod event;
mod id;
mod live_events;
mod manager;
mod model;
mod policy;

pub use control::{SessionCancellation, SessionControl};
pub use event::{SessionEvent, SessionEventKind, SessionEventRecord};
pub use id::{SessionExecutionClaimId, SessionId, SessionInputId};
pub use live_events::{SessionEventHub, SessionSubscription, SessionSubscriptionError};
pub use manager::SessionManager;
pub use model::{
    ClaimedSession, IdleWakeupInterval, Session, SessionExecutionClaim, SessionExecutionOwner,
    SessionExecutionStart, SessionQueryResult, SessionResolution, SessionStatus,
};

pub(crate) use policy::{can_start, resolve_sessions_at_head};
