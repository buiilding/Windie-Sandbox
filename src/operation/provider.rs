//! Provider-manager lifecycle operations.
//!
//! These operations persist provider state and run explicit health checks. They
//! also own the approved MCP setup workflows. Each setup flow first uses the
//! matching local dependency installer, then verifies the provider by loading
//! its MCP catalog before enabling it.

use anyhow::Result;
use serde::Serialize;
use std::env;
use std::net::TcpStream;
use std::process::Command;
use std::time::Duration;

use crate::error;
use crate::local;
use crate::store::{InstalledProvider, ProviderCatalogStatus, ProviderToolCatalog, Store};
use crate::tool::ToolProviderId;
use crate::tool_provider::{
    ChromeDevToolsConnectionMode, ProviderInstallState, ProviderManifest, ProviderReadiness,
    ProviderRuntime, ToolProviderRegistry,
};

#[derive(Debug, Clone, PartialEq, Serialize)]
/// One known provider plus its persisted local lifecycle record.
pub struct ProviderInstallation {
    pub manifest: ProviderManifest,
    pub installation: Option<InstalledProvider>,
    pub tool_catalog: Option<ProviderToolCatalog>,
    pub chrome_devtools_mode: Option<ChromeDevToolsConnectionMode>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
/// One provider's persisted tool-catalog status for clients.
pub struct ToolProviderStatus {
    pub provider_id: ToolProviderId,
    pub display_name: String,
    pub manifest: ProviderManifest,
    pub available: bool,
    pub tool_count: usize,
    pub catalog_status: ProviderCatalogStatus,
    pub discovered_at: Option<i64>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
/// Result of the TCP-only Chrome remote-debugging preflight.
pub struct ChromeDevToolsRemoteDebuggingStatus {
    pub available: bool,
    pub address: String,
}

/// Checks whether Chrome is listening for remote debugging on its documented
/// localhost port. This intentionally checks only TCP reachability; the MCP
/// process remains responsible for Chrome's permission and tool handshake.
pub fn chrome_devtools_remote_debugging_status() -> ChromeDevToolsRemoteDebuggingStatus {
    const ADDRESS: &str = "127.0.0.1:9222";
    ChromeDevToolsRemoteDebuggingStatus {
        available: TcpStream::connect_timeout(
            &ADDRESS.parse().expect("Chrome debugging address is valid"),
            Duration::from_millis(400),
        )
        .is_ok(),
        address: ADDRESS.to_string(),
    }
}

/// Opens Chrome's user-facing remote-debugging settings page.
///
/// Normal web pages cannot navigate to `chrome://` URLs because Chrome blocks
/// those schemes from page JavaScript. The Inspector therefore asks Windie's
/// localhost backend to open this fixed page through the operating system.
/// This helper intentionally accepts no caller-provided URL.
pub fn open_chrome_devtools_remote_debugging() -> Result<()> {
    const PAGE: &str = "chrome://inspect/#remote-debugging";

    #[cfg(target_os = "macos")]
    {
        let status = Command::new("open")
            .args(["-a", "Google Chrome", PAGE])
            .status()?;
        if !status.success() {
            anyhow::bail!("Google Chrome could not open its remote-debugging settings");
        }
        Ok(())
    }

    #[cfg(target_os = "windows")]
    {
        let status = Command::new("cmd")
            .args(["/C", "start", "", PAGE])
            .status()?;
        if !status.success() {
            anyhow::bail!("Chrome could not open its remote-debugging settings");
        }
        return Ok(());
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        for executable in [
            "google-chrome",
            "google-chrome-stable",
            "chromium",
            "chromium-browser",
        ] {
            if Command::new("sh")
                .args(["-c", &format!("command -v {executable}")])
                .status()
                .is_ok_and(|status| status.success())
            {
                Command::new(executable).arg(PAGE).spawn()?;
                return Ok(());
            }
        }
        anyhow::bail!("Google Chrome was not found to open its remote-debugging settings");
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", unix)))]
    {
        anyhow::bail!("opening Chrome remote-debugging settings is unsupported on this platform");
    }
}

/// Lists every provider known to the registry and its persisted state.
pub fn list_provider_installations(
    store: &Store,
    registry: &ToolProviderRegistry,
) -> Result<Vec<ProviderInstallation>> {
    registry
        .provider_manifests()
        .into_iter()
        .map(|manifest| {
            Ok(ProviderInstallation {
                installation: store.load_installed_provider(&manifest.provider_id)?,
                tool_catalog: store.load_provider_tool_catalog(&manifest.provider_id)?,
                chrome_devtools_mode: if manifest.provider_id.as_str() == "chrome-devtools" {
                    Some(store.load_chrome_devtools_mode()?.unwrap_or_default())
                } else {
                    None
                },
                manifest,
            })
        })
        .collect()
}

/// Returns whether a known provider is eligible for conversation access.
pub(super) fn require_enabled_provider(
    store: &Store,
    registry: &ToolProviderRegistry,
    provider_id: &ToolProviderId,
) -> Result<()> {
    ensure_manifest(registry, provider_id)?;

    let Some(installation) = store.load_installed_provider(provider_id)? else {
        return Err(error::invalid_request(format!(
            "provider is not installed: {provider_id}"
        )));
    };

    if installation.state != ProviderInstallState::Enabled || installation.error.is_some() {
        return Err(error::invalid_request(format!(
            "provider is not enabled and healthy: {provider_id}"
        )));
    }

    Ok(())
}

/// Lists only enabled providers using their persisted catalog state.
pub fn enabled_provider_statuses(
    store: &Store,
    registry: &ToolProviderRegistry,
) -> Result<Vec<ToolProviderStatus>> {
    let mut statuses = Vec::new();
    for manifest in registry.provider_manifests() {
        if !store.provider_is_enabled(&manifest.provider_id)? {
            continue;
        }
        let catalog = store.load_provider_tool_catalog(&manifest.provider_id)?;
        statuses.push(provider_tool_status(manifest, catalog));
    }
    Ok(statuses)
}

/// Records one known provider as installed.
pub fn install_provider(
    store: &Store,
    registry: &ToolProviderRegistry,
    provider_id: &ToolProviderId,
) -> Result<ProviderInstallation> {
    ensure_manifest(registry, provider_id)?;
    store.install_provider(provider_id)?;
    provider_installation(store, registry, provider_id)
}

/// Installs, configures, verifies, and enables one approved MCP provider.
///
/// Provider provisioning and catalog discovery are one lifecycle operation.
/// A failed provisioning or verification is retained as `broken` so clients
/// can show the actionable error and offer repair without losing the record.
pub fn setup_provider(
    store: &Store,
    registry: &ToolProviderRegistry,
    provider_id: &ToolProviderId,
) -> Result<ProviderInstallation> {
    setup_provider_with_mode(store, registry, provider_id, None)
}

/// Installs and enables one provider using an optional provider-specific mode.
pub fn setup_provider_with_mode(
    store: &Store,
    registry: &ToolProviderRegistry,
    provider_id: &ToolProviderId,
    requested_chrome_devtools_mode: Option<ChromeDevToolsConnectionMode>,
) -> Result<ProviderInstallation> {
    let manifest = ensure_manifest(registry, provider_id)?;

    if provider_id.as_str() == "chrome-devtools" {
        let mode = requested_chrome_devtools_mode
            .or(store.load_chrome_devtools_mode()?)
            .unwrap_or_default();
        registry.set_chrome_devtools_mode(mode)?;
        store.install_provider(provider_id)?;
        store.set_chrome_devtools_mode(mode)?;
    } else {
        store.install_provider(provider_id)?;
    }
    store.set_provider_state(provider_id, ProviderInstallState::Updating, None)?;

    let setup_result =
        prepare_provider_setup(store, registry, provider_id, &manifest).and_then(|tools| {
            store.save_provider_tool_catalog(provider_id, &tools)?;
            Ok(tools)
        });

    match setup_result {
        Ok(_) => {
            store.record_provider_health(provider_id, ProviderInstallState::Enabled, None)?;
        }
        Err(provider_error) => {
            let provider_error_text = provider_error.to_string();
            store.record_provider_tool_catalog_error(provider_id, &provider_error_text)?;
            let readiness = readiness_for_provider_error(provider_id, &manifest, &provider_error);
            store.record_provider_result(
                provider_id,
                ProviderInstallState::Broken,
                readiness,
                Some(next_action_for_readiness(readiness)),
                Some(provider_error_text.as_str()),
            )?;
        }
    }

    provider_installation(store, registry, provider_id)
}

/// Enables one installed provider.
pub fn enable_provider(
    store: &Store,
    registry: &ToolProviderRegistry,
    provider_id: &ToolProviderId,
) -> Result<ProviderInstallation> {
    ensure_manifest(registry, provider_id)?;
    let installation = require_installation(store, provider_id)?;
    match installation.state {
        ProviderInstallState::Broken => {
            return Err(error::invalid_request(format!(
                "provider is broken; repair it before enabling: {provider_id}"
            )));
        }
        ProviderInstallState::Updating => {
            return Err(error::invalid_request(format!(
                "provider is updating: {provider_id}"
            )));
        }
        ProviderInstallState::Enabled => {
            return provider_installation(store, registry, provider_id);
        }
        ProviderInstallState::Installed | ProviderInstallState::Disabled => {}
    }

    store.set_provider_state(provider_id, ProviderInstallState::Enabled, None)?;
    provider_installation(store, registry, provider_id)
}

/// Disables one installed provider without deleting its manager record.
pub fn disable_provider(
    store: &Store,
    registry: &ToolProviderRegistry,
    provider_id: &ToolProviderId,
) -> Result<ProviderInstallation> {
    ensure_manifest(registry, provider_id)?;
    let installation = require_installation(store, provider_id)?;
    if installation.state == ProviderInstallState::Updating {
        return Err(error::invalid_request(format!(
            "provider is updating: {provider_id}"
        )));
    }

    store.set_provider_state(provider_id, ProviderInstallState::Disabled, None)?;
    provider_installation(store, registry, provider_id)
}

/// Re-checks one provider and records whether it is healthy.
pub fn health_check_provider(
    store: &Store,
    registry: &ToolProviderRegistry,
    provider_id: &ToolProviderId,
) -> Result<ProviderInstallation> {
    let manifest = ensure_manifest(registry, provider_id)?;
    let installation = require_installation(store, provider_id)?;
    configure_runtime_mode(store, registry, provider_id)?;
    if installation.state == ProviderInstallState::Updating {
        return Err(error::invalid_request(format!(
            "provider is updating: {provider_id}"
        )));
    }

    let state_after_check = if installation.state == ProviderInstallState::Enabled {
        ProviderInstallState::Enabled
    } else {
        ProviderInstallState::Installed
    };

    let discovery = registry
        .discover_provider_tools(provider_id)
        .and_then(|tools| {
            store.save_provider_tool_catalog(provider_id, &tools)?;
            registry.check_provider_readiness(provider_id)
        });

    match discovery {
        Ok(_) => {
            store.record_provider_health(provider_id, state_after_check, None)?;
        }
        Err(provider_error) => {
            let provider_error_text = provider_error.to_string();
            store.record_provider_tool_catalog_error(provider_id, &provider_error_text)?;
            let readiness = readiness_for_provider_error(provider_id, &manifest, &provider_error);
            store.record_provider_result(
                provider_id,
                ProviderInstallState::Broken,
                readiness,
                Some(next_action_for_readiness(readiness)),
                Some(provider_error_text.as_str()),
            )?;
        }
    }

    provider_installation(store, registry, provider_id)
}

/// Re-checks a provider after moving it through the updating state.
pub fn repair_provider(
    store: &Store,
    registry: &ToolProviderRegistry,
    provider_id: &ToolProviderId,
) -> Result<ProviderInstallation> {
    let manifest = ensure_manifest(registry, provider_id)?;
    require_installation(store, provider_id)?;
    configure_runtime_mode(store, registry, provider_id)?;
    store.set_provider_state(provider_id, ProviderInstallState::Updating, None)?;

    let repair_result =
        prepare_provider_setup(store, registry, provider_id, &manifest).and_then(|tools| {
            store.save_provider_tool_catalog(provider_id, &tools)?;
            Ok(tools)
        });

    match repair_result {
        Ok(_) => {
            store.record_provider_health(provider_id, ProviderInstallState::Installed, None)?;
        }
        Err(provider_error) => {
            let provider_error_text = provider_error.to_string();
            store.record_provider_tool_catalog_error(provider_id, &provider_error_text)?;
            let readiness = readiness_for_provider_error(provider_id, &manifest, &provider_error);
            store.record_provider_result(
                provider_id,
                ProviderInstallState::Broken,
                readiness,
                Some(next_action_for_readiness(readiness)),
                Some(provider_error_text.as_str()),
            )?;
        }
    }

    provider_installation(store, registry, provider_id)
}

/// Removes one provider from the persisted manager state.
pub fn uninstall_provider(
    store: &mut Store,
    registry: &ToolProviderRegistry,
    provider_id: &ToolProviderId,
) -> Result<()> {
    let manifest = ensure_manifest(registry, provider_id)?;
    let installation = require_installation(store, provider_id)?;
    if installation.state == ProviderInstallState::Updating {
        return Err(error::invalid_request(format!(
            "provider is updating: {provider_id}"
        )));
    }

    let remove_runtime =
        !runtime_is_used_by_another_provider(store, registry, provider_id, manifest.runtime)?;
    registry.uninstall_provider_runtime(provider_id, remove_runtime)?;

    let secret_keys = manifest
        .secrets
        .iter()
        .map(|secret| secret.env_key.clone())
        .collect::<Vec<_>>();
    if !secret_keys.is_empty() {
        local::unset_env_values(&secret_keys)?;
    }

    store.uninstall_provider(provider_id)
}

/// Switches an installed provider's runtime configuration and re-discovers its
/// catalog without reinstalling its package or managed runtime.
pub fn configure_provider(
    store: &Store,
    registry: &ToolProviderRegistry,
    provider_id: &ToolProviderId,
    mode: ChromeDevToolsConnectionMode,
) -> Result<ProviderInstallation> {
    let manifest = ensure_manifest(registry, provider_id)?;
    require_installation(store, provider_id)?;
    if provider_id.as_str() != "chrome-devtools" {
        return Err(error::invalid_request(
            "configuration is only supported for Chrome DevTools",
        ));
    }

    registry.set_chrome_devtools_mode(mode)?;
    store.set_chrome_devtools_mode(mode)?;
    store.set_provider_state(provider_id, ProviderInstallState::Updating, None)?;

    match registry
        .discover_provider_tools(provider_id)
        .and_then(|tools| {
            store.save_provider_tool_catalog(provider_id, &tools)?;
            registry.check_provider_readiness(provider_id)
        }) {
        Ok(()) => {
            store.record_provider_health(provider_id, ProviderInstallState::Enabled, None)?;
        }
        Err(provider_error) => {
            let error_text = provider_error.to_string();
            let readiness = readiness_for_provider_error(provider_id, &manifest, &provider_error);
            store.record_provider_result(
                provider_id,
                ProviderInstallState::Broken,
                readiness,
                Some(next_action_for_readiness(readiness)),
                Some(error_text.as_str()),
            )?;
        }
    }

    provider_installation(store, registry, provider_id)
}

/// Applies the persisted runtime mode to the live registry before a provider
/// operation starts a new MCP process.
fn configure_runtime_mode(
    store: &Store,
    registry: &ToolProviderRegistry,
    provider_id: &ToolProviderId,
) -> Result<()> {
    if provider_id.as_str() == "chrome-devtools" {
        registry
            .set_chrome_devtools_mode(store.load_chrome_devtools_mode()?.unwrap_or_default())?;
    }
    Ok(())
}

/// Returns whether another installed provider still needs this shared runtime.
fn runtime_is_used_by_another_provider(
    store: &Store,
    registry: &ToolProviderRegistry,
    provider_id: &ToolProviderId,
    runtime: ProviderRuntime,
) -> Result<bool> {
    if runtime == ProviderRuntime::Native {
        return Ok(false);
    }

    for manifest in registry.provider_manifests() {
        if manifest.provider_id == *provider_id || manifest.runtime != runtime {
            continue;
        }
        if store
            .load_installed_provider(&manifest.provider_id)?
            .is_some()
        {
            return Ok(true);
        }
    }

    Ok(false)
}

fn ensure_manifest(
    registry: &ToolProviderRegistry,
    provider_id: &ToolProviderId,
) -> Result<ProviderManifest> {
    registry
        .provider_manifest(provider_id)
        .ok_or_else(|| error::not_found(format!("provider does not exist: {provider_id}")))
}

fn require_installation(store: &Store, provider_id: &ToolProviderId) -> Result<InstalledProvider> {
    store
        .load_installed_provider(provider_id)?
        .ok_or_else(|| error::not_found(format!("provider is not installed: {provider_id}")))
}

/// Runs deterministic checks that can explain a setup failure before an MCP
/// process is started. These checks intentionally read only the provider
/// manifest and Windie's explicit environment file.
fn prepare_provider(manifest: &ProviderManifest) -> Result<()> {
    if !crate::tool_provider::ProviderPlatform::supports_current(&manifest.platforms) {
        return Err(anyhow::anyhow!(
            "provider does not support the current operating system"
        ));
    }

    for secret in &manifest.secrets {
        if secret.required {
            let configured =
                local::env_value(&secret.env_key)?.or_else(|| env::var(&secret.env_key).ok());
            if configured.as_deref().is_none_or(str::is_empty) {
                return Err(anyhow::anyhow!(
                    "missing required provider secret {} ({})",
                    secret.env_key,
                    secret.description
                ));
            }
        }
    }

    Ok(())
}

/// Runs provider setup in explicit, observable phases before catalog discovery.
///
/// Runtime and package preparation intentionally happen outside MCP startup.
/// This keeps first-use downloads out of the protocol initialize timeout and
/// gives the Inspector a useful progress message while the operation runs.
fn prepare_provider_setup(
    store: &Store,
    registry: &ToolProviderRegistry,
    provider_id: &ToolProviderId,
    manifest: &ProviderManifest,
) -> Result<Vec<crate::tool::ToolDefinition>> {
    store.set_provider_progress(provider_id, "preparing provider")?;
    prepare_provider(manifest)?;

    if !manifest.dependencies.is_empty() {
        let runtime_message = manifest
            .runtime
            .display_name()
            .map(|name| format!("installing {name}"))
            .unwrap_or_else(|| "installing provider runtime".to_string());
        store.set_provider_progress(provider_id, &runtime_message)?;
        registry.prepare_provider_runtime(provider_id)?;
    }

    if registry.provider_requires_package_preparation(provider_id)? {
        store.set_provider_progress(
            provider_id,
            &format!("installing {}", manifest.display_name),
        )?;
        registry.prepare_provider_package(provider_id)?;
    }

    store.set_provider_progress(provider_id, "preparing provider configuration")?;
    registry.prepare_provider_configuration(provider_id)?;

    store.set_provider_progress(provider_id, "connecting to MCP server")?;
    store.set_provider_progress(provider_id, "checking MCP tools")?;
    registry.discover_provider_tools(provider_id)
}

/// Maps a setup/health error to the stable UI-facing readiness category.
fn readiness_for_provider_error(
    provider_id: &ToolProviderId,
    manifest: &ProviderManifest,
    error: &anyhow::Error,
) -> ProviderReadiness {
    let message = error.to_string().to_ascii_lowercase();
    if !crate::tool_provider::ProviderPlatform::supports_current(&manifest.platforms) {
        return ProviderReadiness::UnsupportedPlatform;
    }
    if message.contains("missing required provider secret")
        || message.contains("environment variable")
        || message.contains("api token")
    {
        return ProviderReadiness::MissingSecret;
    }
    if message.contains("authentication failed") {
        return ProviderReadiness::AuthenticationFailed;
    }
    if message.contains("permission")
        || message.contains("access denied")
        || message.contains("uac")
        || message.contains("operation not permitted")
        || message.contains("chrome remote debugging")
    {
        return ProviderReadiness::PermissionRequired;
    }
    if message.contains("provider package preparation") {
        return ProviderReadiness::PackageSetupFailed;
    }
    if message.contains("required command is not available")
        || message.contains("runtime is not supported")
        || message.contains("runtime installation")
        || message.contains("failed to verify downloaded")
        || message.contains("checksum")
        || message.contains("command not found")
    {
        return ProviderReadiness::MissingRuntime;
    }
    if message.contains("mcp provider timed out")
        || message.contains("mcp provider closed")
        || message.contains("failed to start mcp provider")
        || message.contains("mcp provider stdout")
        || message.contains("failed to read mcp response")
        || message.contains("mcp response")
        || message.contains("mcp error")
    {
        return ProviderReadiness::ProviderStartupFailed;
    }
    if provider_id.as_str() == "blender-mcp"
        || message.contains("blender") && message.contains("bridge")
        || message.contains("external application")
    {
        return ProviderReadiness::ExternalAppRequired;
    }

    ProviderReadiness::Broken
}

fn next_action_for_readiness(readiness: ProviderReadiness) -> &'static str {
    match readiness {
        ProviderReadiness::MissingRuntime => "repair provider to install its runtime",
        ProviderReadiness::PackageSetupFailed => "retry provider package setup",
        ProviderReadiness::ProviderStartupFailed => "retry provider startup",
        ProviderReadiness::ExternalAppRequired => {
            "start the required external application and repair"
        }
        ProviderReadiness::PermissionRequired => "grant the required OS permission and repair",
        ProviderReadiness::MissingSecret => "configure the provider secret and repair",
        ProviderReadiness::AuthenticationFailed => "check the provider credential and repair",
        ProviderReadiness::UnsupportedPlatform => "use this provider on a supported platform",
        ProviderReadiness::Installing => "wait for provider setup to finish",
        ProviderReadiness::Ready => "none",
        ProviderReadiness::Broken => "inspect the error and repair the provider",
    }
}

fn provider_installation(
    store: &Store,
    registry: &ToolProviderRegistry,
    provider_id: &ToolProviderId,
) -> Result<ProviderInstallation> {
    Ok(ProviderInstallation {
        manifest: ensure_manifest(registry, provider_id)?,
        installation: store.load_installed_provider(provider_id)?,
        tool_catalog: store.load_provider_tool_catalog(provider_id)?,
        chrome_devtools_mode: if provider_id.as_str() == "chrome-devtools" {
            Some(store.load_chrome_devtools_mode()?.unwrap_or_default())
        } else {
            None
        },
    })
}

/// Converts one persisted catalog into the compact API status used by tool
/// attachment surfaces. The status is intentionally read-only and never starts
/// the provider process.
fn provider_tool_status(
    manifest: ProviderManifest,
    catalog: Option<ProviderToolCatalog>,
) -> ToolProviderStatus {
    let (available, tool_count, catalog_status, discovered_at, error) = match catalog {
        Some(catalog) => (
            catalog.status != ProviderCatalogStatus::Unavailable,
            catalog.tools.len(),
            catalog.status,
            catalog.discovered_at,
            catalog.last_error,
        ),
        None => (false, 0, ProviderCatalogStatus::Unavailable, None, None),
    };

    ToolProviderStatus {
        provider_id: manifest.provider_id.clone(),
        display_name: manifest.display_name.clone(),
        manifest,
        available,
        tool_count,
        catalog_status,
        discovered_at,
        error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool_provider::ProviderPlatform;

    fn blender_manifest() -> ProviderManifest {
        ProviderManifest::mcp_stdio(
            "blender-mcp",
            "Blender MCP",
            "test provider",
            "uvx",
            &[],
            ProviderPlatform::desktop(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
    }

    #[test]
    fn runtime_failures_take_precedence_over_blender_app_hint() {
        let manifest = blender_manifest();
        let error = anyhow::anyhow!("failed to verify downloaded uv archive");

        assert_eq!(
            readiness_for_provider_error(&ToolProviderId::new("blender-mcp"), &manifest, &error,),
            ProviderReadiness::MissingRuntime
        );
    }

    #[test]
    fn blender_bridge_failures_still_require_the_external_app() {
        let manifest = blender_manifest();
        let error = anyhow::anyhow!("Blender bridge is not running");

        assert_eq!(
            readiness_for_provider_error(&ToolProviderId::new("blender-mcp"), &manifest, &error,),
            ProviderReadiness::ExternalAppRequired
        );
    }

    #[test]
    fn mcp_startup_timeout_is_not_reported_as_missing_runtime() {
        let manifest = blender_manifest();
        let error = anyhow::anyhow!("MCP provider timed out during initialize after 30s: npx");

        assert_eq!(
            readiness_for_provider_error(&ToolProviderId::new("blender-mcp"), &manifest, &error,),
            ProviderReadiness::ProviderStartupFailed
        );
    }

    #[test]
    fn missing_runtime_error_is_reported_as_missing_runtime() {
        let manifest = blender_manifest();
        let error = anyhow::anyhow!("required command is not available: uvx");

        assert_eq!(
            readiness_for_provider_error(&ToolProviderId::new("blender-mcp"), &manifest, &error,),
            ProviderReadiness::MissingRuntime
        );
    }

    #[test]
    fn package_preparation_error_has_its_own_readiness_category() {
        let manifest = blender_manifest();
        let error = anyhow::anyhow!(
            "provider package preparation failed with exit status: 1: uvx\nstderr:\npackage unavailable"
        );

        assert_eq!(
            readiness_for_provider_error(&ToolProviderId::new("blender-mcp"), &manifest, &error,),
            ProviderReadiness::PackageSetupFailed
        );
    }

    #[test]
    fn remote_authentication_failures_are_actionable() {
        let manifest = blender_manifest();
        let error =
            anyhow::anyhow!("MCP HTTP authentication failed during initialize (status 401)");

        assert_eq!(
            readiness_for_provider_error(
                &ToolProviderId::new("parallel-search"),
                &manifest,
                &error,
            ),
            ProviderReadiness::AuthenticationFailed
        );
    }
}
