//! Lifecycle operations for independently managed local components.
//!
//! The CLI owns argument parsing and terminal formatting. This module exposes
//! typed component lifecycle actions so the process boundary stays reusable
//! without making the operation layer know CLI strings or output syntax.

use anyhow::Result;

pub use crate::process::{ManagedComponent, ProcessReport};

/// Starts the detached Windie API process.
pub fn start_api() -> Result<ProcessReport> {
    crate::process::start_api()
}

/// Stops the Windie API process without affecting Bifrost.
pub fn stop_api() -> Result<ProcessReport> {
    crate::process::stop_api()
}

/// Starts the detached Inspector process.
pub fn start_inspector() -> Result<ProcessReport> {
    crate::process::start_inspector()
}

/// Stops the Inspector process without affecting Windie API or Bifrost.
pub fn stop_inspector() -> Result<ProcessReport> {
    crate::process::stop_inspector()
}

/// Reads one component's persisted stdout/stderr output.
pub fn component_output(component: ManagedComponent) -> Result<String> {
    crate::process::read_output(component)
}
