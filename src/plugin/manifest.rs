//! Typed plugin and component manifest contracts.
//!
//! Plugin metadata describes the installable package. Component manifests
//! describe how one runtime consumes a plugin component. Runtime code must
//! validate these contracts before starting processes or connecting to
//! external services.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::mcp::{McpHttpAuthorization, McpHttpEndpoint, McpTransport};
use crate::tool::{
    ProviderAuthentication, ProviderManifest, ProviderPackage, ProviderPackageManager,
    ProviderPermission, ProviderPlatform, ProviderScope, ProviderSecret,
};

const MAX_MANIFEST_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// Top-level metadata for one installable Windie plugin.
pub struct PluginManifest {
    pub manifest_version: u32,
    pub plugin: PluginIdentity,
    pub presentation: PluginPresentation,
    pub components: Vec<PluginComponent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// Stable plugin identity and release metadata.
pub struct PluginIdentity {
    pub id: String,
    pub version: String,
    pub publisher: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// User-facing plugin catalog metadata.
pub struct PluginPresentation {
    pub name: String,
    pub description: String,
    pub readme: String,
    pub icon: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// Reference to a component manifest stored inside the plugin package.
pub struct PluginComponent {
    #[serde(rename = "type")]
    pub kind: PluginComponentKind,
    pub id: String,
    pub manifest: String,
    /// Windie-only policy and runtime metadata for this component.
    #[serde(default)]
    pub windie: WindieMcpMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Parsed metadata and instructions for a packaged `SKILL.md` component.
pub struct SkillManifest {
    pub name: String,
    pub description: String,
    pub instructions: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
/// Optional metadata for an app connector component.
pub struct AppManifest {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Component families supported by the plugin package boundary.
pub enum PluginComponentKind {
    Mcp,
    Skill,
    App,
}

impl fmt::Display for PluginComponentKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Mcp => "mcp",
            Self::Skill => "skill",
            Self::App => "app",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// Windie policy layered on top of standard MCP metadata.
///
/// This is deliberately stored in `plugin.json`, not in `mcp/server.json`.
/// The latter remains consumable by other MCP clients and registries.
pub struct WindieMcpMetadata {
    #[serde(default)]
    pub authentication: McpAuthentication,
    #[serde(default)]
    pub permissions: Vec<ProviderPermission>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub startup_timeout_ms: Option<u64>,
    #[serde(default)]
    pub call_timeout_ms: Option<u64>,
    /// Optional read-only MCP tool Windie calls during provider health checks.
    #[serde(default)]
    pub readiness_probe: Option<String>,
    /// Relative path to a bundled MCPB artifact, when this component is local.
    #[serde(default)]
    pub local_artifact: Option<String>,
    /// Windie-owned local setup applied inside the component runtime directory.
    #[serde(default)]
    pub setup: WindieMcpSetup,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
/// Declarative, shell-free setup for a local MCP component.
pub struct WindieMcpSetup {
    /// Gives the component a private HOME rooted in its installed runtime.
    #[serde(default)]
    pub isolated_home: bool,
    /// Environment values resolved by Windie before the component starts.
    #[serde(default)]
    pub environment: Vec<WindieSetupEnvironment>,
    /// JSON configuration files Windie writes below that private HOME.
    #[serde(default)]
    pub files: Vec<WindieSetupFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// One declarative environment value for a local MCP component.
pub struct WindieSetupEnvironment {
    pub name: String,
    pub value: WindieSetupEnvironmentValue,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
/// Sources from which Windie can safely resolve a component environment value.
pub enum WindieSetupEnvironmentValue {
    /// A package-authored constant that does not contain executable syntax.
    Literal { value: String },
    /// A path below the extracted component directory.
    ComponentPath { path: String },
    /// A path below Windie’s persistent user data directory.
    WindieDataDir { path: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// One JSON file written during local MCP component setup.
pub struct WindieSetupFile {
    pub path: String,
    pub content: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// Standard MCP Registry server metadata loaded from `server.json`.
pub struct McpServerManifest {
    #[serde(rename = "$schema", default)]
    pub schema: Option<String>,
    pub name: String,
    pub title: String,
    pub description: String,
    pub version: String,
    #[serde(default)]
    pub remotes: Vec<McpRemote>,
    #[serde(default)]
    pub packages: Vec<McpPackage>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// One remote MCP endpoint from the MCP Registry `remotes` property.
pub struct McpRemote {
    #[serde(rename = "type")]
    pub transport: McpRemoteTransport,
    pub url: String,
    #[serde(default)]
    pub headers: Vec<McpRemoteHeader>,
    #[serde(default)]
    pub variables: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
/// Remote transports currently defined by the MCP Registry.
pub enum McpRemoteTransport {
    StreamableHttp,
    Sse,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// A standard remote header declaration. Secrets are supplied by the host.
pub struct McpRemoteHeader {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(rename = "isRequired", default)]
    pub is_required: bool,
    #[serde(rename = "isSecret", default)]
    pub is_secret: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// A standard MCP Registry package reference.
pub struct McpPackage {
    #[serde(rename = "registryType")]
    pub registry_type: String,
    pub identifier: String,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(rename = "fileSha256", default)]
    pub file_sha256: Option<String>,
    #[serde(rename = "runtimeHint", default)]
    pub runtime_hint: Option<String>,
    pub transport: McpPackageTransport,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// The local transport selected for one standard package reference.
pub struct McpPackageTransport {
    #[serde(rename = "type")]
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// Authentication requirement for one MCP component.
#[serde(tag = "type", rename_all = "snake_case")]
pub enum McpAuthentication {
    None,
    ApiKey {
        required: bool,
        secret_id: String,
        setup_url: Option<String>,
        delivery: McpDelivery,
    },
}

impl Default for McpAuthentication {
    fn default() -> Self {
        Self::None
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// Where Windie delivers a configured MCP secret.
#[serde(tag = "type", rename_all = "snake_case")]
pub enum McpDelivery {
    BearerHeader,
    Environment { name: String },
}

impl PluginManifest {
    /// Parses and validates a plugin manifest document.
    pub fn parse(document: &str) -> Result<Self> {
        let manifest: Self =
            serde_json::from_str(document).context("failed to parse plugin.json")?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// Validates identity and component references.
    pub fn validate(&self) -> Result<()> {
        if self.manifest_version == 0 || self.manifest_version > MAX_MANIFEST_VERSION {
            bail!(
                "unsupported plugin manifest version {}",
                self.manifest_version
            );
        }
        validate_identifier("plugin id", &self.plugin.id)?;
        validate_version(&self.plugin.version)?;
        if self.plugin.publisher.trim().is_empty() {
            bail!("plugin publisher cannot be empty");
        }
        if self.presentation.name.trim().is_empty() {
            bail!("plugin presentation name cannot be empty");
        }
        if self.components.is_empty() {
            bail!("plugin must contain at least one component");
        }

        let mut component_ids = std::collections::HashSet::new();
        for component in &self.components {
            validate_identifier("component id", &component.id)?;
            if !component_ids.insert(&component.id) {
                bail!("plugin contains duplicate component id: {}", component.id);
            }
            validate_relative_path("component manifest", &component.manifest)?;
        }
        Ok(())
    }
}

impl SkillManifest {
    /// Parses a Markdown skill document without executing any package code.
    ///
    /// Windie accepts the common optional YAML-like front matter fields
    /// `name` and `description`. When they are absent, the first heading and
    /// first non-heading paragraph provide stable display fallbacks.
    pub fn parse(document: &str) -> Result<Self> {
        let mut name = None;
        let mut description = None;
        let mut body_start = 0;
        let lines = document.lines().collect::<Vec<_>>();

        if lines.first().is_some_and(|line| line.trim() == "---") {
            let Some(end) = lines.iter().skip(1).position(|line| line.trim() == "---") else {
                bail!("skill front matter is not terminated");
            };
            let end = end + 1;
            for line in &lines[1..end] {
                let Some((key, value)) = line.split_once(':') else {
                    continue;
                };
                match key.trim() {
                    "name" => name = non_empty(value),
                    "description" => description = non_empty(value),
                    _ => {}
                }
            }
            body_start = end + 1;
        }

        for line in &lines[body_start..] {
            let trimmed = line.trim();
            if name.is_none() && trimmed.starts_with('#') {
                name = non_empty(trimmed.trim_start_matches('#'));
                continue;
            }
            if description.is_none() && !trimmed.is_empty() && !trimmed.starts_with('#') {
                description = non_empty(trimmed);
            }
            if name.is_some() && description.is_some() {
                break;
            }
        }

        let name = name.ok_or_else(|| anyhow!("skill is missing a name"))?;
        let description = description.unwrap_or_else(|| name.clone());
        if document.trim().is_empty() {
            bail!("skill instructions cannot be empty");
        }
        Ok(Self {
            name,
            description,
            instructions: document.to_string(),
        })
    }
}

fn non_empty(value: &str) -> Option<String> {
    let value = value.trim().trim_matches('"').trim_matches('\'');
    (!value.is_empty()).then(|| value.to_string())
}

impl Default for WindieMcpMetadata {
    fn default() -> Self {
        Self {
            authentication: McpAuthentication::None,
            permissions: Vec::new(),
            capabilities: Vec::new(),
            startup_timeout_ms: None,
            call_timeout_ms: None,
            readiness_probe: None,
            local_artifact: None,
            setup: WindieMcpSetup::default(),
        }
    }
}

impl McpServerManifest {
    /// Parses and validates standard MCP Registry server metadata.
    pub fn parse(document: &str) -> Result<Self> {
        let manifest: Self =
            serde_json::from_str(document).context("failed to parse MCP server.json")?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// Validates the standard identity and transport declarations Windie uses.
    pub fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty() || self.name.chars().any(char::is_whitespace) {
            bail!("MCP server name cannot be empty or contain whitespace");
        }
        if self.title.trim().is_empty() {
            bail!("MCP server title cannot be empty");
        }
        if self.version.trim().is_empty() {
            bail!("MCP server version cannot be empty");
        }
        if self.remotes.is_empty() && self.packages.is_empty() {
            bail!("MCP server.json must declare a remote or package");
        }
        for remote in &self.remotes {
            let url = &remote.url;
            let parsed = reqwest::Url::parse(url).context("invalid MCP HTTP endpoint")?;
            if parsed.scheme() != "https" && parsed.scheme() != "http" {
                bail!("MCP HTTP endpoint must use http or https");
            }
            for header in &remote.headers {
                if header.name.trim().is_empty() {
                    bail!("MCP remote header name cannot be empty");
                }
            }
        }
        Ok(())
    }

    /// Builds the live transport described by the standard metadata and
    /// Windie-specific policy.
    pub fn transport(&self, windie: &WindieMcpMetadata) -> Result<McpTransport> {
        let remote = self
            .remotes
            .iter()
            .find(|remote| remote.transport == McpRemoteTransport::StreamableHttp)
            .ok_or_else(|| anyhow!("MCP server has no supported Streamable HTTP remote"))?;
        let startup_timeout = windie.startup_timeout_ms.unwrap_or(30_000);
        let call_timeout = windie.call_timeout_ms.unwrap_or(300_000);
        if startup_timeout == 0 || call_timeout == 0 {
            bail!("MCP HTTP timeouts must be greater than zero");
        }

        let authorization = match &windie.authentication {
            McpAuthentication::None => McpHttpAuthorization::Anonymous,
            McpAuthentication::ApiKey {
                required,
                secret_id,
                delivery,
                ..
            } => match delivery {
                McpDelivery::BearerHeader if *required => {
                    McpHttpAuthorization::BearerEnv(secret_id.clone())
                }
                McpDelivery::BearerHeader => {
                    McpHttpAuthorization::OptionalBearerEnv(secret_id.clone())
                }
                McpDelivery::Environment { .. } => {
                    bail!("environment-delivered credentials are not valid for HTTP MCPs")
                }
            },
        };
        Ok(McpTransport::streamable_http(
            McpHttpEndpoint::with_timeouts(
                &remote.url,
                authorization,
                Duration::from_millis(startup_timeout),
                Duration::from_millis(call_timeout),
            ),
        ))
    }

    /// Converts standard server metadata and Windie's policy into Windie's
    /// internal provider projection.
    pub fn provider_manifest(
        &self,
        plugin: &PluginManifest,
        component_id: &str,
        windie: &WindieMcpMetadata,
    ) -> Result<ProviderManifest> {
        let secrets = match &windie.authentication {
            McpAuthentication::None => Vec::new(),
            McpAuthentication::ApiKey {
                required,
                secret_id,
                ..
            } => vec![if *required {
                ProviderSecret::required(secret_id.clone(), "MCP API key")
            } else {
                ProviderSecret::optional(secret_id.clone(), "MCP API key")
            }],
        };
        let authentication = match windie.authentication {
            McpAuthentication::None => ProviderAuthentication::None,
            McpAuthentication::ApiKey { required: true, .. } => ProviderAuthentication::ApiKey,
            McpAuthentication::ApiKey {
                required: false, ..
            } => ProviderAuthentication::OptionalApiKey,
        };
        let remote = self
            .remotes
            .iter()
            .find(|remote| remote.transport == McpRemoteTransport::StreamableHttp);
        let mut manifest = if let Some(remote) = remote {
            ProviderManifest::mcp_streamable_http(
                component_id,
                self.title.clone(),
                self.description.clone(),
                remote.url.clone(),
                ProviderPlatform::desktop(),
                secrets,
                windie.permissions.clone(),
            )
        } else if !self.packages.is_empty() {
            // The actual command is resolved from the installed MCPB at
            // runtime. This placeholder keeps the existing provider catalog
            // contract typed without pretending that an arbitrary marketplace
            // command is safe to launch directly from metadata.
            let mut manifest = ProviderManifest::mcp_stdio(
                component_id,
                self.title.clone(),
                self.description.clone(),
                "packaged-mcp",
                &[],
                ProviderPlatform::desktop(),
                Vec::new(),
                secrets,
                windie.permissions.clone(),
            );
            if let Some(package) = self.packages.iter().find(|package| {
                package.registry_type == "mcpb" && package.transport.kind == "stdio"
            }) {
                match package.runtime_hint.as_deref() {
                    Some("node") => {
                        manifest.runtime = crate::tool::ProviderRuntime::Node;
                        manifest.dependencies = vec![crate::tool::ProviderDependency::executable(
                            "node",
                            "Node.js runtime for the packaged MCP server",
                        )];
                    }
                    Some("uv") => {
                        manifest.runtime = crate::tool::ProviderRuntime::Uv;
                        manifest.dependencies = vec![crate::tool::ProviderDependency::executable(
                            "uv",
                            "uv runtime for the packaged MCP server",
                        )];
                    }
                    Some(runtime) => {
                        bail!("unsupported packaged MCP runtime hint: {runtime}");
                    }
                    None => {}
                }
            }
            if self
                .packages
                .iter()
                .any(|package| package.runtime_hint.as_deref() == Some("uv"))
            {
                manifest.package = Some(ProviderPackage {
                    manager: ProviderPackageManager::Uv,
                    name: self.title.clone(),
                });
            }
            manifest
        } else {
            bail!("MCP server has neither a supported remote nor a package")
        };
        manifest.author = plugin.plugin.publisher.clone();
        manifest.authentication = authentication;
        manifest.scope = if remote.is_some() {
            ProviderScope::Cloud
        } else {
            ProviderScope::Local
        };
        Ok(manifest)
    }
}

impl WindieMcpMetadata {
    /// Validates Windie's policy metadata independently from standard MCP data.
    pub fn validate(&self) -> Result<()> {
        if let McpAuthentication::ApiKey {
            secret_id,
            setup_url,
            delivery,
            ..
        } = &self.authentication
        {
            validate_identifier("secret id", secret_id)?;
            if let Some(url) = setup_url {
                reqwest::Url::parse(url).context("invalid MCP authentication setup URL")?;
            }
            if let McpDelivery::Environment { name } = delivery {
                validate_environment_name(name)?;
            }
        }
        if self.startup_timeout_ms == Some(0) || self.call_timeout_ms == Some(0) {
            bail!("MCP HTTP timeouts must be greater than zero");
        }
        if self
            .readiness_probe
            .as_deref()
            .is_some_and(|probe| probe.trim().is_empty())
        {
            bail!("MCP readiness probe cannot be empty");
        }
        if let Some(path) = &self.local_artifact {
            validate_relative_path("local MCPB artifact", path)?;
        }
        for environment in &self.setup.environment {
            validate_environment_name(&environment.name)?;
            match &environment.value {
                WindieSetupEnvironmentValue::Literal { .. } => {}
                WindieSetupEnvironmentValue::ComponentPath { path }
                | WindieSetupEnvironmentValue::WindieDataDir { path } => {
                    validate_relative_path("local MCP environment path", path)?;
                }
            }
        }
        for file in &self.setup.files {
            validate_relative_path("local MCP setup file", &file.path)?;
        }
        Ok(())
    }
}

pub(crate) fn validate_relative_path(label: &str, value: &str) -> Result<()> {
    let path = Path::new(value);
    if path.is_absolute()
        || value.is_empty()
        || path
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        return Err(anyhow!(
            "{label} must be a relative path without parent traversal: {value}"
        ));
    }
    Ok(())
}

fn validate_identifier(label: &str, value: &str) -> Result<()> {
    if value.is_empty()
        || !value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        })
    {
        bail!("invalid {label}: {value}");
    }
    Ok(())
}

fn validate_version(value: &str) -> Result<()> {
    if value.is_empty() || value.contains("latest") || value.contains('*') {
        bail!("plugin version must be an exact version: {value}");
    }
    Ok(())
}

fn validate_environment_name(value: &str) -> Result<()> {
    if value.is_empty()
        || !value.chars().all(|character| {
            character.is_ascii_uppercase() || character.is_ascii_digit() || character == '_'
        })
    {
        bail!("invalid environment variable name: {value}");
    }
    Ok(())
}

/// Resolves one package-relative path without allowing the package to escape.
pub(crate) fn resolve_package_path(root: &Path, relative: &str) -> Result<PathBuf> {
    validate_relative_path("package path", relative)?;
    Ok(root.join(relative))
}
