//! Plugin catalog, skill lookup, and MCP server activation.

use anyhow::Result;
use std::path::Path;

use crate::error;
use crate::local;
use crate::mcp::{McpRegistry, ProviderInstallState};
use crate::skills::{SkillDocument, SkillId, SkillPath, SkillRegistry};
use crate::store::Store;
use crate::tool::ToolSchemaName;

use super::curated;
use super::manifest::{ExtensionComposition, ExtensionTarget, PluginId, PluginManifest};
use super::package::PluginPackage;

#[derive(Debug, Clone)]
struct PackageEntry {
    package: PluginPackage,
    manifest: PluginManifest,
}

#[derive(Debug, Clone)]
/// Registry of curated definitions and installed file-based plugins.
pub struct PluginRegistry {
    plugins: Vec<PluginManifest>,
    skills: SkillRegistry,
    packages: Vec<PackageEntry>,
    package_errors: Vec<String>,
}

impl PluginRegistry {
    /// Returns the curated plugin definitions before package discovery.
    pub fn curated() -> Self {
        Self {
            plugins: vec![curated::cua_driver()],
            skills: SkillRegistry::default(),
            packages: Vec::new(),
            package_errors: Vec::new(),
        }
    }

    /// Discovers installed file-based plugins and merges them with the
    /// trusted code-owned catalog.
    pub fn discover() -> Self {
        let mut registry = Self::curated();
        let Ok(root) = local::windie_home_dir().map(|path| path.join("plugins")) else {
            return registry;
        };
        registry.load_packages_from(root);
        registry
    }

    /// Loads package directories from a specific root using the same package
    /// validation path as installed plugins.
    pub fn load_packages_from(&mut self, root: impl AsRef<Path>) {
        let root = root.as_ref();
        let Ok(entries) = std::fs::read_dir(root) else {
            return;
        };
        let mut package_roots = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            if path.join(".codex-plugin/plugin.json").is_file() {
                package_roots.push(path);
                continue;
            }
            let Ok(versions) = std::fs::read_dir(&path) else {
                continue;
            };
            for version in versions.flatten() {
                let version_path = version.path();
                if version_path.join(".codex-plugin/plugin.json").is_file() {
                    package_roots.push(version_path);
                }
            }
        }
        package_roots.sort();

