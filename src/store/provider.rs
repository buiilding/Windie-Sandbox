//! Persisted provider-manager state.
//!
//! This module stores lifecycle records only. Provider manifests remain owned
//! by `tool_provider`, and installation processes will be added by the next
//! provider-manager phase.

use super::*;
use serde::{Deserialize, Serialize};

use crate::tool_provider::ProviderInstallState;

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
    /// This records manager state only. It intentionally does not download a
    /// package or execute a setup command; phase 3 will supply that behavior.
    pub fn install_provider(&self, provider_id: &ToolProviderId) -> Result<InstalledProvider> {
        let now = now_millis()?;
        self.connection
            .execute(
                "
                INSERT INTO installed_providers (
                    provider_id,
                    state,
                    error,
                    installed_at,
                    updated_at,
                    last_health_check_at
                )
                VALUES (?1, ?2, NULL, ?3, ?3, NULL)
                ON CONFLICT(provider_id) DO UPDATE SET
                    state = excluded.state,
                    error = NULL,
                    updated_at = excluded.updated_at
                ",
                params![
                    provider_id.as_str(),
                    ProviderInstallState::Installed.as_storage(),
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
                SET state = ?1, error = ?2, updated_at = ?3
                WHERE provider_id = ?4
                ",
                params![state.as_storage(), error, now, provider_id.as_str()],
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

    /// Records the result of an explicit provider health check.
    pub fn record_provider_health(
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
                    error = ?2,
                    updated_at = ?3,
                    last_health_check_at = ?3
                WHERE provider_id = ?4
                ",
                params![state.as_storage(), error, now, provider_id.as_str()],
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

    Ok(InstalledProvider {
        provider_id: ToolProviderId::new(row.get::<_, String>(0)?),
        state,
        error: row.get(2)?,
        installed_at: row.get(3)?,
        updated_at: row.get(4)?,
        last_health_check_at: row.get(5)?,
    })
}
