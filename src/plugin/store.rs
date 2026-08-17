//! Windie-owned plugin package storage.
//!
//! Plugin installation copies a validated package directory or verified
//! marketplace archive into an immutable versioned store.

use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use flate2::read::GzDecoder;
use tar::Archive;
use zip::ZipArchive;

use crate::mcp::McpTransport;

use super::manifest::{
    McpServerManifest, PluginComponentKind, PluginManifest, WindieMcpMetadata, resolve_package_path,
};
use super::mcpb::McpbRuntime;

#[derive(Debug, Clone)]
/// One plugin loaded from an installed package directory.
pub struct InstalledPlugin {
    pub root: PathBuf,
    pub manifest: PluginManifest,
}

impl InstalledPlugin {
    /// Loads and validates a plugin package directory.
    pub fn load(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        let manifest_path = root.join("plugin.json");
        let manifest = PluginManifest::parse(
            &fs::read_to_string(&manifest_path)
                .with_context(|| format!("failed to read {}", manifest_path.display()))?,
        )?;

        for path in [&manifest.presentation.readme, &manifest.presentation.icon] {
            let path = resolve_package_path(&root, path)?;
            if !path.is_file() {
                bail!(
                    "plugin presentation asset does not exist: {}",
                    path.display()
                );
            }
        }

        for component in &manifest.components {
            let component_path = resolve_package_path(&root, &component.manifest)?;
            if !component_path.is_file() {
                bail!(
                    "plugin component manifest does not exist: {}",
                    component.manifest
                );
            }
            if component.kind == PluginComponentKind::Mcp {
                McpServerManifest::parse(&fs::read_to_string(&component_path)?)?;
                component.windie.validate()?;
                if let Some(artifact) = &component.windie.local_artifact {
                    let artifact_path = resolve_package_path(&root, artifact)?;
                    if !artifact_path.is_file() {
                        bail!("local MCPB artifact does not exist: {artifact}");
                    }
                }
            }
        }

        Ok(Self { root, manifest })
    }

    /// Loads the MCP components contained by this plugin.
    pub fn mcp_components(&self) -> Result<Vec<LoadedMcpComponent>> {
        self.manifest
            .components
            .iter()
            .filter(|component| component.kind == PluginComponentKind::Mcp)
            .map(|component| {
                let path = resolve_package_path(&self.root, &component.manifest)?;
                let manifest = McpServerManifest::parse(&fs::read_to_string(&path)?)?;
                let packaged_runtime = match &component.windie.local_artifact {
                    Some(artifact) => {
                        let package = manifest
                            .packages
                            .iter()
                            .find(|package| package.registry_type == "mcpb")
                            .ok_or_else(|| {
                                anyhow!(
                                    "local MCPB component {} has no mcpb package reference",
                                    component.id
                                )
                            })?;
                        if package.transport.kind != "stdio" {
                            bail!("local MCPB component transport must be stdio");
                        }
                        let digest = package
                            .file_sha256
                            .as_deref()
                            .ok_or_else(|| anyhow!("local MCPB package is missing fileSha256"))?;
                        let artifact_path = resolve_package_path(&self.root, artifact)?;
                        let runtime_root = self.root.join("runtime").join(&component.id);
                        Some(McpbRuntime::install(&artifact_path, &runtime_root, digest)?)
                    }
                    None => None,
                };
                let package_command = packaged_runtime
                    .as_ref()
                    .map(|runtime| runtime.package_command(&component.windie.setup))
                    .transpose()?
                    .flatten();
                let transport = match packaged_runtime {
                    Some(runtime) => McpTransport::PackagedStdio {
                        command: runtime
                            .prepare(&component.windie.setup, &component.windie.authentication)?,
                        shutdown_command: None,
                    },
                    None => manifest.transport(&component.windie)?,
                };
                let readme_path =
                    resolve_package_path(&self.root, &self.manifest.presentation.readme)?;
                let readme = fs::read_to_string(readme_path).unwrap_or_default();
                Ok(LoadedMcpComponent {
                    plugin: self.manifest.clone(),
                    component_id: component.id.clone(),
                    manifest,
                    windie: component.windie.clone(),
                    transport,
                    package_command,
                    readme,
                })
            })
            .collect()
    }
}

#[derive(Debug, Clone)]
/// One validated MCP component plus its containing plugin metadata.
pub struct LoadedMcpComponent {
    pub plugin: PluginManifest,
    pub component_id: String,
    pub manifest: McpServerManifest,
    pub windie: WindieMcpMetadata,
    pub transport: McpTransport,
    pub package_command: Option<crate::mcp::McpOwnedCommand>,
    pub readme: String,
}

