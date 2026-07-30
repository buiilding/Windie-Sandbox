//! Persisted lifecycle states for installed Windie providers.
//!
//! These states describe Windie's local provider-manager record. They do not
//! install software or grant a model access to provider tools.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Computed readiness of one provider installation.
pub enum ProviderReadiness {
    Ready,
    MissingRuntime,
    ExternalAppRequired,
    PermissionRequired,
    MissingSecret,
    UnsupportedPlatform,
    Installing,
    Broken,
}

impl ProviderReadiness {
    /// Returns the stable SQLite representation.
    pub fn as_storage(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::MissingRuntime => "missing_runtime",
            Self::ExternalAppRequired => "external_app_required",
            Self::PermissionRequired => "permission_required",
            Self::MissingSecret => "missing_secret",
            Self::UnsupportedPlatform => "unsupported_platform",
            Self::Installing => "installing",
            Self::Broken => "broken",
        }
    }

    /// Decodes the stable SQLite representation.
    pub fn from_storage(value: &str) -> Option<Self> {
        match value {
            "ready" => Some(Self::Ready),
            "missing_runtime" => Some(Self::MissingRuntime),
            "external_app_required" => Some(Self::ExternalAppRequired),
            "permission_required" => Some(Self::PermissionRequired),
            "missing_secret" => Some(Self::MissingSecret),
            "unsupported_platform" => Some(Self::UnsupportedPlatform),
            "installing" => Some(Self::Installing),
            "broken" => Some(Self::Broken),
            _ => None,
        }
    }
}

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
