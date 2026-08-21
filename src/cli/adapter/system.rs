//! Terminal adapters for Windie process, gateway, onboarding, and environment commands.

use std::io::{self, Write};
use std::net::SocketAddr;

use anyhow::Result;

use crate::cli::{EnvCommand, TerminalOnboarding};
use crate::llm::BaseUrl;
use crate::llm::gateway::GatewayUrl;
use crate::local::process::ManagedComponent;
use crate::operation;
use crate::output::TerminalOutput;
use crate::{config, local};

const INVALID_USAGE_EXIT_CODE: i32 = 2;

/// Starts Windie's local developer API server in the foreground.
pub(crate) async fn run_api() -> Result<()> {
    let gateway_url = gateway_url();
    let base_url = base_url();
    crate::api::serve(api_address(), gateway_url.as_str(), base_url.as_str()).await
}

/// Starts the detached Windie API process.
pub(crate) fn start_api_process() -> Result<()> {
    let report = operation::start_api()?;
    TerminalOutput.component_report(&report);
    Ok(())
}

/// Stops the Windie API process without touching Bifrost.
pub(crate) fn stop_api_process() -> Result<()> {
    let report = operation::stop_api()?;
    TerminalOutput.component_report(&report);
    Ok(())
}

/// Starts the detached native tray process.
pub(crate) fn start_tray_process() -> Result<()> {
    let report = operation::start_tray()?;
    TerminalOutput.component_report(&report);
    Ok(())
}

/// Stops only the native tray process.
pub(crate) fn stop_tray_process() -> Result<()> {
    let report = operation::stop_tray()?;
    TerminalOutput.component_report(&report);
    Ok(())
}

/// Runs the native tray in the foreground for its detached and development
/// entrypoints.
pub(crate) fn run_tray() -> Result<()> {
    crate::local::tray::run()
}

/// Starts the detached notification component without changing the tray or
/// any runtime service.
pub(crate) fn start_notifier_process() -> Result<()> {
    let report = operation::start_notifier()?;
    TerminalOutput.component_report(&report);
    Ok(())
}

/// Stops only the notification component.
pub(crate) fn stop_notifier_process() -> Result<()> {
    let report = operation::stop_notifier()?;
    TerminalOutput.component_report(&report);
    Ok(())
}

/// Runs the notification component in the foreground for detached and
/// development entrypoints.
pub(crate) fn run_notifier() -> Result<()> {
    crate::local::notifier::run()
}

/// Prints persisted output for one independent local component.
pub(crate) fn output_component(component: ManagedComponent) -> Result<()> {
    let output = operation::component_output(component)?;
    TerminalOutput.component_output(component, &output);
    Ok(())
}

/// Runs the terminal-only onboarding wizard without opening a browser.
pub(crate) async fn onboard() -> Result<()> {
    let mut console = TerminalOnboarding::new();
    let gateway_url = gateway_url();
    operation::run_onboarding(
        &mut console,
        gateway_url.clone(),
        gateway_url.as_str(),
        base_url(),
    )
    .await
}

/// Confirms and runs complete Windie cleanup from the CLI boundary.
pub(crate) async fn uninstall_windie(yes: bool, dry_run: bool) -> Result<()> {
    let output = TerminalOutput;
    if !dry_run && !yes && !confirm_uninstall()? {
        println!("windie uninstall: cancelled");
        return Ok(());
    }

    let report = operation::uninstall_windie(dry_run, gateway_url()).await?;
    output.uninstall_report(&report);
    Ok(())
}

/// Reads the destructive-action confirmation without exposing provider keys.
fn confirm_uninstall() -> Result<bool> {
    eprintln!(
        "This will stop Windie and delete all Windie data, provider keys, logs, and installed binaries."
    );
    eprint!("Continue? [y/N] ");
    io::stderr().flush()?;

    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    Ok(matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

/// Prints the generated CLI help text.
pub(crate) fn print_help() -> Result<()> {
    TerminalOutput.help();
    Ok(())
}

/// Prints usage and exits with the conventional bad-usage exit code.
pub(crate) fn invalid_usage() -> Result<()> {
    TerminalOutput.invalid_usage();
    std::process::exit(INVALID_USAGE_EXIT_CODE);
}

/// Prints the package version embedded by Cargo.
pub(crate) fn print_version() -> Result<()> {
    TerminalOutput.version();
    Ok(())
}

/// Runs one user-local environment command.
pub(crate) fn env_command(command: EnvCommand) -> Result<()> {
    let output = TerminalOutput;

    match command {
        EnvCommand::Set(assignments) => {
            let path = local::set_env_values(&assignments)?;
            output.env_updated(&path, assignments.len());
        }
        EnvCommand::List => {
            let keys = local::list_env_keys()?;
            output.env_keys(&keys);
        }
        EnvCommand::Unset(keys) => {
            let path = local::unset_env_values(&keys)?;
            output.env_updated(&path, keys.len());
        }
        EnvCommand::Path => {
            let path = local::env_file_path()?;
            output.env_path(&path);
        }
    }

    Ok(())
}

/// Installs or verifies one approved Windie dependency.
pub(crate) fn install_target(target: &str) -> Result<()> {
    let report = local::install_target(target)?;
    TerminalOutput.install_report(&report);
    Ok(())
}

/// Lists models exposed by the currently running Bifrost gateway.
pub(crate) async fn list_models() -> Result<()> {
    let models = operation::list_models(gateway_url(), base_url()).await?;
    TerminalOutput.models(&models);
    Ok(())
}

/// Prints current local runtime readiness.
pub(crate) async fn status() -> Result<()> {
    let statuses = operation::component_statuses(gateway_url()).await?;
    TerminalOutput.status(&statuses);
    Ok(())
}

/// Starts the local Bifrost gateway when it is not already running.
pub(crate) async fn start_gateway() -> Result<()> {
    let report = operation::start_gateway(gateway_url()).await?;
    TerminalOutput.component_report(&report);
    Ok(())
}

/// Stops the local Bifrost gateway process owned by the configured port.
pub(crate) async fn stop_gateway() -> Result<()> {
    let report = operation::stop_gateway(gateway_url()).await?;
    TerminalOutput.component_report(&report);
    Ok(())
}

/// Centralizes the gateway health base URL for CLI adapters.
pub(super) fn gateway_url() -> GatewayUrl {
    GatewayUrl::new(config::gateway_url())
}

/// Centralizes the OpenAI-compatible API base URL for CLI adapters.
pub(super) fn base_url() -> BaseUrl {
    BaseUrl::new(
        std::env::var("WINDIE_BASE_URL")
            .unwrap_or_else(|_| format!("{}/v1", gateway_url().as_str())),
    )
}

/// Centralizes the local developer API bind address.
fn api_address() -> SocketAddr {
    config::api_address()
        .parse()
        .expect("WINDIE_API_ADDRESS must be a valid socket address")
}
