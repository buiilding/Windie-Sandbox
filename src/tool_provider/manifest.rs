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
    pub scope: ProviderScope,
    pub authentication: ProviderAuthentication,
    pub category: String,
    pub tags: Vec<String>,
    pub documentation_url: Option<String>,
    pub setup_guide: Vec<String>,
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
            scope: ProviderScope::Local,
            authentication: ProviderAuthentication::None,
            category: "other".to_string(),
            tags: Vec::new(),
            documentation_url: None,
            setup_guide: Vec::new(),
        }
    }

    /// Adds catalog metadata used by terminal and inspector onboarding.
    pub fn with_metadata(
        mut self,
        scope: ProviderScope,
        authentication: ProviderAuthentication,
        category: impl Into<String>,
        tags: &[&str],
        documentation_url: Option<&str>,
        setup_guide: &[&str],
    ) -> Self {
        self.scope = scope;
        self.authentication = authentication;
        self.category = category.into();
        self.tags = tags.iter().map(|tag| (*tag).to_string()).collect();
        self.documentation_url = documentation_url.map(str::to_string);
        self.setup_guide = setup_guide.iter().map(|step| (*step).to_string()).collect();
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Where the provider's useful service runs relative to Windie.
pub enum ProviderScope {
    Local,
    Cloud,
    Hybrid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Credential interaction required before the provider can start.
pub enum ProviderAuthentication {
    None,
    ApiKey,
    Environment,
    OAuth,
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
