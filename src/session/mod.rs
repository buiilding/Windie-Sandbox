//! Session boundary.
//!
//! A session is one durable execution handle. It records what conversation head
//! Windie is advancing, what lifecycle state that execution is in, what
//! replayable events clients can inspect, and how live session tasks are
//! supervised.

mod event;
mod id;
mod manager;
mod model;

pub use event::{SessionEvent, SessionEventRecord};
pub use id::SessionId;
pub use manager::{SessionManager, SessionSubscription};
pub use model::{Session, SessionStatus};
