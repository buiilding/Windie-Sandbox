//! Plugin catalog, skill lookup, and MCP server activation.

use anyhow::Result;

use crate::error;
use crate::mcp::{McpRegistry, ProviderInstallState};
use crate::skills::{SkillId, SkillRegistry};
use crate::store::Store;
use crate::tool::{ToolProviderId, ToolSchemaName};

use super::manifest::{PluginId, PluginManifest, PluginVersion};

#[derive(Debug, Clone)]
/// Registry of plugins bundled and curated by Windie.
pub struct PluginRegistry {
    plugins: Vec<PluginManifest>,
    skills: SkillRegistry,
}

impl PluginRegistry {
    /// Returns the plugins and skills shipped by this Windie build.
    pub fn curated() -> Self {
        Self {
            plugins: vec![PluginManifest {
                plugin_id: PluginId::new("driver"),
                version: PluginVersion::new("0.1.0"),
                display_name: "Computer Driver".to_string(),
                description: "Use approved local computer-control tools through a repeatable driver workflow.".to_string(),
                skills: vec![SkillId::new("driver")],
                mcp_servers: vec![ToolProviderId::new("cua-driver")],
            }],
            skills: SkillRegistry::default(),
        }
    }

    /// Returns every curated plugin in deterministic catalog order.
    pub fn plugins(&self) -> &[PluginManifest] {
        &self.plugins
    }

    /// Finds one plugin by its exact stable identifier.
    pub fn plugin(&self, plugin_id: &PluginId) -> Result<&PluginManifest> {
        self.plugins
            .iter()
            .find(|plugin| plugin.plugin_id == *plugin_id)
            .ok_or_else(|| error::not_found(format!("plugin does not exist: {plugin_id}")))
    }

    /// Returns one plugin-owned skill's full instructions.
    pub fn read_skill(&self, plugin_id: &PluginId, skill_id: &SkillId) -> Result<String> {
        let plugin = self.plugin(plugin_id)?;
        if !plugin.skills.contains(skill_id) {
            return Err(error::not_found(format!(
                "skill is not part of plugin: {plugin_id}/{skill_id}"
            )));
        }
        self.skills.read(skill_id)
    }

    /// Builds compact runtime context without loading full skill instructions.
    pub fn catalog_prompt(&self, store: &Store, mcp: &McpRegistry) -> String {
        let mut lines = vec!["Available plugins:".to_string()];
        for plugin in &self.plugins {
            lines.push(String::new());
            lines.push(format!("{} ({}):", plugin.plugin_id, plugin.display_name));
            lines.push(format!("  Purpose: {}", plugin.description));
            lines.push("  Skills:".to_string());
            for skill_id in &plugin.skills {
                if let Some(skill) = self
                    .skills
                    .skills()
                    .iter()
                    .find(|skill| skill.skill_id == *skill_id)
                {
                    lines.push(format!(
                        "    - {}: {}",
                        skill.skill_id.as_str(),
                        skill.description
                    ));
                }
            }
            lines.push("  MCP servers:".to_string());
            for server_id in &plugin.mcp_servers {
                lines.push(format!("    - {server_id}"));
            }
            lines.push(format!("  Status: {}", self.status(store, mcp, plugin)));
        }
        lines.join("\n")
    }

    /// Activates every MCP server referenced by one plugin.
    pub fn attach_plugin(
        &self,
        store: &mut Store,
        conversation_id: &crate::conversation::ConversationId,
        plugin_id: &PluginId,
        mcp: &McpRegistry,
    ) -> Result<Vec<ToolSchemaName>> {
        let plugin = self.plugin(plugin_id)?;
        let mut names = Vec::new();
        for server_id in &plugin.mcp_servers {
            names.extend(mcp.attach_provider_tools(store, conversation_id, server_id)?);
        }
        Ok(names)
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
}

impl Default for PluginRegistry {
    fn default() -> Self {
        Self::curated()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::McpCommand;
    use crate::tool::{
        ProviderToolName, ToolAnnotations, ToolDefinition, ToolPermission, ToolProviderKind,
        ToolProviderRef, ToolSchemaName,
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
            skills: SkillRegistry {
                skills: vec![crate::skills::SkillManifest {
                    skill_id: SkillId::new("test-skill"),
                    display_name: "Test skill".to_string(),
                    description: "Test skill description".to_string(),
                    content: "test skill instructions".to_string(),
                }],
            },
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
        let registry = PluginRegistry::default();
        let plugin = registry.plugin(&PluginId::new("driver")).unwrap();

        assert_eq!(plugin.mcp_servers[0].as_str(), "cua-driver");
        assert_eq!(plugin.skills[0].as_str(), "driver");
        assert!(
            registry
                .read_skill(&PluginId::new("driver"), &SkillId::new("driver"))
                .unwrap()
                .contains("Windie computer driver")
        );
    }

    #[test]
    fn plugin_catalog_is_compact_and_reports_mcp_setup() {
        let store = Store::open_memory().unwrap();
        let mcp = McpRegistry::new();
        let prompt = PluginRegistry::default().catalog_prompt(&store, &mcp);

        assert!(prompt.contains("Available plugins:"));
        assert!(prompt.contains("driver"));
        assert!(prompt.contains("setup required"));
        assert!(!prompt.contains("Windie computer driver\n\nUse the computer"));
    }

    #[test]
    fn attach_plugin_reuses_persisted_mcp_catalog() {
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
            .attach_plugin(
                &mut store,
                &conversation_id,
                &PluginId::new("test-plugin"),
                &mcp,
            )
            .unwrap();

        assert_eq!(attached, vec![ToolSchemaName::new("test_provider__read")]);
        assert_eq!(store.load_tool_schemas(&conversation_id).unwrap().len(), 1);
    }
}
