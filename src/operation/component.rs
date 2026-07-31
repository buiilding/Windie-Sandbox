//! Lifecycle operations for independently managed local components.
//!
//! The CLI owns argument parsing and terminal formatting. This module exposes
//! typed component lifecycle actions so the process boundary stays reusable
//! without making the operation layer know CLI strings or output syntax.

use anyhow::{Result, anyhow};

pub use crate::process::{ManagedComponent, ProcessReport};

/// Complete result of one Windie uninstall operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UninstallReport {
    pub dry_run: bool,
    pub plan: crate::local::UninstallPlan,
    pub processes: Vec<ProcessReport>,
    pub gateway: Option<ProcessReport>,
    pub cleanup: Option<crate::local::UninstallCleanup>,
}

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

/// Plans or performs complete Windie cleanup.
///
/// The operation stops every process before deleting any data. If one process
/// cannot be verified as Windie-owned and stopped, cleanup is aborted so its
/// files and state remain available for diagnosis.
pub async fn uninstall_windie(dry_run: bool) -> Result<UninstallReport> {
    let plan = crate::local::uninstall_plan()?;
    if dry_run {
        return Ok(UninstallReport {
            dry_run: true,
            plan,
            processes: Vec::new(),
            gateway: None,
            cleanup: None,
        });
    }

    let process_result = crate::process::stop_windie_processes();
    let gateway_result =
        crate::operation::stop_gateway(crate::gateway::GatewayUrl::new("http://127.0.0.1:8080"))
            .await;
    let mut failures = Vec::new();
    let processes = match process_result {
        Ok(reports) => reports,
        Err(error) => {
            failures.push(format!("{error:#}"));
            Vec::new()
        }
    };
    let gateway = match gateway_result {
        Ok(report) => report,
        Err(error) => {
            failures.push(format!("gateway: {error:#}"));
            return Err(anyhow!(
                "Windie uninstall could not stop all processes: {}",
                failures.join("; ")
            ));
        }
    };
    if !failures.is_empty() {
        return Err(anyhow!(
            "Windie uninstall could not stop all processes: {}",
            failures.join("; ")
        ));
    }
    let cleanup = crate::local::remove_uninstall_plan(&plan)?;

    Ok(UninstallReport {
        dry_run: false,
        plan,
        processes,
        gateway: Some(gateway),
        cleanup: Some(cleanup),
    })
}
