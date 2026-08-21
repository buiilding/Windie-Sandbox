//! Persisted installed component state.
//!
//! This module stores lifecycle records only. Runtime provider manifests remain
//! owned by the tool domain, while package installation remains owned by the
//! plugin store.

use super::*;
use serde::{Deserialize, Serialize};

use crate::tool::{ProviderInstallState, ProviderReadiness};

impl Store {
    /// Returns whether a provider is installed, enabled, and has no recorded
    /// health error.
    ///
    /// Provider manifests describe what Windie knows about. This persisted
    /// lifecycle record decides whether that provider may be exposed to a
    /// conversation or executed by a runtime session.
    pub fn provider_is_enabled(&self, provider_id: &ToolProviderId) -> Result<bool> {
        Ok(self
            .load_installed_provider(provider_id)?
            .is_some_and(|provider| {
                provider.state == ProviderInstallState::Enabled && provider.error.is_none()
            }))
    }

    /// Loads one installed-provider lifecycle record.
    pub fn load_installed_provider(
        &self,
        provider_id: &ToolProviderId,
    ) -> Result<Option<InstalledProvider>> {
        self.connection
            .query_row(
                "
                SELECT
                    provider_id,
                    state,
                    readiness,
                    next_action,
                    error,
                    installed_at,
                    updated_at,
                    last_health_check_at
                FROM installed_providers
                WHERE provider_id = ?1
                ",
                params![provider_id.as_str()],
                read_installed_provider_row,
            )
            .optional()
            .context("failed to load installed provider")
    }

    /// Creates or resets one provider lifecycle record to `installed`.
    ///
    /// This records manager state only. Provider-specific setup and dependency
    /// installation are orchestrated by `operation::component::setup_provider`.
    pub fn install_provider(&self, provider_id: &ToolProviderId) -> Result<InstalledProvider> {
        let now = now_millis()?;
        self.connection
            .execute(
                "
                INSERT INTO installed_providers (
                    provider_id,
                    state,
                    readiness,
                    next_action,
                    error,
                    installed_at,
                    updated_at,
                    last_health_check_at
                )
                VALUES (?1, ?2, ?3, ?4, NULL, ?5, ?5, NULL)
                ON CONFLICT(provider_id) DO UPDATE SET
                    state = excluded.state,
                    readiness = excluded.readiness,
                    next_action = excluded.next_action,
                    error = NULL,
                    updated_at = excluded.updated_at,
                    last_health_check_at = NULL
                ",
                params![
                    provider_id.as_str(),
                    ProviderInstallState::Installed.as_storage(),
                    ProviderReadiness::Installing.as_storage(),
                    "provision provider runtime",
                    now
                ],
            )
            .context("failed to install provider lifecycle record")?;

        self.load_installed_provider(provider_id)?
            .ok_or_else(|| anyhow!("installed provider was not persisted: {provider_id}"))
    }

    /// Updates the lifecycle state for one installed provider.
    pub fn set_provider_state(
        &self,
        provider_id: &ToolProviderId,
        state: ProviderInstallState,
        error: Option<&str>,
    ) -> Result<InstalledProvider> {
        let now = now_millis()?;
        let changed = self
            .connection
            .execute(
                "
                UPDATE installed_providers
                SET state = ?1,
                    readiness = ?2,
                    next_action = ?3,
                    error = ?4,
                    updated_at = ?5
                WHERE provider_id = ?6
                ",
                params![
                    state.as_storage(),
                    readiness_for_state(state).as_storage(),
                    next_action_for_state(state),
                    error,
                    now,
                    provider_id.as_str()
                ],
            )
            .context("failed to update provider lifecycle state")?;
        if changed == 0 {
            return Err(error::not_found(format!(
                "installed provider does not exist: {provider_id}"
            )));
        }

        self.load_installed_provider(provider_id)?
            .ok_or_else(|| anyhow!("updated provider was not persisted: {provider_id}"))
    }

    /// Persists the current setup phase while the provider remains updating.
    ///
    /// The existing `next_action` column carries this short-lived progress so
    /// the Inspector can poll it without adding a schema migration.
    pub fn set_provider_progress(
        &self,
        provider_id: &ToolProviderId,
        progress: &str,
    ) -> Result<InstalledProvider> {
        let now = now_millis()?;
        let changed = self
            .connection
            .execute(
                "
                UPDATE installed_providers
                SET state = ?1,
                    readiness = ?2,
                    next_action = ?3,
                    error = NULL,
                    updated_at = ?4,
                    last_health_check_at = NULL
                WHERE provider_id = ?5
                ",
                params![
                    ProviderInstallState::Updating.as_storage(),
                    ProviderReadiness::Installing.as_storage(),
                    progress,
                    now,
                    provider_id.as_str()
                ],
            )
            .context("failed to update provider setup progress")?;
        if changed == 0 {
            return Err(error::not_found(format!(
                "installed provider does not exist: {provider_id}"
            )));
        }

        self.load_installed_provider(provider_id)?
            .ok_or_else(|| anyhow!("provider setup progress was not persisted: {provider_id}"))
    }

