//! Marketplace index contracts.
//!
//! The index is a discovery and distribution catalog for plugins. It is not
//! the runtime source of truth; Windie loads and validates the plugin manifest
//! from the verified plugin artifact before activation.

use std::collections::HashSet;
use std::fmt;
use std::sync::{Arc, RwLock};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::store::Store;
use crate::tool::{
    ProviderInstallState, ProviderReadiness, ToolDefinition, ToolProviderId, ToolProviderRegistry,
};

use super::manifest::{validate_github_repository_url, validate_relative_path};
use super::{InstalledPlugin, PluginStore};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// A versioned marketplace index.
pub struct MarketplaceIndex {
    pub index_version: u32,
    pub plugins: Vec<MarketplacePlugin>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// One plugin listing in the marketplace.
pub struct MarketplacePlugin {
    pub id: String,
    pub versions: Vec<MarketplaceVersion>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// One immutable plugin release reference.
pub struct MarketplaceVersion {
    pub version: String,
    pub components: Vec<String>,
    pub capabilities: Vec<String>,
    /// Generated presentation summary used by marketplace clients.
    ///
    /// The installed plugin manifest remains authoritative. These fields are
    /// denormalized discovery data so clients can render a catalog without
    /// downloading the plugin artifact first.
    #[serde(default)]
    pub presentation: Option<MarketplacePresentation>,
    pub manifest_url: String,
    pub artifact_url: String,
    pub digest: String,
    pub publisher: String,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// User-facing plugin metadata copied from the plugin manifest for discovery.
pub struct MarketplacePresentation {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub readme_url: Option<String>,
    #[serde(default)]
    pub icon_url: Option<String>,
    #[serde(default)]
    pub repository_url: Option<String>,
}

/// The only catalog snapshot exposed to the model.
///
/// This contains discovery metadata only. MCP schemas and complete skill
/// instructions are loaded through explicit built-in tools when the model
/// needs them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginIndex {
    pub installed: Vec<PluginSummary>,
    pub available: Vec<PluginSummary>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
/// A device's model-facing index plus its current executable provider mapping.
///
/// The compact index remains discovery-only. The provider entries carry the
/// exact schema-to-local-provider facts needed to validate a later hosted
/// attachment without exposing package paths or command lines.
pub struct PluginCapabilitySnapshot {
    pub index: PluginIndex,
    pub providers: Vec<PluginProviderCapability>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
/// Current persisted catalog facts for one installed MCP component.
pub struct PluginProviderCapability {
    pub plugin_id: String,
    pub component_id: String,
    pub provider_id: ToolProviderId,
    pub state: PluginState,
    pub tools: Vec<ToolDefinition>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// One plugin entry in the model-facing index.
pub struct PluginSummary {
    pub id: String,
    pub name: String,
    pub purpose: String,
    pub version: Option<String>,
    pub state: PluginState,
    pub component_kinds: Vec<String>,
    pub capabilities: Vec<String>,
    pub skills: Vec<SkillSummary>,
    pub mcps: Vec<McpSummary>,
    pub apps: Vec<AppSummary>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Aggregate plugin state shown in the compact index.
pub enum PluginState {
    Available,
    Installed,
    Enabled,
    Disabled,
    Broken,
    Updating,
    Unavailable,
}

impl fmt::Display for PluginState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Available => "available",
            Self::Installed => "installed",
            Self::Enabled => "enabled",
            Self::Disabled => "disabled",
            Self::Broken => "broken",
            Self::Updating => "updating",
            Self::Unavailable => "unavailable",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// Metadata for one skill nested inside a plugin.
pub struct SkillSummary {
    pub id: String,
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// Metadata and runtime state for one MCP nested inside a plugin.
pub struct McpSummary {
    pub id: String,
    pub name: String,
    pub purpose: String,
    pub state: PluginState,
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// Metadata for one app connector nested inside a plugin.
pub struct AppSummary {
    pub id: String,
    pub name: String,
    pub purpose: String,
    pub state: PluginState,
}

/// Shared plugin catalog dependency used by API and runtime sessions.
///
/// The marketplace release list is refreshed by the API discovery route. The
/// installed package store is read on every snapshot, so installation and
/// removal are reflected in the next model turn without persisting an index.
#[derive(Debug, Clone)]
pub struct PluginCatalog {
    plugin_store: Arc<PluginStore>,
    marketplace: Arc<RwLock<MarketplaceIndex>>,
}

impl PluginCatalog {
    /// Creates a catalog from the current marketplace snapshot and package store.
    pub fn new(plugin_store: Arc<PluginStore>, marketplace: MarketplaceIndex) -> Self {
        Self {
            plugin_store,
            marketplace: Arc::new(RwLock::new(marketplace)),
        }
    }

