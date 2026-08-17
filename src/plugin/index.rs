//! Marketplace index contracts.
//!
//! The index is a discovery and distribution catalog for plugins. It is not
//! the runtime source of truth; Windie loads and validates the plugin manifest
//! from the verified plugin artifact before activation.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use super::manifest::validate_relative_path;

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
