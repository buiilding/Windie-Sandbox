//! Local developer tooling boundary.
//!
//! This folder owns helper code for developer-only clients and harnesses. It
//! should not own runtime state, persistence, provider calls, or operation
//! orchestration.

mod inspector;

pub(crate) use inspector::open;