    /// Replaces the marketplace snapshot after a successful index refresh.
    pub fn replace_marketplace(&self, marketplace: MarketplaceIndex) {
        *self
            .marketplace
            .write()
            .expect("plugin marketplace lock poisoned") = marketplace;
    }

    /// Returns the package store used by this catalog.
    pub fn plugin_store(&self) -> &Arc<PluginStore> {
        &self.plugin_store
    }

    /// Builds the current model-facing index from local packages and SQLite state.
    pub fn build_index(
        &self,
        store: &Store,
        registry: &ToolProviderRegistry,
    ) -> Result<PluginIndex> {
        Ok(self.build_capability_snapshot(store, registry)?.index)
    }

    /// Builds both the compact index and the current executable provider facts
    /// from the same package and SQLite inputs. This never starts discovery or
    /// an MCP process; it reports only already persisted catalogs.
    pub fn build_capability_snapshot(
        &self,
        store: &Store,
        _registry: &ToolProviderRegistry,
    ) -> Result<PluginCapabilitySnapshot> {
        let installed_plugins = self.plugin_store.installed_plugins()?;
        let installed_plugins = installed_plugins.into_iter().fold(
            Vec::<InstalledPlugin>::new(),
            |mut plugins, plugin| {
                if let Some(existing) = plugins
                    .iter_mut()
                    .find(|existing| existing.manifest.plugin.id == plugin.manifest.plugin.id)
                {
                    if existing.manifest.plugin.version < plugin.manifest.plugin.version {
                        *existing = plugin;
                    }
                } else {
                    plugins.push(plugin);
                }
                plugins
            },
        );
        let installed = installed_plugins
            .iter()
            .map(|plugin| installed_summary(plugin, store))
            .collect::<Result<Vec<_>>>()?;
        let providers = installed_plugins
            .iter()
            .map(|plugin| provider_capabilities(plugin, store))
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .flatten()
            .collect();

        let marketplace = self
            .marketplace
            .read()
            .expect("plugin marketplace lock poisoned")
            .clone();
        Ok(PluginCapabilitySnapshot {
            index: PluginIndex::from_installed(installed, marketplace),
            providers,
        })
    }

    /// Renders the compact plugin index inserted into each model request.
    pub fn compact_index(&self, store: &Store, registry: &ToolProviderRegistry) -> Result<String> {
        let index = self.build_index(store, registry)?;
        Ok(index.render())
    }