        for package_root in package_roots {
            match PluginPackage::load(&package_root) {
                Ok(package) => {
                    let manifest = package.plugin_manifest();
                    if self
                        .packages
                        .iter()
                        .any(|entry| entry.manifest.plugin_id == manifest.plugin_id)
                    {
                        self.package_errors.push(format!(
                            "ignored package with duplicate plugin id: {}",
                            manifest.plugin_id
                        ));
                        continue;
                    }
                    // A materialized curated plugin is the installed
                    // realization of its code-owned recipe. Once present, it
                    // replaces the recipe in the runtime catalog so its
                    // file-backed skills and manifests are authoritative.
                    self.plugins
                        .retain(|plugin| plugin.plugin_id != manifest.plugin_id);
                    self.packages.push(PackageEntry { package, manifest });
                }
                Err(error) => self.package_errors.push(error.to_string()),
            }
        }
    }

    /// Returns every curated plugin in deterministic catalog order.
    pub fn plugins(&self) -> &[PluginManifest] {
        &self.plugins
    }

    /// Returns valid package plugins discovered from the local package store.
    pub fn package_plugins(&self) -> impl Iterator<Item = &PluginManifest> {
        self.packages
            .iter()
            .filter(|entry| entry.manifest.composition() == ExtensionComposition::Plugin)
            .map(|entry| &entry.manifest)
    }

    /// Returns installed packages containing skills but no MCP server.
    pub fn standalone_skills(&self) -> impl Iterator<Item = &PluginManifest> {
        self.packages
            .iter()
            .filter(|entry| entry.manifest.composition() == ExtensionComposition::Skill)
            .map(|entry| &entry.manifest)
    }

    /// Returns installed packages containing MCP servers but no skills.
    pub fn standalone_mcp(&self) -> impl Iterator<Item = &PluginManifest> {
        self.packages
            .iter()
            .filter(|entry| entry.manifest.composition() == ExtensionComposition::Mcp)
            .map(|entry| &entry.manifest)
    }

    /// Finds one plugin by its exact stable identifier.
    pub fn plugin(&self, plugin_id: &PluginId) -> Result<&PluginManifest> {
        self.plugins
            .iter()
            .find(|plugin| plugin.plugin_id == *plugin_id)
            .or_else(|| {
                self.packages
                    .iter()
                    .find(|entry| entry.manifest.plugin_id == *plugin_id)
                    .map(|entry| &entry.manifest)
            })
            .ok_or_else(|| error::not_found(format!("plugin does not exist: {plugin_id}")))
    }

    /// Finds the plugin that owns one of its declared MCP servers.
    ///
    /// The MCP server remains the transport-level implementation, but the
    /// owning plugin is the extension-level identity shown to users. Keeping
    /// this lookup here prevents API clients from having to infer plugin
    /// ownership from provider IDs.
    pub fn plugin_for_mcp_server(
        &self,
        provider_id: &crate::tool::ToolProviderId,
    ) -> Option<&PluginManifest> {
        self.plugins
            .iter()
            .find(|plugin| {
                plugin
                    .mcp_servers
                    .iter()
                    .any(|server| server == provider_id)
            })
            .or_else(|| {
                self.packages.iter().find_map(|entry| {
                    entry
                        .manifest
                        .mcp_servers
                        .iter()
                        .any(|server| server == provider_id)
                        .then_some(&entry.manifest)
                })
            })
    }

    /// Returns one plugin-owned skill's full instructions.
    pub fn read_skill(
        &self,
        plugin_id: &PluginId,
        skill_id: &SkillId,
        path: Option<&SkillPath>,
    ) -> Result<SkillDocument> {
        let plugin = self.plugin(plugin_id)?;
        if !plugin.skills.contains(skill_id) {
            return Err(error::not_found(format!(
                "skill is not part of plugin: {plugin_id}/{skill_id}"
            )));
        }
        if let Some(package) = self
            .packages
            .iter()
            .find(|entry| entry.manifest.plugin_id == *plugin_id)
        {
            return package.package.read_skill(skill_id, path);
        }
        self.skills.read(skill_id, path)
    }

    /// Reads every file in one plugin-owned skill in deterministic order.
    ///
    /// This is used by inspection surfaces that need to render the complete
    /// installed bundle. Model-facing reads remain bounded through
    /// [`Self::read_skill`], which loads only the requested file.
    pub fn read_skill_files(
        &self,
        plugin_id: &PluginId,
        skill_id: &SkillId,
    ) -> Result<Vec<SkillDocument>> {
        let plugin = self.plugin(plugin_id)?;
        if !plugin.skills.contains(skill_id) {
            return Err(error::not_found(format!(
                "skill is not part of plugin: {plugin_id}/{skill_id}"
            )));
        }
        if let Some(package) = self
            .packages
            .iter()
            .find(|entry| entry.manifest.plugin_id == *plugin_id)
        {
            return package.package.read_skill_files(skill_id);
        }

        let manifest = self
            .skills
            .skills()
            .find(|skill| skill.skill_id == *skill_id)
            .ok_or_else(|| {
                error::not_found(format!("skill does not exist: {plugin_id}/{skill_id}"))
            })?;
        manifest
            .files
            .iter()
            .map(|path| self.skills.read(skill_id, Some(path)))
            .collect()
    }

    /// Returns the catalog description for one plugin-owned skill.
    pub fn skill_description(&self, plugin_id: &PluginId, skill_id: &SkillId) -> Option<&str> {
        self.packages
            .iter()
            .find(|entry| entry.manifest.plugin_id == *plugin_id)
            .and_then(|entry| entry.package.skill_description(skill_id))
            .or_else(|| {
                self.skills
                    .skills()
                    .find(|skill| skill.skill_id == *skill_id)
                    .map(|skill| skill.description.as_str())
            })
    }

    /// Builds compact runtime context without loading full skill instructions.
    pub fn catalog_prompt(&self, store: &Store, mcp: &McpRegistry) -> String {
        let mut lines = vec!["Available plugins:".to_string()];
        for plugin in &self.plugins {
            lines.push(String::new());
            lines.push(format!("{} ({}):", plugin.plugin_id, plugin.display_name));
            lines.push(format!("  Purpose: {}", plugin.description));
            lines.push("  Skills:".to_string());
            let mut described_skill = false;
            for skill_id in &plugin.skills {
                if let Some(skill) = self
                    .skills
                    .skills()
                    .find(|skill| skill.skill_id == *skill_id)
                {
                    described_skill = true;
                    lines.push(format!(
                        "    - {}: {}",
                        skill.skill_id.as_str(),
                        skill.description
                    ));
                }
            }
            if !described_skill {
                for skill_id in &plugin.skills {
                    lines.push(format!(
                        "    - {}: install this plugin to load its upstream skill",
                        skill_id
                    ));
                }
            }
            lines.push("  MCP servers:".to_string());
            for server_id in &plugin.mcp_servers {
                lines.push(format!("    - {server_id}"));
            }
            lines.push(format!("  Status: {}", self.status(store, mcp, plugin)));
        }
        for entry in self
            .packages
            .iter()
            .filter(|entry| entry.manifest.composition() == ExtensionComposition::Plugin)
        {
            let plugin = &entry.manifest;
            lines.push(String::new());
            lines.push(format!("{} ({}):", plugin.plugin_id, plugin.display_name));
            lines.push(format!("  Purpose: {}", plugin.description));
            if let Some(author) = entry.package.author() {
                lines.push(format!("  Owner: {author}"));
            }
            lines.push("  Skills:".to_string());
            for skill_id in &plugin.skills {
                let description = entry
                    .package
                    .skill_description(skill_id)
                    .unwrap_or("Package-provided instructions.");
                lines.push(format!("    - {}: {}", skill_id.as_str(), description));
            }
            lines.push("  MCP servers:".to_string());
            for server_id in &plugin.mcp_servers {
                lines.push(format!("    - {server_id}"));
            }
            lines.push(format!(
                "  Status: {}",
                self.package_status(store, mcp, entry)
            ));
        }
        let standalone_skills = self
            .packages
            .iter()
            .filter(|entry| entry.manifest.composition() == ExtensionComposition::Skill)
            .collect::<Vec<_>>();
        if !standalone_skills.is_empty() {
            lines.push(String::new());
            lines.push("Available standalone skills:".to_string());
            for entry in standalone_skills {
                lines.push(format!(
                    "  {} ({}):",
                    entry.manifest.plugin_id, entry.manifest.display_name
                ));
                lines.push(format!("    Purpose: {}", entry.manifest.description));
                for skill_id in &entry.manifest.skills {
                    let description = entry
                        .package
                        .skill_description(skill_id)
                        .unwrap_or("Package-provided instructions.");
                    lines.push(format!("    - {}: {}", skill_id, description));
                }
                lines.push("    Status: installed; read_skill available on demand".to_string());
            }
        }
        let standalone_mcp = self
            .packages
            .iter()
            .filter(|entry| entry.manifest.composition() == ExtensionComposition::Mcp)
            .collect::<Vec<_>>();
        if !standalone_mcp.is_empty() {
            lines.push(String::new());
            lines.push("Available standalone MCP servers:".to_string());
            for entry in &standalone_mcp {
                lines.push(format!(
                    "  {} ({}):",
                    entry.manifest.plugin_id, entry.manifest.display_name
                ));
                lines.push(format!("    Purpose: {}", entry.manifest.description));
                for server_id in &entry.manifest.mcp_servers {
                    lines.push(format!("    - {}", server_id));
                }
                lines.push(format!(
                    "    Status: {}",
                    self.package_status(store, mcp, entry)
                ));
            }
        }
        let declared_mcp_ids = self
            .plugins
            .iter()
            .chain(self.packages.iter().map(|entry| &entry.manifest))
            .flat_map(|plugin| plugin.mcp_servers.iter())
            .collect::<std::collections::HashSet<_>>();
        let standalone_approved_mcp = mcp
            .provider_manifests()
            .into_iter()
            .filter(|manifest| !declared_mcp_ids.contains(&manifest.provider_id))
            .collect::<Vec<_>>();
        if !standalone_approved_mcp.is_empty() {
            lines.push(String::new());
            if standalone_mcp.is_empty() {
                lines.push("Available standalone MCP servers:".to_string());
            }
            for manifest in standalone_approved_mcp {
                lines.push(format!("  mcp:{}:", manifest.provider_id));
                lines.push(format!("    Purpose: {}", manifest.description));
                lines.push(format!(
                    "    Status: {}",
                    self.standalone_mcp_status(store, mcp, &manifest.provider_id)
                ));
            }
        }
        for package_error in &self.package_errors {
            lines.push(format!("  Package unavailable: {package_error}"));
        }
        lines.join("\n")
    }

    /// Activates every MCP server referenced by one plugin.
    pub fn attach_extension(
        &self,
        store: &mut Store,
        conversation_id: &crate::conversation::ConversationId,
        target: &ExtensionTarget,
        mcp: &McpRegistry,
    ) -> Result<Vec<ToolSchemaName>> {
        let server_ids = match target {
            ExtensionTarget::Plugin(plugin_id) => self.plugin(plugin_id)?.mcp_servers.clone(),
            ExtensionTarget::Mcp(provider_id) => vec![provider_id.clone()],
        };
        if server_ids.is_empty() {
            return Err(error::invalid_request(
                "extension has no MCP servers; use read_skill for its instructions",
            ));
        }

        for provider_id in &server_ids {
            if let Some(entry) = self.packages.iter().find(|entry| {
                entry
                    .manifest
                    .mcp_servers
                    .iter()
                    .any(|server_id| server_id == provider_id)
            }) {
                let server = entry.package.mcp_server(provider_id).ok_or_else(|| {
                    error::not_found(format!("MCP server does not exist: {provider_id}"))
                })?;
                mcp.register_package_provider(&entry.package, server)?;
                if store.load_installed_provider(provider_id)?.is_none() {
                    store.install_provider(provider_id)?;
                    match mcp.discover_provider_tools(provider_id) {
                        Ok(tools) => {
                            store.save_provider_tool_catalog(provider_id, &tools)?;
                            store.set_provider_state(
                                provider_id,
                                ProviderInstallState::Enabled,
                                None,
                            )?;
                        }
                        Err(error) => {
                            store.set_provider_state(
                                provider_id,
                                ProviderInstallState::Broken,
                                Some(&error.to_string()),
                            )?;
                            return Err(error);
                        }
                    }
                }
            }
        }

        server_ids
            .into_iter()
            .try_fold(Vec::new(), |mut names, provider_id| {
                names.extend(mcp.attach_provider_tools(store, conversation_id, &provider_id)?);
                Ok(names)
            })
    }

    fn status(&self, store: &Store, mcp: &McpRegistry, plugin: &PluginManifest) -> &'static str {
        for server_id in &plugin.mcp_servers {
            if mcp.provider_manifest(server_id).is_none() {
                return "unavailable: unknown MCP server";
            }
            let Ok(Some(installation)) = store.load_installed_provider(server_id) else {
                return "setup required";
            };
            if installation.state != ProviderInstallState::Enabled || installation.error.is_some() {
                return "setup required";
            }
            let Ok(Some(catalog)) = store.load_provider_tool_catalog(server_id) else {
                return "setup required";
            };
            if catalog.status == crate::store::ProviderCatalogStatus::Unavailable {
                return "unavailable";
            }
        }
        "enabled; tools available on demand"
    }

    fn package_status(
        &self,
        store: &Store,
        mcp: &McpRegistry,
        entry: &PackageEntry,
    ) -> &'static str {
        if entry.package.mcp_servers().next().is_none() {
            return "enabled; skills available on demand";
        }
        for server in entry.package.mcp_servers() {
            if mcp.provider_manifest(&server.provider_id).is_none() {
                return "available; attach to activate";
            }
            let Ok(Some(installation)) = store.load_installed_provider(&server.provider_id) else {
                return "available; attach to activate";
            };
            if installation.state != ProviderInstallState::Enabled || installation.error.is_some() {
                return "setup required";
            }
        }
        "enabled; tools available on demand"
    }

    fn standalone_mcp_status(
        &self,
        store: &Store,
        mcp: &McpRegistry,
        provider_id: &crate::tool::ToolProviderId,
    ) -> &'static str {
        if mcp.provider_manifest(provider_id).is_none() {
            return "unavailable";
        }
        let Ok(Some(installation)) = store.load_installed_provider(provider_id) else {
            return "setup required";
        };
        if installation.state != ProviderInstallState::Enabled || installation.error.is_some() {
            return "setup required";
        }
        let Ok(Some(catalog)) = store.load_provider_tool_catalog(provider_id) else {
            return "setup required";
        };
        if catalog.status == crate::store::ProviderCatalogStatus::Unavailable {
            return "unavailable";
        }
        "enabled; tools available on demand"
    }
}

