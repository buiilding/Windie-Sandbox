//! Persisted lifecycle states for installed Windie providers.
//!
//! These states describe Windie's local provider-manager record. They do not
//! install software or grant a model access to provider tools.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Lifecycle state persisted for one installed provider.
pub enum ProviderInstallState {
    Installed,
    Enabled,
    Disabled,
    Broken,
    Updating,
}

impl ProviderInstallState {
    /// Returns the stable SQLite representation.
    pub fn as_storage(self) -> &'static str {
        match self {
            Self::Installed => "installed",
            Self::Enabled => "enabled",
            Self::Disabled => "disabled",
            Self::Broken => "broken",
            Self::Updating => "updating",
        }
    }

    /// Decodes the stable SQLite representation.
    pub fn from_storage(value: &str) -> Option<Self> {
        match value {
            "installed" => Some(Self::Installed),
            "enabled" => Some(Self::Enabled),
            "disabled" => Some(Self::Disabled),
            "broken" => Some(Self::Broken),
            "updating" => Some(Self::Updating),
            _ => None,
        }
    }
}
