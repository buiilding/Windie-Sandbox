//! File-based plugin packages.
//!
//! This module reads the Codex-compatible plugin directory shape without
//! executing anything. Package loading is deliberately split from MCP
//! activation: reading `plugin.json`, indexing skills, and validating paths is
//! safe discovery; starting a package-declared server happens only after the
//! plugin is explicitly attached.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::error;
use crate::skills::{SkillDocument, SkillId, SkillPath};
use crate::tool::ToolProviderId;

use super::manifest::{PluginId, PluginVersion};

/// The required manifest location inside a Codex-compatible plugin package.
pub const PLUGIN_MANIFEST_RELATIVE_PATH: &str = ".codex-plugin/plugin.json";

/// A validated plugin package loaded from a local directory.
#[derive(Debug, Clone)]
pub struct PluginPackage {
    root: PathBuf,
    manifest: PackageManifest,
    skills: BTreeMap<SkillId, PackageSkill>,
    mcp_servers: BTreeMap<ToolProviderId, PackageMcpServer>,
    content_hash: String,
}

impl PluginPackage {
    /// Loads and validates one package directory without starting an MCP
    /// process or executing any package-provided file.
    pub fn load(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref().canonicalize().with_context(|| {
            format!(
                "failed to resolve plugin package root: {}",
                root.as_ref().display()
            )
        })?;
        if !root.is_dir() {
            bail!("plugin package root is not a directory: {}", root.display());
        }

        let manifest_path = root.join(PLUGIN_MANIFEST_RELATIVE_PATH);
        let manifest_bytes = fs::read(&manifest_path).with_context(|| {
            format!(
                "failed to read plugin manifest: {}",
                manifest_path.display()
            )
        })?;
        let manifest: PackageManifest = serde_json::from_slice(&manifest_bytes)
            .with_context(|| format!("failed to parse {}", manifest_path.display()))?;
        manifest.validate()?;

        let skills_root = resolve_package_path(&root, &manifest.skills)?;
        let skills = load_skills(&skills_root)?;
        let mcp_servers = load_mcp_servers(&root, manifest.mcp_servers.as_deref())?;
        let content_hash = hash_package(&root)?;

        Ok(Self {
            root,
            manifest,
            skills,
            mcp_servers,
            content_hash,
        })
    }

    /// Returns the package root after canonical path validation.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Returns the stable package identity.
    pub fn plugin_id(&self) -> PluginId {
        PluginId::new(self.manifest.name.clone())
    }

    /// Returns the package version.
    pub fn version(&self) -> PluginVersion {
        PluginVersion::new(self.manifest.version.clone())
    }

    /// Returns the human-facing package description.
    pub fn description(&self) -> &str {
        &self.manifest.description
    }

    /// Returns the declared package owner when the package provides one.
    pub fn author(&self) -> Option<&str> {
        self.manifest.author.as_ref().map(PackageAuthor::name)
    }

    /// Returns the optional marketplace display name.
    pub fn display_name(&self) -> &str {
        self.manifest
            .interface
            .as_ref()
            .and_then(|interface| interface.display_name.as_deref())
            .unwrap_or(&self.manifest.name)
    }

    /// Returns the package content hash used for install/update identity.
    pub fn content_hash(&self) -> &str {
        &self.content_hash
    }

    /// Returns the package-owned skill IDs in deterministic order.
    pub fn skill_ids(&self) -> impl Iterator<Item = &SkillId> {
        self.skills.keys()
    }

    /// Returns the short description for one package-owned skill.
    pub fn skill_description(&self, skill_id: &SkillId) -> Option<&str> {
        self.skills
            .get(skill_id)
            .map(|skill| skill.description.as_str())
    }

    /// Reads one bounded package skill file, defaulting to `SKILL.md`.
    pub fn read_skill(
        &self,
        skill_id: &SkillId,
        requested_path: Option<&SkillPath>,
    ) -> Result<SkillDocument> {
        let skill = self
            .skills
            .get(skill_id)
            .ok_or_else(|| error::not_found(format!("skill does not exist: {skill_id}")))?;
        let path = requested_path
            .cloned()
            .unwrap_or_else(SkillPath::entrypoint);
        if !skill.files.contains(&path) {
            return Err(error::not_found(format!(
                "skill file does not exist: {skill_id}/{path}"
            )));
        }
        let file_path = skill.root.join(path.as_str());
        let content = fs::read_to_string(&file_path)
            .with_context(|| format!("failed to read package skill: {}", file_path.display()))?;
        Ok(SkillDocument {
            skill_id: skill_id.clone(),
            path: path.clone(),
            content,
            available_files: skill
                .files
                .iter()
                .filter(|candidate| **candidate != path)
                .cloned()
                .collect(),
        })
    }