impl Default for PluginRegistry {
    fn default() -> Self {
        Self::discover()
    }
}

#[cfg(test)]
mod tests {
    use super::super::manifest::PluginVersion;
    use super::*;
    use crate::mcp::McpCommand;
    use crate::tool::{
        ProviderToolName, ToolAnnotations, ToolDefinition, ToolPermission, ToolProviderId,
        ToolProviderKind, ToolProviderRef, ToolSchemaName,
    };

    fn test_plugin(provider_id: &str) -> PluginRegistry {
        PluginRegistry {
            plugins: vec![PluginManifest {
                plugin_id: PluginId::new("test-plugin"),
                version: PluginVersion::new("1.0.0"),
                display_name: "Test plugin".to_string(),
                description: "Test plugin description".to_string(),
                skills: vec![SkillId::new("test-skill")],
                mcp_servers: vec![ToolProviderId::new(provider_id)],
            }],
            skills: SkillRegistry::from_bundles(vec![crate::skills::SkillBundle {
                manifest: crate::skills::SkillManifest {
                    skill_id: SkillId::new("test-skill"),
                    display_name: "Test skill".to_string(),
                    description: "Test skill description".to_string(),
                    entrypoint: crate::skills::SkillPath::entrypoint(),
                    files: vec![crate::skills::SkillPath::entrypoint()],
                },
                files: vec![crate::skills::SkillFile {
                    path: crate::skills::SkillPath::entrypoint(),
                    content: "test skill instructions".to_string(),
                }],
            }]),
            packages: Vec::new(),
            package_errors: Vec::new(),
        }
    }

