//! Read-only plugin inspection operations.
//!
//! The model reads one skill file at a time through the runtime skill tool.
//! Inspector needs a different view: a human-facing page that can show every
//! Markdown file installed in a plugin package. These operations keep that
//! file enumeration out of the HTTP handler and preserve the package registry
//! as the source of truth.

use anyhow::Result;
use serde::Serialize;

use crate::plugins::{ExtensionComposition, PluginId, PluginManifest, PluginRegistry};
use crate::skills::SkillId;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
/// One Markdown file in an installed plugin skill bundle.
pub struct PluginSkillFile {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
/// One skill and all of its installed Markdown files.
pub struct PluginSkillFiles {
    pub skill_id: SkillId,
    pub description: String,
    pub files: Vec<PluginSkillFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
/// Complete skill content for one installed plugin.
pub struct PluginSkillsResponse {
    pub plugin_id: PluginId,
    pub display_name: String,
    pub version: crate::plugins::PluginVersion,
    pub skills: Vec<PluginSkillFiles>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
/// One package-backed extension for the Inspector catalog.
pub struct ExtensionInstallation {
    pub id: PluginId,
    pub display_name: String,
    pub description: String,
    pub version: crate::plugins::PluginVersion,
    pub composition: ExtensionComposition,
    pub skills: Vec<SkillId>,
    pub mcp_servers: Vec<crate::tool::ToolProviderId>,
    pub installed: bool,
}

/// Lists curated definitions and installed package extensions.
pub fn list_extensions() -> Vec<ExtensionInstallation> {
    let registry = PluginRegistry::discover();
    let mut extensions = registry
        .plugins()
        .iter()
        .cloned()
        .map(|manifest| extension_from_manifest(manifest, false))
        .collect::<Vec<_>>();
    extensions.extend(
        registry
            .package_plugins()
            .chain(registry.standalone_skills())
            .chain(registry.standalone_mcp())
            .cloned()
            .map(|manifest| extension_from_manifest(manifest, true)),
    );
    extensions
}

fn extension_from_manifest(manifest: PluginManifest, installed: bool) -> ExtensionInstallation {
    ExtensionInstallation {
        id: manifest.plugin_id.clone(),
        display_name: manifest.display_name.clone(),
        description: manifest.description.clone(),
        version: manifest.version.clone(),
        composition: manifest.composition(),
        skills: manifest.skills,
        mcp_servers: manifest.mcp_servers,
        installed,
    }
}

/// Reads all Markdown files found in every installed skill for a plugin.
pub fn read_plugin_skills(plugin_id: &PluginId) -> Result<PluginSkillsResponse> {
    let registry = PluginRegistry::discover();
    let manifest = registry.plugin(plugin_id)?.clone();
    let skills = manifest
        .skills
        .iter()
        .map(|skill_id| {
            let files = registry
                .read_skill_files(plugin_id, skill_id)?
                .into_iter()
                .filter(|document| document.path.as_str().to_ascii_lowercase().ends_with(".md"))
                .map(|document| PluginSkillFile {
                    path: document.path.as_str().to_string(),
                    content: document.content,
                })
                .collect();
            Ok(PluginSkillFiles {
                skill_id: skill_id.clone(),
                description: registry
                    .skill_description(plugin_id, skill_id)
                    .unwrap_or("Installed plugin skill instructions.")
                    .to_string(),
                files,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(PluginSkillsResponse {
        plugin_id: manifest.plugin_id,
        display_name: manifest.display_name,
        version: manifest.version,
        skills,
    })
}