    /// Returns MCP declarations that came from the package's `.mcp.json`.
    pub fn mcp_servers(&self) -> impl Iterator<Item = &PackageMcpServer> {
        self.mcp_servers.values()
    }

    /// Returns one MCP declaration by package server ID.
    pub fn mcp_server(&self, provider_id: &ToolProviderId) -> Option<&PackageMcpServer> {
        self.mcp_servers.get(provider_id)
    }

    /// Converts package metadata into Windie's current plugin composition
    /// manifest. Runtime source and file-backed skill content remain owned by
    /// `PluginPackage`; this value is the shared catalog-facing shape.
    pub fn plugin_manifest(&self) -> super::manifest::PluginManifest {
        super::manifest::PluginManifest {
            plugin_id: self.plugin_id(),
            version: self.version(),
            display_name: self.display_name().to_string(),
            description: self.manifest.description.clone(),
            skills: self.skills.keys().cloned().collect(),
            mcp_servers: self.mcp_servers.keys().cloned().collect(),
        }
    }
}

/// One package-owned skill entrypoint and its bounded directory.
#[derive(Debug, Clone)]
pub struct PackageSkill {
    skill_id: SkillId,
    root: PathBuf,
    entrypoint: PathBuf,
    description: String,
    files: Vec<SkillPath>,
}

