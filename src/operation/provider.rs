//! Provider-manager lifecycle operations.
//!
//! These operations persist provider state and run explicit health checks. They
//! also own the approved MCP setup workflows. Each setup flow first uses the
//! matching local dependency installer, then verifies the provider by loading
//! its MCP catalog before enabling it.

use anyhow::Result;
use serde::Serialize;
use std::env;

use crate::error;
use crate::local;
use crate::store::{InstalledProvider, Store};
use crate::tool::ToolProviderId;
use crate::tool_provider::{
    ProviderInstallState, ProviderManifest, ProviderReadiness, ToolProviderRegistry,
    ToolProviderStatus,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
/// One known provider plus its persisted local lifecycle record.
pub struct ProviderInstallation {
    pub manifest: ProviderManifest,
    pub installation: Option<InstalledProvider>,
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

/// Lists only enabled providers and probes only those providers for tools.
pub fn enabled_provider_statuses(
    store: &Store,
    registry: &ToolProviderRegistry,
) -> Result<Vec<ToolProviderStatus>> {
    let mut statuses = Vec::new();
    for manifest in registry.provider_manifests() {
        if store.provider_is_enabled(&manifest.provider_id)?
            && let Some(status) = registry.provider_status(&manifest.provider_id)
        {
            statuses.push(status);
        }
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
    let manifest = ensure_manifest(registry, provider_id)?;

    store.install_provider(provider_id)?;
    store.set_provider_state(provider_id, ProviderInstallState::Updating, None)?;

    let setup_result = prepare_provider(&manifest)
        .and_then(|_| {
            if manifest.dependencies.is_empty() {
                Ok(())
            } else {
                local::install_target(provider_id.as_str()).map(|_| ())
            }
        })
        .and_then(|_| registry.list_provider_tools(provider_id));

    match setup_result {
        Ok(_) => {
            store.record_provider_health(provider_id, ProviderInstallState::Enabled, None)?;
        }
        Err(provider_error) => {
            let readiness = readiness_for_provider_error(provider_id, &manifest, &provider_error);
            store.record_provider_result(
                provider_id,
                ProviderInstallState::Broken,
                readiness,
                Some(next_action_for_readiness(readiness)),
                Some(provider_error.to_string().as_str()),
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

    match registry.list_provider_tools(provider_id) {
        Ok(_) => {
            store.record_provider_health(provider_id, state_after_check, None)?;
        }
        Err(provider_error) => {
            let readiness = readiness_for_provider_error(provider_id, &manifest, &provider_error);
            store.record_provider_result(
                provider_id,
                ProviderInstallState::Broken,
                readiness,
                Some(next_action_for_readiness(readiness)),
                Some(provider_error.to_string().as_str()),
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
    store.set_provider_state(provider_id, ProviderInstallState::Updating, None)?;

    let repair_result = prepare_provider(&manifest)
        .and_then(|_| {
            if manifest.dependencies.is_empty() {
                Ok(())
            } else {
                local::install_target(provider_id.as_str()).map(|_| ())
            }
        })
        .and_then(|_| registry.list_provider_tools(provider_id));

    match repair_result {
        Ok(_) => {
            store.record_provider_health(provider_id, ProviderInstallState::Installed, None)?;
        }
        Err(provider_error) => {
            let readiness = readiness_for_provider_error(provider_id, &manifest, &provider_error);
            store.record_provider_result(
                provider_id,
                ProviderInstallState::Broken,
                readiness,
                Some(next_action_for_readiness(readiness)),
                Some(provider_error.to_string().as_str()),
            )?;
        }
    }

    provider_installation(store, registry, provider_id)
}

/// Removes one provider from the persisted manager state.
pub fn uninstall_provider(
    store: &Store,
    registry: &ToolProviderRegistry,
    provider_id: &ToolProviderId,
) -> Result<()> {
    ensure_manifest(registry, provider_id)?;
    let installation = require_installation(store, provider_id)?;
    if installation.state == ProviderInstallState::Updating {
        return Err(error::invalid_request(format!(
            "provider is updating: {provider_id}"
        )));
    }

    store.uninstall_provider(provider_id)
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
    if provider_id.as_str() == "blender-mcp"
        || message.contains("blender") && message.contains("bridge")
        || message.contains("external application")
    {
        return ProviderReadiness::ExternalAppRequired;
    }
    if message.contains("permission")
        || message.contains("access denied")
        || message.contains("uac")
        || message.contains("operation not permitted")
    {
        return ProviderReadiness::PermissionRequired;
    }
    if message.contains("runtime")
        || message.contains("command not found")
        || message.contains("no such file")
        || message.contains("npx")
        || message.contains("uvx")
        || message.contains("node.js")
        || message.contains("node runtime")
    {
        return ProviderReadiness::MissingRuntime;
    }

    ProviderReadiness::Broken
}

fn next_action_for_readiness(readiness: ProviderReadiness) -> &'static str {
    match readiness {
        ProviderReadiness::MissingRuntime => "repair provider to install its runtime",
        ProviderReadiness::ExternalAppRequired => {
            "start the required external application and repair"
        }
        ProviderReadiness::PermissionRequired => "grant the required OS permission and repair",
        ProviderReadiness::MissingSecret => "configure the provider secret and repair",
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
    })
}