#[derive(Debug, Clone)]
/// Filesystem-backed Windie plugin store.
pub struct PluginStore {
    root: PathBuf,
}

impl PluginStore {
    /// Creates a plugin store rooted at a caller-controlled directory.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Returns the default user-local plugin store.
    pub fn default_store() -> Result<Self> {
        Ok(Self::new(crate::local::windie_home_dir()?.join("plugins")))
    }

    /// Returns the package root used by this store.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Installs one plugin from Windie's checked-in bundled package directory.
    pub fn install_bundled(&self, plugin_id: &str) -> Result<InstalledPlugin> {
        let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("packages")
            .join(plugin_id);
        self.install_from_directory(source)
    }

    /// Installs a validated plugin directory into the versioned store.
    pub fn install_from_directory(&self, source: impl AsRef<Path>) -> Result<InstalledPlugin> {
        let source = source.as_ref();
        let plugin = InstalledPlugin::load(source)?;
        let staging = self.new_staging_path()?;
        let result = (|| {
            copy_directory(source, &staging)?;
            self.publish_staging(staging.as_path(), &plugin)
        })();
        if result.is_err() {
            let _ = fs::remove_dir_all(&staging);
        }
        result
    }

    /// Installs a verified plugin archive with no marketplace identity check.
    pub fn install_from_archive(
        &self,
        bytes: &[u8],
        archive_name: &str,
    ) -> Result<InstalledPlugin> {
        self.install_from_archive_internal(bytes, archive_name, None)
    }

    /// Installs a verified archive and confirms its manifest matches the
    /// marketplace release before publishing it into the store.
    pub(crate) fn install_from_archive_checked(
        &self,
        bytes: &[u8],
        archive_name: &str,
        expected_id: &str,
        expected_version: &str,
        expected_publisher: &str,
    ) -> Result<InstalledPlugin> {
        self.install_from_archive_internal(
            bytes,
            archive_name,
            Some((expected_id, expected_version, expected_publisher)),
        )
    }

    fn install_from_archive_internal(
        &self,
        bytes: &[u8],
        archive_name: &str,
        expected: Option<(&str, &str, &str)>,
    ) -> Result<InstalledPlugin> {
        let staging = self.new_staging_path()?;
        let result = (|| {
            extract_archive(bytes, archive_name, &staging)?;
            let plugin = InstalledPlugin::load(&staging)?;
            if let Some((expected_id, expected_version, expected_publisher)) = expected
                && (plugin.manifest.plugin.id != expected_id
                    || plugin.manifest.plugin.version != expected_version
                    || plugin.manifest.plugin.publisher != expected_publisher)
            {
                bail!("plugin artifact identity does not match its marketplace release");
            }
            self.publish_staging(&staging, &plugin)
        })();
        if result.is_err() {
            let _ = fs::remove_dir_all(&staging);
        }
        result
    }

    fn new_staging_path(&self) -> Result<PathBuf> {
        fs::create_dir_all(&self.root).with_context(|| {
            format!(
                "failed to create plugin store directory {}",
                self.root.display()
            )
        })?;
        Ok(self.root.join(format!(".staging-{}", uuid::Uuid::new_v4())))
    }

    fn publish_staging(&self, staging: &Path, plugin: &InstalledPlugin) -> Result<InstalledPlugin> {
        let destination = self
            .root
            .join(&plugin.manifest.plugin.id)
            .join(&plugin.manifest.plugin.version);
        if destination.exists() {
            fs::remove_dir_all(staging)?;
            return InstalledPlugin::load(destination);
        }

        let parent = destination
            .parent()
            .ok_or_else(|| anyhow!("plugin destination has no parent"))?;
        fs::create_dir_all(parent)?;
        fs::rename(staging, &destination).with_context(|| {
            format!("failed to publish plugin package {}", destination.display())
        })?;
        InstalledPlugin::load(destination)
    }