/// One package-declared MCP server.
///
/// The declaration is intentionally data-only. The MCP registry decides later
/// whether the command is allowed to start and how its discovered tools enter
/// a conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageMcpServer {
    pub provider_id: ToolProviderId,
    pub command: String,
    pub args: Vec<String>,
    pub cwd: Option<String>,
    pub env_vars: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct PackageManifest {
    name: String,
    version: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    author: Option<PackageAuthor>,
    #[serde(default = "default_skills_path")]
    skills: String,
    #[serde(rename = "mcpServers")]
    mcp_servers: Option<String>,
    #[serde(default)]
    interface: Option<PackageInterface>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum PackageAuthor {
    Name(String),
    Metadata {
        name: String,
        #[serde(default)]
        email: Option<String>,
        #[serde(default)]
        url: Option<String>,
    },
}

impl PackageAuthor {
    fn name(&self) -> &str {
        match self {
            Self::Name(name) | Self::Metadata { name, .. } => name,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct PackageInterface {
    #[serde(rename = "displayName")]
    display_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct McpFile {
    #[serde(rename = "mcpServers", default)]
    mcp_servers: BTreeMap<String, McpFileServer>,
}

#[derive(Debug, Clone, Deserialize)]
struct McpFileServer {
    command: String,
    #[serde(default)]
    args: Vec<String>,
    cwd: Option<String>,
    #[serde(default, alias = "env_vars")]
    env_vars: Vec<String>,
}

fn default_skills_path() -> String {
    "./skills/".to_string()
}

impl PackageManifest {
    fn validate(&self) -> Result<()> {
        validate_identifier("plugin name", &self.name)?;
        validate_identifier("plugin version", &self.version)?;
        if self.skills.trim().is_empty() {
            bail!("plugin skills path cannot be empty");
        }
        Ok(())
    }
}

fn load_skills(skills_root: &Path) -> Result<BTreeMap<SkillId, PackageSkill>> {
    if !skills_root.is_dir() {
        bail!(
            "plugin skills path is not a directory: {}",
            skills_root.display()
        );
    }

    let mut entries = fs::read_dir(skills_root)
        .with_context(|| format!("failed to read plugin skills: {}", skills_root.display()))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());

    let mut skills = BTreeMap::new();
    for entry in entries {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let skill_name = entry.file_name().to_string_lossy().into_owned();
        validate_identifier("skill name", &skill_name)?;
        let entrypoint = path.join("SKILL.md");
        if !entrypoint.is_file() {
            continue;
        }
        let content = fs::read_to_string(&entrypoint).with_context(|| {
            format!(
                "failed to read package skill entrypoint: {}",
                entrypoint.display()
            )
        })?;
        let description = frontmatter_description(&content)
            .unwrap_or_else(|| format!("Instructions provided by the {skill_name} skill."));
        let files = skill_files(&path)?;
        let skill_id = SkillId::new(skill_name);
        if skills
            .insert(
                skill_id.clone(),
                PackageSkill {
                    skill_id,
                    root: path,
                    entrypoint,
                    description,
                    files,
                },
            )
            .is_some()
        {
            bail!("duplicate package skill");
        }
    }

    Ok(skills)
}

fn load_mcp_servers(
    root: &Path,
    relative_path: Option<&str>,
) -> Result<BTreeMap<ToolProviderId, PackageMcpServer>> {
    let Some(relative_path) = relative_path else {
        return Ok(BTreeMap::new());
    };
    let path = resolve_package_path(root, relative_path)?;
    let bytes = fs::read(&path)
        .with_context(|| format!("failed to read MCP declaration: {}", path.display()))?;
    let file: McpFile = serde_json::from_slice(&bytes)
        .with_context(|| format!("failed to parse MCP declaration: {}", path.display()))?;

    let mut servers = BTreeMap::new();
    for (name, server) in file.mcp_servers {
        validate_identifier("MCP server name", &name)?;
        if server.command.trim().is_empty() {
            bail!("MCP server command cannot be empty: {name}");
        }
        if server
            .cwd
            .as_deref()
            .is_some_and(|cwd| cwd != "." && cwd != "./")
        {
            bail!("package MCP server cwd must be the package root (`.`): {name}");
        }
        let provider_id = ToolProviderId::new(name);
        servers.insert(
            provider_id.clone(),
            PackageMcpServer {
                provider_id,
                command: server.command,
                args: server.args,
                cwd: server.cwd,
                env_vars: server.env_vars,
            },
        );
    }
    Ok(servers)
}

fn skill_files(root: &Path) -> Result<Vec<SkillPath>> {
    let mut files = Vec::new();
    collect_skill_files(root, root, &mut files)?;
    files.sort_by(|left, right| left.as_str().cmp(right.as_str()));
    Ok(files)
}

fn collect_skill_files(root: &Path, directory: &Path, files: &mut Vec<SkillPath>) -> Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            bail!("plugin skills cannot contain symlinks: {}", path.display());
        }
        if file_type.is_dir() {
            collect_skill_files(root, &path, files)?;
        } else if file_type.is_file() {
            let relative = path
                .strip_prefix(root)
                .expect("skill file must be below skill root")
                .to_string_lossy()
                .replace('\\', "/");
            files.push(SkillPath::new(relative).map_err(anyhow::Error::msg)?);
        }
    }
    Ok(())
}

fn resolve_package_path(root: &Path, relative: &str) -> Result<PathBuf> {
    let relative_path = Path::new(relative);
    if relative_path.is_absolute()
        || relative_path
            .components()
            .any(|component| matches!(component, Component::ParentDir))
    {
        bail!("package path must be relative and cannot escape the package: {relative}");
    }
    let path = root.join(relative_path);
    if !path.starts_with(root) {
        bail!("package path escapes the package root: {relative}");
    }
    Ok(path)
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

fn frontmatter_description(content: &str) -> Option<String> {
    let mut lines = content.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }
    for line in lines {
        let trimmed = line.trim();
        if trimmed == "---" {
            break;
        }
        if let Some(value) = trimmed.strip_prefix("description:") {
            let value = value.trim().trim_matches(['"', '\'']);
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

fn hash_package(root: &Path) -> Result<String> {
    let mut files = Vec::new();
    collect_files(root, root, &mut files)?;
    files.sort();

    let mut hasher = Sha256::new();
    for path in files {
        let relative = path
            .strip_prefix(root)
            .expect("collected package path must be below package root");
        hasher.update(relative.to_string_lossy().as_bytes());
        hasher.update([0]);
        hasher.update(fs::read(&path)?);
        hasher.update([0]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn collect_files(root: &Path, directory: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    let mut entries = fs::read_dir(directory)
        .with_context(|| format!("failed to read package directory: {}", directory.display()))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            bail!(
                "plugin packages cannot contain symlinks: {}",
                path.display()
            );
        }
        if file_type.is_dir() {
            collect_files(root, &path, files)?;
        } else if file_type.is_file() {
            files.push(path);
        }
    }
    let _ = root;
    Ok(())
}

/// Copies a validated local package into Windie's package directory.
pub fn install_local_package(
    source: impl AsRef<Path>,
    destination_root: impl AsRef<Path>,
) -> Result<PathBuf> {
    let package = PluginPackage::load(&source)?;
    let destination = destination_root
        .as_ref()
        .join(package.plugin_id().as_str())
        .join(package.version().as_str());
    if destination.exists() {
        let installed = PluginPackage::load(&destination).with_context(|| {
            format!(
                "existing installed plugin is invalid: {}",
                destination.display()
            )
        })?;
        if installed.content_hash() != package.content_hash() {
            bail!(
                "plugin destination already contains different content: {}",
                destination.display()
            );
        }
        return Ok(destination);
    }
    copy_directory(package.root(), &destination)?;
    Ok(destination)
}

/// Installs a local package into Windie's user-local package store.
pub fn install_local_package_into_windie(source: impl AsRef<Path>) -> Result<PathBuf> {
    let destination_root = crate::local::windie_home_dir()?.join("plugins");
    install_local_package(source, destination_root)
}

/// Copies a validated directory tree without following symlinks.
pub(crate) fn copy_directory(source: &Path, destination: &Path) -> Result<()> {
    fs::create_dir_all(destination)
        .with_context(|| format!("failed to create package cache: {}", destination.display()))?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            bail!(
                "plugin packages cannot contain symlinks: {}",
                source_path.display()
            );
        }
        if file_type.is_dir() {
            copy_directory(&source_path, &destination_path)?;
        } else if file_type.is_file() {
            fs::copy(&source_path, &destination_path).with_context(|| {
                format!("failed to copy package file {}", source_path.display())
            })?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn fixture_root() -> PathBuf {
        std::env::temp_dir().join(format!(
            "windie-plugin-test-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn write_fixture(root: &Path) {
        fs::create_dir_all(root.join(".codex-plugin")).unwrap();
        fs::create_dir_all(root.join("skills/computer-use/references")).unwrap();
        fs::write(
            root.join(".codex-plugin/plugin.json"),
            r#"{
                "name": "computer-use",
                "version": "1.0.0",
                "description": "Control a computer.",
                "skills": "./skills/",
                "mcpServers": "./.mcp.json",
                "interface": {"displayName": "Computer Use"}
            }"#,
        )
        .unwrap();
        fs::write(
            root.join(".mcp.json"),
            r#"{"mcpServers":{"computer-use":{"command":"./bin/server","args":["mcp"],"env_vars":["TOKEN"]}}}"#,
        )
        .unwrap();
        fs::write(
            root.join("skills/computer-use/SKILL.md"),
            "---\ndescription: Control the computer safely.\n---\nUse the computer.",
        )
        .unwrap();
        fs::write(
            root.join("skills/computer-use/references/MACOS.md"),
            "Mac notes",
        )
        .unwrap();
    }

    #[test]
    fn loads_codex_shaped_package_without_executing_it() {
        let root = fixture_root();
        write_fixture(&root);

        let package = PluginPackage::load(&root).unwrap();
        assert_eq!(package.plugin_id().as_str(), "computer-use");
        assert_eq!(package.display_name(), "Computer Use");
        assert_eq!(package.skill_ids().count(), 1);
        assert!(
            package
                .read_skill(&SkillId::new("computer-use"), None)
                .unwrap()
                .content
                .contains("Use the computer")
        );
        assert_eq!(package.mcp_servers().count(), 1);
        assert!(!package.content_hash().is_empty());

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_parent_directory_paths() {
        let root = fixture_root();
        write_fixture(&root);
        let manifest_path = root.join(".codex-plugin/plugin.json");
        fs::write(
            &manifest_path,
            r#"{"name":"bad","version":"1","skills":"../outside"}"#,
        )
        .unwrap();

        let error = PluginPackage::load(&root).unwrap_err().to_string();
        assert!(error.contains("cannot escape"));
        fs::remove_dir_all(root).unwrap();
    }
}
