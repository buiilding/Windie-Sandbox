//! Provider-manager lifecycle operations.
//!
//! These operations persist provider state and run explicit health checks. They
//! also own the first provider-specific setup workflow. Desktop Commander is
//! currently the only provider with one-click setup; its `npx` launch command
//! acquires the package during the first verified MCP catalog request.

use anyhow::Result;
use serde::Serialize;

use crate::error;
use crate::store::{InstalledProvider, Store};
use crate::tool::ToolProviderId;
use crate::tool_provider::{
    ProviderInstallState, ProviderManifest, ToolProviderRegistry, ToolProviderStatus,
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
        if store.provider_is_enabled(&manifest.provider_id)? {
            if let Some(status) = registry.provider_status(&manifest.provider_id) {
                statuses.push(status);
            }
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

/// Installs, configures, verifies, and enables Desktop Commander.
///
/// The provider's MCP command uses `npx -y`, so the first catalog request
/// downloads the published package without asking the user to open a shell.
/// The provider definition prepares an isolated `~/.windie` configuration
/// before that request starts. A failed download, launch, or MCP handshake is
/// retained as `broken` so the caller can show the actionable error.
pub fn setup_provider(
    store: &Store,
    registry: &ToolProviderRegistry,
    provider_id: &ToolProviderId,
) -> Result<ProviderInstallation> {
    let desktop_commander_id = ToolProviderId::new("desktop-commander");
    if provider_id != &desktop_commander_id {
        return Err(error::invalid_request(format!(
            "one-click setup is not implemented for provider: {provider_id}"
        )));
    }

    ensure_manifest(registry, provider_id)?;
    store.install_provider(provider_id)?;
    store.set_provider_state(provider_id, ProviderInstallState::Updating, None)?;

    match registry.list_provider_tools(provider_id) {
        Ok(_) => {
            store.record_provider_health(provider_id, ProviderInstallState::Enabled, None)?;
        }
        Err(provider_error) => {
            store.record_provider_health(
                provider_id,
                ProviderInstallState::Broken,
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
            return Ok(provider_installation(store, registry, provider_id)?);
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
    ensure_manifest(registry, provider_id)?;
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
            store.record_provider_health(
                provider_id,
                ProviderInstallState::Broken,
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
    ensure_manifest(registry, provider_id)?;
    require_installation(store, provider_id)?;
    store.set_provider_state(provider_id, ProviderInstallState::Updating, None)?;

    match registry.list_provider_tools(provider_id) {
        Ok(_) => {
            store.record_provider_health(provider_id, ProviderInstallState::Installed, None)?;
        }
        Err(provider_error) => {
            store.record_provider_health(
                provider_id,
                ProviderInstallState::Broken,
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
