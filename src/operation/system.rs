//! Lifecycle operations for independently managed local components.
//!
//! The CLI owns argument parsing and terminal formatting. This module exposes
//! typed component lifecycle actions so the process boundary stays reusable
//! without making the operation layer know CLI strings or output syntax.

use std::time::Duration;

use anyhow::{Result, anyhow};

pub use crate::local::process::{ManagedComponent, ProcessReport};

/// One component's current availability as observed by the shared lifecycle
/// boundary. `running` means its health endpoint responded, except for the
/// tray, whose owned process identity is its only local liveness signal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ComponentStatus {
    pub component: ManagedComponent,
    pub running: bool,
}

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
    crate::local::process::start_api()
}

/// Stops the Windie API process without affecting Bifrost.
pub fn stop_api() -> Result<ProcessReport> {
    crate::local::process::stop_api()
}

/// Starts the detached Inspector process.
pub fn start_inspector() -> Result<ProcessReport> {
    crate::local::process::start_inspector()
}

/// Stops the Inspector process without affecting Windie API or Bifrost.
pub fn stop_inspector() -> Result<ProcessReport> {
    crate::local::process::stop_inspector()
}

/// Starts the detached native tray without changing any other component.
pub fn start_tray() -> Result<ProcessReport> {
    crate::local::process::start_tray()
}

/// Stops only the native tray process.
pub fn stop_tray() -> Result<ProcessReport> {
    crate::local::process::stop_tray()
}

/// Starts the detached notification component without starting the tray.
pub fn start_notifier() -> Result<ProcessReport> {
    crate::local::process::start_notifier()
}

/// Stops only the notification component.
pub fn stop_notifier() -> Result<ProcessReport> {
    crate::local::process::stop_notifier()
}

/// Reads all independently managed component states without starting,
/// stopping, or otherwise changing any of them.
pub async fn component_statuses(
    gateway_url: crate::llm::gateway::GatewayUrl,
) -> Result<Vec<ComponentStatus>> {
    let gateway = crate::operation::gateway_status(gateway_url);
    let api = endpoint_is_running(format!("{}/api/health", crate::config::api_url()));
    let inspector = endpoint_is_running(format!("http://{}/", crate::config::inspector_address()));
    let (gateway_running, api_running, inspector_running) = tokio::join!(gateway, api, inspector);

    Ok(vec![
        ComponentStatus {
            component: ManagedComponent::Gateway,
            running: gateway_running,
        },
        ComponentStatus {
            component: ManagedComponent::Api,
            running: api_running,
        },
        ComponentStatus {
            component: ManagedComponent::Inspector,
            running: inspector_running,
        },
        ComponentStatus {
            component: ManagedComponent::Tray,
            running: crate::local::process::is_managed_component_running(ManagedComponent::Tray)?,
        },
        ComponentStatus {
            component: ManagedComponent::Notifier,
            running: crate::local::process::is_managed_component_running(
                ManagedComponent::Notifier,
            )?,
        },
    ])
}

/// Checks one component endpoint with the same short timeout used by local
/// process controls, so `windie status` stays responsive when it is offline.
async fn endpoint_is_running(url: String) -> bool {
    let Ok(client) = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
    else {
        return false;
    };

    client
        .get(url)
        .send()
        .await
        .is_ok_and(|response| response.status().is_success())
}

/// Reads one component's persisted stdout/stderr output.
pub fn component_output(component: ManagedComponent) -> Result<String> {
    crate::local::process::read_output(component)
}

/// Plans or performs complete Windie cleanup.
///
/// The operation stops every process before deleting any data. If one process
/// cannot be verified as Windie-owned and stopped, cleanup is aborted so its
/// files and state remain available for diagnosis.
pub async fn uninstall_windie(
    dry_run: bool,
    gateway_url: crate::llm::gateway::GatewayUrl,
) -> Result<UninstallReport> {
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

    let process_result = crate::local::process::stop_windie_processes();
    let gateway_result = crate::operation::stop_gateway(gateway_url).await;
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
