//! Typed metadata describing an installable Windie provider.
//!
//! A provider manifest describes what a provider is and what it needs to run.
//! It does not install software, start processes, expose tools to a model, or
//! grant execution permission. Those actions remain owned by the provider
//! manager, registry, and approval policy respectively.

use serde::{Deserialize, Serialize};

use crate::tool::{ToolProviderId, ToolProviderKind};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// Metadata contract for one Windie provider.
pub struct ProviderManifest {
    pub provider_id: ToolProviderId,
    pub display_name: String,
    pub description: String,
    pub kind: ToolProviderKind,
    pub transport: ProviderTransport,
    pub launch: ProviderLaunch,
    pub platforms: Vec<ProviderPlatform>,
    pub dependencies: Vec<ProviderDependency>,
    pub secrets: Vec<ProviderSecret>,
    pub permissions: Vec<ProviderPermission>,
}

impl ProviderManifest {
    /// Builds the initial manifest shape for an MCP stdio provider.
    pub fn mcp_stdio(
        provider_id: impl Into<String>,
        display_name: impl Into<String>,
        description: impl Into<String>,
        program: impl Into<String>,
        args: &[&str],
        platforms: Vec<ProviderPlatform>,
        dependencies: Vec<ProviderDependency>,
        secrets: Vec<ProviderSecret>,
        permissions: Vec<ProviderPermission>,
    ) -> Self {
        Self {
            provider_id: ToolProviderId::new(provider_id),
            display_name: display_name.into(),
            description: description.into(),
            kind: ToolProviderKind::Mcp,
            transport: ProviderTransport::Stdio,
            launch: ProviderLaunch {
                program: program.into(),
                args: args.iter().map(|arg| (*arg).to_string()).collect(),
            },
            platforms,
            dependencies,
            secrets,
            permissions,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Transport used between Windie and a provider.
pub enum ProviderTransport {
    Stdio,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// Process launch information for a provider manifest.
pub struct ProviderLaunch {
    pub program: String,
    pub args: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Operating-system target declared by a provider.
pub enum ProviderPlatform {
    Windows,
    Macos,
    Linux,
}

impl ProviderPlatform {
    /// Returns the platforms currently supported by Windie's local providers.
    pub fn desktop() -> Vec<Self> {
        vec![Self::Windows, Self::Macos, Self::Linux]
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// One executable or runtime dependency required by a provider.
pub struct ProviderDependency {
    pub executable: String,
    pub description: String,
}

impl ProviderDependency {
    /// Creates one executable dependency declaration.
    pub fn executable(executable: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            executable: executable.into(),
            description: description.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// One provider secret required or optionally supported during setup.
pub struct ProviderSecret {
    pub env_key: String,
    pub description: String,
    pub required: bool,
}

impl ProviderSecret {
    /// Creates one required provider secret declaration.
    pub fn required(env_key: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            env_key: env_key.into(),
            description: description.into(),
            required: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Coarse capability or risk category declared by a provider.
pub enum ProviderPermission {
    ExternalProcess,
    Filesystem,
    ComputerControl,
    Network,
}