    fn test_tool(provider_id: &str) -> ToolDefinition {
        ToolDefinition {
            schema_name: ToolSchemaName::new("test_provider__read"),
            display_name: "Test read".to_string(),
            description: "Read test data".to_string(),
            parameters: serde_json::json!({"type": "object"}),
            provider: ToolProviderRef::new(
                ToolProviderId::new(provider_id),
                ProviderToolName::new("read"),
                ToolProviderKind::Mcp,
            ),
            permissions: vec![ToolPermission::ExternalProcess],
            annotations: ToolAnnotations::default(),
        }
    }

    #[test]
    fn curated_driver_plugin_references_separate_skill_and_mcp_server() {
        let registry = PluginRegistry::curated();
        let plugin = registry.plugin(&PluginId::new("cua-driver")).unwrap();

        assert_eq!(plugin.mcp_servers[0].as_str(), "cua-driver");
        assert_eq!(plugin.skills[0].as_str(), "cua-driver");
        assert!(
            registry
                .read_skill(
                    &PluginId::new("cua-driver"),
                    &SkillId::new("cua-driver"),
                    None,
                )
                .is_err()
        );
    }

    #[test]
    fn plugin_catalog_is_compact_and_reports_mcp_setup() {
        let store = Store::open_memory().unwrap();
        let mcp = McpRegistry::new();
        let prompt = PluginRegistry::curated().catalog_prompt(&store, &mcp);

        assert!(prompt.contains("Available plugins:"));
        assert!(prompt.contains("cua-driver"));
        assert!(prompt.contains("setup required"));
        assert!(!prompt.contains("Windie CUA Driver\n\nUse the computer"));
    }

