//! Persisted connection-mode types for the package-owned Chrome DevTools MCP.

use serde::{Deserialize, Serialize};

/// Environment variable consumed by the package-owned Chrome launcher.
pub(crate) const CHROME_DEVTOOLS_CONNECTION_MODE_ENV: &str = "WINDIE_CHROME_CONNECTION_MODE";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Selects which Chrome browser owns a Chrome DevTools MCP session.
pub(crate) enum ChromeDevToolsConnectionMode {
    /// Start a separate persistent browser profile owned by Windie.
    #[default]
    Managed,
    /// Attach to the user's already-running Chrome through Chrome's local
    /// remote-debugging approval flow.
    Existing,
}

impl ChromeDevToolsConnectionMode {
    /// Returns the stable SQLite representation.
    pub(crate) fn as_storage(self) -> &'static str {
        match self {
            Self::Managed => "managed",
            Self::Existing => "existing",
        }
    }

    /// Decodes the stable SQLite representation.
    pub(crate) fn from_storage(value: &str) -> Option<Self> {
        match value {
            "managed" => Some(Self::Managed),
            "existing" => Some(Self::Existing),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_modes_round_trip_through_storage_names() {
        assert_eq!(
            ChromeDevToolsConnectionMode::from_storage("managed"),
            Some(ChromeDevToolsConnectionMode::Managed)
        );
        assert_eq!(
            ChromeDevToolsConnectionMode::from_storage("existing"),
            Some(ChromeDevToolsConnectionMode::Existing)
        );
        assert_eq!(ChromeDevToolsConnectionMode::from_storage("other"), None);
    }
}
