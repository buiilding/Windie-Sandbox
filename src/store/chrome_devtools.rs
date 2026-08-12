//! Persisted Chrome DevTools MCP connection settings.
//!
//! The provider manifest remains code-owned, while this table stores the one
//! user-selected runtime mode that changes how the approved provider connects.

use super::*;
use crate::mcp::ChromeDevToolsConnectionMode;

impl Store {
    /// Loads the user's selected Chrome DevTools connection mode.
    pub fn load_chrome_devtools_mode(&self) -> Result<Option<ChromeDevToolsConnectionMode>> {
        self.connection
            .query_row(
                "SELECT mode FROM chrome_devtools_settings WHERE provider_id = 'chrome-devtools'",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .context("failed to load Chrome DevTools connection mode")?
            .map(|value| {
                ChromeDevToolsConnectionMode::from_storage(&value)
                    .ok_or_else(|| anyhow!("unknown Chrome DevTools connection mode: {value}"))
            })
            .transpose()
    }

    /// Persists the user's selected Chrome DevTools connection mode.
    pub fn set_chrome_devtools_mode(
        &self,
        mode: ChromeDevToolsConnectionMode,
    ) -> Result<ChromeDevToolsConnectionMode> {
        let now = now_millis()?;
        self.connection
            .execute(
                "
                INSERT INTO chrome_devtools_settings (provider_id, mode, updated_at)
                VALUES ('chrome-devtools', ?1, ?2)
                ON CONFLICT(provider_id) DO UPDATE SET
                    mode = excluded.mode,
                    updated_at = excluded.updated_at
                ",
                params![mode.as_storage(), now],
            )
            .context("failed to save Chrome DevTools connection mode")?;
        Ok(mode)
    }
}