    /// Reads one installed skill after validating both plugin and component IDs.
    pub fn read_skill(&self, plugin_id: &str, skill_id: &str) -> Result<String> {
        let plugin = self
            .plugin_store
            .installed_plugin(plugin_id)?
            .ok_or_else(|| anyhow::anyhow!("installed plugin does not exist: {plugin_id}"))?;
        plugin.read_skill(skill_id)
    }
}

fn installed_summary(plugin: &InstalledPlugin, store: &Store) -> Result<PluginSummary> {
    let skills = plugin
        .skills()?
        .into_iter()
        .map(|skill| SkillSummary {
            id: skill.id,
            name: skill.name,
            description: skill.description,
        })
        .collect::<Vec<_>>();
    let mcps = plugin
        .mcp_metadata()?
        .into_iter()
        .map(|mcp| {
            let state = component_state(store, &mcp.id)?;
            Ok(McpSummary {
                id: mcp.id,
                name: mcp.name,
                purpose: mcp.purpose,
                state,
                capabilities: mcp.capabilities,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let apps = plugin
        .app_metadata()?
        .into_iter()
        .map(|app| AppSummary {
            id: app.id,
            name: app.name,
            purpose: app.purpose,
            state: PluginState::Installed,
        })
        .collect::<Vec<_>>();
    Ok(project_installed_plugin(
        &plugin.manifest,
        skills,
        mcps,
        apps,
    ))
}

fn provider_capabilities(
    plugin: &InstalledPlugin,
    store: &Store,
) -> Result<Vec<PluginProviderCapability>> {
    plugin
        .mcp_metadata()?
        .into_iter()
        .map(|mcp| {
            let provider_id = ToolProviderId::new(&mcp.id);
            let state = component_state(store, &mcp.id)?;
            let tools = match store.load_provider_tool_catalog(&provider_id)? {
                Some(catalog)
                    if state == PluginState::Enabled
                        && catalog.status == crate::store::ProviderCatalogStatus::Fresh =>
                {
                    catalog.tools
                }
                _ => Vec::new(),
            };
            Ok(PluginProviderCapability {
                plugin_id: plugin.manifest.plugin.id.clone(),
                component_id: mcp.id,
                provider_id,
                state,
                tools,
            })
        })
        .collect()
}

/// Projects package metadata and loaded component facts without reading a file
/// or database. Local catalog loading and device reports use this same shape.
pub fn project_installed_plugin(
    manifest: &super::PluginManifest,
    skills: Vec<SkillSummary>,
    mcps: Vec<McpSummary>,
    apps: Vec<AppSummary>,
) -> PluginSummary {
    let state = aggregate_plugin_state(&mcps, !skills.is_empty() || !apps.is_empty());
    let component_kinds = manifest
        .components
        .iter()
        .map(|component| component.kind.to_string())
        .collect();
    let capabilities = manifest
        .components
        .iter()
        .flat_map(|component| component.windie.capabilities.iter().cloned())
        .collect::<Vec<_>>();

    PluginSummary {
        id: manifest.plugin.id.clone(),
        name: manifest.presentation.name.clone(),
        purpose: manifest.presentation.description.clone(),
        version: Some(manifest.plugin.version.clone()),
        state,
        component_kinds,
        capabilities,
        skills,
        mcps,
        apps,
    }
}

impl PluginIndex {
    /// Combines authoritative installed facts with discovery-only marketplace
    /// entries. A marketplace listing never grants executable capabilities.
    pub fn from_installed(
        mut installed: Vec<PluginSummary>,
        marketplace: MarketplaceIndex,
    ) -> Self {
        let installed_ids = installed
            .iter()
            .map(|plugin| plugin.id.clone())
            .collect::<HashSet<_>>();
        let mut available = marketplace
            .plugins
            .into_iter()
            .filter(|plugin| !installed_ids.contains(&plugin.id))
            .map(available_summary)
            .collect::<Vec<_>>();
        installed.sort_by(|left, right| left.id.cmp(&right.id));
        available.sort_by(|left, right| left.id.cmp(&right.id));
        Self {
            installed,
            available,
        }
    }

    /// Produces exactly the compact metadata used by the local model compiler.
    /// Full skill instructions and MCP schemas are deliberately absent.
    pub fn render(&self) -> String {
        render_compact_index(self)
    }
}

fn available_summary(plugin: MarketplacePlugin) -> PluginSummary {
    let release = plugin.versions.first();
    let presentation = release.and_then(|release| release.presentation.as_ref());
    PluginSummary {
        id: plugin.id,
        name: presentation
            .map(|presentation| presentation.name.clone())
            .unwrap_or_default(),
        purpose: presentation
            .map(|presentation| presentation.description.clone())
            .unwrap_or_default(),
        version: release.map(|release| release.version.clone()),
        state: PluginState::Available,
        component_kinds: release
            .map(|release| release.components.clone())
            .unwrap_or_default(),
        capabilities: release
            .map(|release| release.capabilities.clone())
            .unwrap_or_default(),
        skills: Vec::new(),
        mcps: Vec::new(),
        apps: Vec::new(),
    }
}

fn component_state(store: &Store, provider_id: &str) -> Result<PluginState> {
    let provider_id = ToolProviderId::new(provider_id);
    if store
        .load_provider_tool_catalog(&provider_id)?
        .is_some_and(|catalog| catalog.status == crate::store::ProviderCatalogStatus::Unavailable)
    {
        return Ok(PluginState::Unavailable);
    }
    let Some(provider) = store.load_installed_provider(&provider_id)? else {
        return Ok(PluginState::Installed);
    };
    if provider.error.is_some() || provider.state == ProviderInstallState::Broken {
        return Ok(PluginState::Broken);
    }
    if provider.state == ProviderInstallState::Disabled {
        return Ok(PluginState::Disabled);
    }
    if provider.state == ProviderInstallState::Updating {
        return Ok(PluginState::Updating);
    }
    if provider.state == ProviderInstallState::Enabled
        && provider.readiness != ProviderReadiness::Ready
    {
        return Ok(PluginState::Unavailable);
    }
    Ok(match provider.state {
        ProviderInstallState::Enabled => PluginState::Enabled,
        _ => PluginState::Installed,
    })
}

fn aggregate_plugin_state(mcps: &[McpSummary], has_non_mcp_components: bool) -> PluginState {
    if mcps.iter().any(|mcp| mcp.state == PluginState::Broken) {
        return PluginState::Broken;
    }
    if mcps.iter().any(|mcp| mcp.state == PluginState::Unavailable) {
        return PluginState::Unavailable;
    }
    if mcps.iter().any(|mcp| mcp.state == PluginState::Disabled) {
        return PluginState::Disabled;
    }
    if mcps.iter().any(|mcp| mcp.state == PluginState::Updating) {
        return PluginState::Updating;
    }
    if mcps.iter().any(|mcp| mcp.state == PluginState::Enabled) || has_non_mcp_components {
        return PluginState::Enabled;
    }
    PluginState::Installed
}

fn render_compact_index(index: &PluginIndex) -> String {
    let mut output = String::from("Installed plugins:\n");
    render_plugin_group(&mut output, &index.installed, true);
    output.push_str("\nAvailable plugins:\n");
    render_plugin_group(&mut output, &index.available, false);
    output
}

fn render_plugin_group(output: &mut String, plugins: &[PluginSummary], installed: bool) {
    if plugins.is_empty() {
        output.push_str("\n- none\n");
        return;
    }
    for plugin in plugins {
        output.push_str(&format!("\n- {}\n", plugin.id));
        if !plugin.name.is_empty() && plugin.name != plugin.id {
            output.push_str(&format!("  name: {}\n", plugin.name));
        }
        output.push_str(&format!("  purpose: {}\n", plugin.purpose));
        if installed {
            output.push_str(&format!("  state: {}\n", plugin.state));
            render_skills(output, &plugin.skills);
            render_mcps(output, &plugin.mcps);
            render_apps(output, &plugin.apps);
        } else {
            if let Some(version) = &plugin.version {
                output.push_str(&format!("  version: {version}\n"));
            }
            output.push_str(&format!(
                "  components: {}\n",
                plugin.component_kinds.join(", ")
            ));
            if !plugin.capabilities.is_empty() {
                output.push_str(&format!(
                    "  capabilities: {}\n",
                    plugin.capabilities.join(", ")
                ));
            }
        }
    }
}

fn render_skills(output: &mut String, skills: &[SkillSummary]) {
    if skills.is_empty() {
        return;
    }
    output.push_str("\n  skills:\n");
    for skill in skills {
        output.push_str(&format!("  - {}: {}\n", skill.id, skill.description));
    }
}

fn render_mcps(output: &mut String, mcps: &[McpSummary]) {
    if mcps.is_empty() {
        return;
    }
    output.push_str("\n  mcps:\n");
    for mcp in mcps {
        output.push_str(&format!("  - {}: {}\n", mcp.id, mcp.purpose));
        output.push_str(&format!("    state: {}\n", mcp.state));
    }
}

fn render_apps(output: &mut String, apps: &[AppSummary]) {
    if apps.is_empty() {
        return;
    }
    output.push_str("\n  apps:\n");
    for app in apps {
        output.push_str(&format!("  - {}: {}\n", app.id, app.purpose));
        output.push_str(&format!("    state: {}\n", app.state));
    }
}

impl MarketplaceIndex {
    /// Parses and validates a marketplace index document.
    pub fn parse(document: &str) -> Result<Self> {
        let index: Self =
            serde_json::from_str(document).context("failed to parse marketplace index")?;
        index.validate()?;
        Ok(index)
    }

    /// Validates plugin IDs, releases, and artifact references.
    pub fn validate(&self) -> Result<()> {
        if self.index_version != 1 {
            bail!(
                "unsupported marketplace index version {}",
                self.index_version
            );
        }
        for plugin in &self.plugins {
            if plugin.id.is_empty() {
                bail!("marketplace plugin id cannot be empty");
            }
            for version in &plugin.versions {
                if version.version.is_empty() || version.digest.is_empty() {
                    bail!("marketplace plugin release is missing version or digest");
                }
                if version.publisher.trim().is_empty() {
                    bail!("marketplace plugin release is missing publisher");
                }
                if version.status.trim().is_empty() {
                    bail!("marketplace plugin release is missing status");
                }
                if let Some(repository_url) = version
                    .presentation
                    .as_ref()
                    .and_then(|presentation| presentation.repository_url.as_deref())
                {
                    validate_github_repository_url(repository_url)?;
                }
                validate_reference("marketplace manifest URL", &version.manifest_url)?;
                validate_reference("marketplace artifact URL", &version.artifact_url)?;
            }
        }
        Ok(())
    }

    /// Finds one plugin listing by its stable marketplace ID.
    pub fn plugin(&self, plugin_id: &str) -> Result<&MarketplacePlugin> {
        self.plugins
            .iter()
            .find(|plugin| plugin.id == plugin_id)
            .ok_or_else(|| anyhow::anyhow!("plugin is not listed in the marketplace: {plugin_id}"))
    }

    /// Selects the first release listed for one plugin.
    ///
    /// Marketplace publishers must order releases newest-first. A future
    /// index version can replace this with an explicit version constraint.
    pub fn latest_version(&self, plugin_id: &str) -> Result<&MarketplaceVersion> {
        self.plugin(plugin_id)?
            .versions
            .first()
            .ok_or_else(|| anyhow::anyhow!("plugin has no published versions: {plugin_id}"))
    }
}

fn validate_reference(label: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        bail!("{label} cannot be empty");
    }
    if let Ok(url) = reqwest::Url::parse(value) {
        if url.scheme() != "https" && url.scheme() != "http" {
            bail!("{label} must use http or https");
        }
        return Ok(());
    }
    validate_relative_path(label, value)
}

/// Loads Windie's checked-in bundled marketplace index.
pub fn bundled() -> Result<MarketplaceIndex> {
    MarketplaceIndex::parse(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/marketplace/index.json"
    )))
}
