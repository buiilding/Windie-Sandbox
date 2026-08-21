//! Persisted provider-owned MCP tool catalogs.
//!
//! A provider tool catalog is the last successfully discovered set of tools
//! for one installed provider. It is distinct from conversation tool schemas:
//! the catalog describes what a provider offers, while conversation schemas
//! describe what a conversation has explicitly exposed to the model.

use super::*;
use serde::{Deserialize, Serialize};

use crate::tool::{ToolDefinition, ToolProviderId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Availability state for a persisted provider tool catalog.
pub enum ProviderCatalogStatus {
    Fresh,
    Stale,
    Unavailable,
}

impl ProviderCatalogStatus {
    fn as_storage(self) -> &'static str {
        match self {
            Self::Fresh => "fresh",
            Self::Stale => "stale",
            Self::Unavailable => "unavailable",
        }
    }

    fn from_storage(value: &str) -> Option<Self> {
        match value {
            "fresh" => Some(Self::Fresh),
            "stale" => Some(Self::Stale),
            "unavailable" => Some(Self::Unavailable),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
/// One persisted provider tool catalog and its discovery state.
pub struct ProviderToolCatalog {
    pub provider_id: ToolProviderId,
    pub tools: Vec<ToolDefinition>,
    pub status: ProviderCatalogStatus,
    pub discovered_at: Option<i64>,
    pub last_error: Option<String>,
}

impl Store {
    /// Loads the persisted catalog for one provider without starting it.
    pub fn load_provider_tool_catalog(
        &self,
        provider_id: &ToolProviderId,
    ) -> Result<Option<ProviderToolCatalog>> {
        self.connection
            .query_row(
                "
                SELECT provider_id, tools_json, status, discovered_at, last_error
                FROM provider_tool_catalogs
                WHERE provider_id = ?1
                ",
                params![provider_id.as_str()],
                read_provider_tool_catalog_row,
            )
            .optional()
            .context("failed to load provider tool catalog")
    }

    /// Saves a successful live provider discovery as the current catalog.
    pub fn save_provider_tool_catalog(
        &self,
        provider_id: &ToolProviderId,
        tools: &[ToolDefinition],
    ) -> Result<ProviderToolCatalog> {
        let now = now_millis()?;
        let tools_json =
            serde_json::to_string(tools).context("failed to serialize provider tool catalog")?;
        self.connection
            .execute(
                "
                INSERT INTO provider_tool_catalogs (
                    provider_id, tools_json, status, discovered_at, last_error
                )
                VALUES (?1, ?2, ?3, ?4, NULL)
                ON CONFLICT(provider_id) DO UPDATE SET
                    tools_json = excluded.tools_json,
                    status = excluded.status,
                    discovered_at = excluded.discovered_at,
                    last_error = NULL
                ",
                params![
                    provider_id.as_str(),
                    tools_json,
                    ProviderCatalogStatus::Fresh.as_storage(),
                    now
                ],
            )
            .context("failed to save provider tool catalog")?;

        self.load_provider_tool_catalog(provider_id)?
            .ok_or_else(|| anyhow!("provider tool catalog was not persisted: {provider_id}"))
    }

    /// Records discovery failure while preserving the last known tool list.
    pub fn record_provider_tool_catalog_error(
        &self,
        provider_id: &ToolProviderId,
        error: &str,
    ) -> Result<ProviderToolCatalog> {
        let existing = self.load_provider_tool_catalog(provider_id)?;
        match existing {
            Some(_) => {
                self.connection
                    .execute(
                        "
                        UPDATE provider_tool_catalogs
                        SET status = ?1, last_error = ?2
                        WHERE provider_id = ?3
                        ",
                        params![
                            ProviderCatalogStatus::Stale.as_storage(),
                            error,
                            provider_id.as_str()
                        ],
                    )
                    .context("failed to mark provider tool catalog stale")?;
            }
            None => {
                self.connection
                    .execute(
                        "
                        INSERT INTO provider_tool_catalogs (
                            provider_id, tools_json, status, discovered_at, last_error
                        )
                        VALUES (?1, ?2, ?3, NULL, ?4)
                        ",
                        params![
                            provider_id.as_str(),
                            "[]",
                            ProviderCatalogStatus::Unavailable.as_storage(),
                            error
                        ],
                    )
                    .context("failed to save unavailable provider tool catalog")?;
            }
        }

        self.load_provider_tool_catalog(provider_id)?
            .ok_or_else(|| anyhow!("provider tool catalog error was not persisted: {provider_id}"))
    }
}

fn read_provider_tool_catalog_row(row: &Row<'_>) -> rusqlite::Result<ProviderToolCatalog> {
    let tools_json = row.get::<_, String>(1)?;
    let tools = serde_json::from_str(&tools_json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(1, Type::Text, Box::new(error))
    })?;
    let status_text = row.get::<_, String>(2)?;
    let status = ProviderCatalogStatus::from_storage(&status_text).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            2,
            Type::Text,
            format!("unknown provider catalog status: {status_text}").into(),
        )
    })?;

    Ok(ProviderToolCatalog {
        provider_id: ToolProviderId::new(row.get::<_, String>(0)?),
        tools,
        status,
        discovered_at: row.get(3)?,
        last_error: row.get(4)?,
    })
}