    /// Records the result of an explicit provider health check.
    pub fn record_provider_health(
        &self,
        provider_id: &ToolProviderId,
        state: ProviderInstallState,
        error: Option<&str>,
    ) -> Result<InstalledProvider> {
        self.record_provider_result(
            provider_id,
            state,
            if error.is_none() {
                ProviderReadiness::Ready
            } else {
                ProviderReadiness::Broken
            },
            None,
            error,
        )
    }

    /// Persists a provider health result together with its actionable readiness.
    pub fn record_provider_result(
        &self,
        provider_id: &ToolProviderId,
        state: ProviderInstallState,
        readiness: ProviderReadiness,
        next_action: Option<&str>,
        error: Option<&str>,
    ) -> Result<InstalledProvider> {
        let now = now_millis()?;
        let changed = self
            .connection
            .execute(
                "
                UPDATE installed_providers
                SET state = ?1,
                    readiness = ?2,
                    next_action = ?3,
                    error = ?4,
                    updated_at = ?5,
                    last_health_check_at = ?5
                WHERE provider_id = ?6
                ",
                params![
                    state.as_storage(),
                    readiness.as_storage(),
                    next_action,
                    error,
                    now,
                    provider_id.as_str()
                ],
            )
            .context("failed to record provider health")?;
        if changed == 0 {
            return Err(error::not_found(format!(
                "installed provider does not exist: {provider_id}"
            )));
        }

        self.load_installed_provider(provider_id)?
            .ok_or_else(|| anyhow!("health result was not persisted: {provider_id}"))
    }

    /// Removes one provider lifecycle record.
    ///
    /// No package files are removed yet. Package cleanup belongs to the phase 3
    /// installer and will call this method after cleanup succeeds.
    pub fn uninstall_provider(&self, provider_id: &ToolProviderId) -> Result<()> {
        let changed = self
            .connection
            .execute(
                "DELETE FROM installed_providers WHERE provider_id = ?1",
                params![provider_id.as_str()],
            )
            .context("failed to uninstall provider lifecycle record")?;
        if changed == 0 {
            return Err(error::not_found(format!(
                "installed provider does not exist: {provider_id}"
            )));
        }

        Ok(())
    }
}

/// Persisted provider-manager row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledProvider {
    pub provider_id: ToolProviderId,
    pub state: ProviderInstallState,
    pub readiness: ProviderReadiness,
    pub next_action: Option<String>,
    pub error: Option<String>,
    pub installed_at: i64,
    pub updated_at: i64,
    pub last_health_check_at: Option<i64>,
}

fn read_installed_provider_row(row: &Row<'_>) -> rusqlite::Result<InstalledProvider> {
    let state_text = row.get::<_, String>(1)?;
    let state = ProviderInstallState::from_storage(&state_text).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            1,
            Type::Text,
            format!("unknown provider install state: {state_text}").into(),
        )
    })?;
    let readiness_text = row.get::<_, String>(2)?;
    let readiness = ProviderReadiness::from_storage(&readiness_text).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            2,
            Type::Text,
            format!("unknown provider readiness: {readiness_text}").into(),
        )
    })?;

    Ok(InstalledProvider {
        provider_id: ToolProviderId::new(row.get::<_, String>(0)?),
        state,
        readiness,
        next_action: row.get(3)?,
        error: row.get(4)?,
        installed_at: row.get(5)?,
        updated_at: row.get(6)?,
        last_health_check_at: row.get(7)?,
    })
}

fn readiness_for_state(state: ProviderInstallState) -> ProviderReadiness {
    match state {
        ProviderInstallState::Updating => ProviderReadiness::Installing,
        ProviderInstallState::Broken => ProviderReadiness::Broken,
        ProviderInstallState::Installed | ProviderInstallState::Disabled => {
            ProviderReadiness::Installing
        }
        ProviderInstallState::Enabled => ProviderReadiness::Ready,
    }
}

fn next_action_for_state(state: ProviderInstallState) -> Option<&'static str> {
    match state {
        ProviderInstallState::Updating => Some("provision and verify provider"),
        _ => None,
    }
}
