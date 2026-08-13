//! Local marketplace metadata for file-based plugins.
//!
//! A marketplace is an index of package sources, not an execution boundary.
//! This module only reads the index and resolves local package paths. The
//! caller still validates every package through [`PluginPackage::load`] before
//! installing or exposing it to the runtime.

use std::fs;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use super::package::{PluginPackage, install_local_package};

/// A marketplace index loaded from a local JSON file.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct MarketplaceManifest {
    /// Human-readable marketplace name.
    pub name: String,
    /// Optional marketplace owner or publisher.
    #[serde(default)]
    pub owner: Option<String>,
    /// Packages listed by the marketplace.
    #[serde(default)]
    pub plugins: Vec<MarketplacePlugin>,
}

impl MarketplaceManifest {
    /// Reads and validates a marketplace index.
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let bytes = fs::read(path)
            .with_context(|| format!("failed to read marketplace: {}", path.display()))?;
        let manifest: Self = serde_json::from_slice(&bytes)
            .with_context(|| format!("failed to parse marketplace: {}", path.display()))?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// Resolves local package entries relative to the marketplace directory.
    /// Remote sources are intentionally not executable through this API.
    pub fn local_package_paths(&self, marketplace_root: impl AsRef<Path>) -> Result<Vec<PathBuf>> {
        let marketplace_root = marketplace_root.as_ref().canonicalize().with_context(|| {
            format!(
                "failed to resolve marketplace directory: {}",
                marketplace_root.as_ref().display()
            )
        })?;
        let mut paths = Vec::new();
        for plugin in &self.plugins {
            let MarketplaceSource::Local { path } = &plugin.source else {
                continue;
            };
            let relative = Path::new(path);
            if relative.is_absolute()
                || relative
                    .components()
                    .any(|component| matches!(component, Component::ParentDir))
            {
                bail!(
                    "marketplace package path must stay inside the marketplace: {}",
                    path
                );
            }
            let package_root = marketplace_root
                .join(relative)
                .canonicalize()
                .with_context(|| format!("failed to resolve marketplace package: {path}"))?;
            if !package_root.starts_with(&marketplace_root) {
                bail!("marketplace package path escapes the marketplace: {path}");
            }
            // Validate before returning the path so callers cannot accidentally
            // install a marketplace entry without the package checks.
            PluginPackage::load(&package_root)?;
            paths.push(package_root);
        }
        Ok(paths)
    }

    /// Installs every local package listed by this marketplace into a package
    /// store. Git entries are metadata only and are skipped.
    pub fn install_local_packages(
        &self,
        marketplace_root: impl AsRef<Path>,
        destination_root: impl AsRef<Path>,
    ) -> Result<Vec<PathBuf>> {
        self.local_package_paths(marketplace_root)?
            .into_iter()
            .map(|path| install_local_package(path, destination_root.as_ref()))
            .collect()
    }

    fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty() {
            bail!("marketplace name cannot be empty");
        }
        for plugin in &self.plugins {
            if plugin.name.trim().is_empty() {
                bail!("marketplace plugin name cannot be empty");
            }
            match &plugin.source {
                MarketplaceSource::Git { url, .. } if url.trim().is_empty() => {
                    bail!("marketplace Git URL cannot be empty: {}", plugin.name);
                }
                MarketplaceSource::Local { .. } | MarketplaceSource::Git { .. } => {}
            }
        }
        Ok(())
    }
}

/// One marketplace package entry.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct MarketplacePlugin {
    /// Marketplace-local display or lookup name.
    pub name: String,
    /// Source from which the package can be resolved.
    pub source: MarketplaceSource,
}

/// Supported package source declarations.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "source", rename_all = "lowercase")]
pub enum MarketplaceSource {
    /// A package directory relative to the marketplace index.
    Local { path: String },
    /// A Git repository source. Fetching is deliberately left to a future
    /// host-owned installer; it is metadata until explicitly resolved.
    Git {
        url: String,
        #[serde(rename = "ref", default)]
        reference: Option<String>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn fixture_root() -> PathBuf {
        std::env::temp_dir().join(format!(
            "windie-marketplace-test-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn parses_local_and_git_sources() {
        let root = fixture_root();
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("marketplace.json"),
            r#"{
                "name":"Local plugins",
                "plugins":[
                    {"name":"demo","source":{"source":"local","path":"demo"}},
                    {"name":"remote","source":{"source":"git","url":"https://example.test/demo.git","ref":"v1"}}
                ]
            }"#,
        )
        .unwrap();

        let manifest = MarketplaceManifest::load(root.join("marketplace.json")).unwrap();
        assert_eq!(manifest.plugins.len(), 2);
        assert_eq!(
            manifest.local_package_paths(&root).unwrap_err().to_string(),
            "failed to resolve marketplace package: demo"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_parent_directory_source() {
        let root = fixture_root();
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("marketplace.json"),
            r#"{"name":"bad","plugins":[{"name":"bad","source":{"source":"local","path":"../outside"}}]}"#,
        )
        .unwrap();
        let manifest = MarketplaceManifest::load(root.join("marketplace.json")).unwrap();
        let error = manifest.local_package_paths(&root).unwrap_err().to_string();
        assert!(error.contains("stay inside"));
        fs::remove_dir_all(root).unwrap();
    }
}