    /// Loads all installed plugin versions in deterministic order.
    pub fn installed_plugins(&self) -> Result<Vec<InstalledPlugin>> {
        if !self.root.exists() {
            return Ok(Vec::new());
        }
        let mut plugins = Vec::new();
        for plugin_entry in fs::read_dir(&self.root)? {
            let plugin_entry = plugin_entry?;
            if !plugin_entry.file_type()?.is_dir() {
                continue;
            }
            for version_entry in fs::read_dir(plugin_entry.path())? {
                let version_entry = version_entry?;
                if version_entry.file_type()?.is_dir()
                    && !version_entry.file_name().to_string_lossy().starts_with('.')
                {
                    plugins.push(InstalledPlugin::load(version_entry.path())?);
                }
            }
        }
        plugins.sort_by(|left, right| {
            left.manifest
                .plugin
                .id
                .cmp(&right.manifest.plugin.id)
                .then_with(|| {
                    left.manifest
                        .plugin
                        .version
                        .cmp(&right.manifest.plugin.version)
                })
        });
        Ok(plugins)
    }

    /// Removes every installed release of one plugin and returns the packages
    /// that were removed so the live registry can discard their providers.
    ///
    /// The method removes only roots discovered beneath this store and whose
    /// validated manifest has the requested exact ID. It never constructs a
    /// filesystem target directly from caller input.
    pub fn remove_plugin(&self, plugin_id: &str) -> Result<Vec<InstalledPlugin>> {
        let plugins = self
            .installed_plugins()?
            .into_iter()
            .filter(|plugin| plugin.manifest.plugin.id == plugin_id)
            .collect::<Vec<_>>();
        for plugin in &plugins {
            fs::remove_dir_all(&plugin.root).with_context(|| {
                format!("failed to remove plugin package {}", plugin.root.display())
            })?;
        }
        if let Some(plugin_root) = plugins.first().and_then(|plugin| plugin.root.parent())
            && fs::read_dir(plugin_root)?.next().is_none()
        {
            fs::remove_dir(plugin_root)?;
        }
        Ok(plugins)
    }
}

fn copy_directory(source: &Path, destination: &Path) -> Result<()> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_directory(&source_path, &destination_path)?;
        } else if entry.file_type()?.is_file() {
            fs::copy(&source_path, &destination_path)?;
        } else {
            bail!(
                "plugin package contains unsupported filesystem entry: {}",
                source_path.display()
            );
        }
    }
    Ok(())
}

fn extract_archive(bytes: &[u8], archive_name: &str, destination: &Path) -> Result<()> {
    if archive_name.ends_with(".tar.gz") || archive_name.ends_with(".tgz") {
        let decoder = GzDecoder::new(Cursor::new(bytes));
        let mut archive = Archive::new(decoder);
        for entry in archive.entries().context("failed to read tar archive")? {
            let mut entry = entry.context("failed to read tar archive entry")?;
            let entry_type = entry.header().entry_type();
            if !entry_type.is_file() && !entry_type.is_dir() {
                bail!("plugin archive contains an unsupported filesystem entry");
            }
            let path = archive_path(destination, &entry.path()?)?;
            if entry_type.is_dir() {
                fs::create_dir_all(path)?;
            } else {
                let parent = path
                    .parent()
                    .ok_or_else(|| anyhow!("plugin archive file has no parent"))?;
                fs::create_dir_all(parent)?;
                let mut output = fs::File::create(path)?;
                std::io::copy(&mut entry, &mut output)?;
            }
        }
        return Ok(());
    }

    if archive_name.ends_with(".zip") {
        let mut archive =
            ZipArchive::new(Cursor::new(bytes)).context("failed to read zip plugin archive")?;
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index)?;
            let path = archive_path(destination, Path::new(entry.name()))?;
            if entry.is_dir() {
                fs::create_dir_all(path)?;
            } else {
                if entry
                    .unix_mode()
                    .is_some_and(|mode| mode & 0o170000 == 0o120000)
                {
                    bail!("plugin archive contains a symbolic link");
                }
                let parent = path
                    .parent()
                    .ok_or_else(|| anyhow!("plugin archive file has no parent"))?;
                fs::create_dir_all(parent)?;
                let mut output = fs::File::create(path)?;
                std::io::copy(&mut entry, &mut output)?;
            }
        }
        return Ok(());
    }

    bail!("unsupported plugin archive format: {archive_name}")
}

fn archive_path(destination: &Path, path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        bail!("plugin archive contains an absolute path");
    }
    let mut relative = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::Normal(part) => relative.push(part),
            std::path::Component::ParentDir
            | std::path::Component::RootDir
            | std::path::Component::Prefix(_) => {
                bail!("plugin archive contains a path traversal")
            }
        }
    }
    if relative.as_os_str().is_empty() {
        bail!("plugin archive contains an empty path");
    }
    let path = destination.join(relative);
    if !path.starts_with(destination) {
        bail!("plugin archive path escapes its staging directory");
    }
    Ok(path)
}