    #[test]
    fn uninstalled_curated_plugin_does_not_read_upstream_skill_files() {
        let registry = PluginRegistry::curated();
        let path = SkillPath::new("MACOS.md").unwrap();
        assert!(
            registry
                .read_skill(
                    &PluginId::new("cua-driver"),
                    &SkillId::new("cua-driver"),
                    Some(&path),
                )
                .is_err()
        );
    }

    #[test]
    fn attach_extension_reuses_persisted_mcp_catalog() {
        let mut store = Store::open_memory().unwrap();
        let conversation_id = store.create_conversation("openai/test").unwrap();
        let provider_id = ToolProviderId::new("desktop-commander");
        store.install_provider(&provider_id).unwrap();
        store
            .save_provider_tool_catalog(&provider_id, &[test_tool("desktop-commander")])
            .unwrap();
        store
            .set_provider_state(&provider_id, ProviderInstallState::Enabled, None)
            .unwrap();

        let mcp = McpRegistry::with_test_mcp_provider(
            "desktop-commander",
            "test_provider",
            "Test provider",
            McpCommand {
                program: "true",
                args: &[],
                env: &[],
            },
        );
        let registry = test_plugin("desktop-commander");
        let attached = registry
            .attach_extension(
                &mut store,
                &conversation_id,
                &ExtensionTarget::Plugin(PluginId::new("test-plugin")),
                &mcp,
            )
            .unwrap();

        assert_eq!(attached, vec![ToolSchemaName::new("test_provider__read")]);
        assert_eq!(store.load_tool_schemas(&conversation_id).unwrap().len(), 1);
    }
}
