//! Installable Windie plugin packages.
//!
//! A plugin is Windie's marketplace and installation unit. Plugins contain
//! typed components such as MCP servers, skills, and app connectors. This
//! module owns package metadata, component manifest validation, the local
//! plugin store, and the marketplace index; component runtimes remain owned
//! by their respective runtime modules.

mod catalog;
mod installer;
pub(crate) mod manifest;
mod store;

pub use catalog::{
    AppSummary, MarketplaceIndex, MarketplacePlugin, MarketplacePresentation, MarketplaceVersion,
    McpSummary, PluginCatalog, PluginIndex, PluginState, PluginSummary, SkillSummary,
    bundled as bundled_index,
};
pub use installer::MarketplaceInstaller;
pub use manifest::{
    AppManifest, McpAuthentication, McpDelivery, McpPackage, McpPackageTransport, McpRemote,
    McpRemoteHeader, McpRemoteTransport, McpServerManifest, PluginComponent, PluginComponentKind,
    PluginManifest, SkillManifest, WindieMcpMetadata, WindieMcpSetup, WindieSetupEnvironment,
    WindieSetupEnvironmentValue, WindieSetupFile,
};
pub use store::{InstalledPlugin, InstalledSkill, PluginStore};

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::Arc;
    use std::time::Duration;

    use super::*;

    #[test]
    fn plugin_index_moves_installed_plugins_out_of_available_plugins() {
        let root = std::env::temp_dir().join(format!(
            "windie-plugin-index-package-test-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let plugin_store = Arc::new(PluginStore::new(&root));
        plugin_store.install_bundled("parallel-search").unwrap();
        let catalog = PluginCatalog::new(plugin_store.clone(), bundled_index().unwrap());
        let store = crate::store::Store::open_memory().unwrap();
        let index = catalog
            .build_index(&store, &crate::tool::ToolProviderRegistry::new())
            .unwrap();

        assert_eq!(index.installed.len(), 1);
        assert_eq!(index.installed[0].id, "parallel-search");
        assert_eq!(index.installed[0].mcps[0].state, PluginState::Installed);
        assert!(
            index
                .available
                .iter()
                .all(|plugin| plugin.id != "parallel-search")
        );
        assert!(
            index
                .available
                .iter()
                .any(|plugin| plugin.id == "basic-memory")
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn installed_skill_is_parsed_and_read_from_package_content() {
        let source = std::env::temp_dir().join(format!(
            "windie-skill-source-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(source.join("skills/driver")).unwrap();
        fs::write(source.join("README.md"), "# Driver plugin").unwrap();
        fs::write(source.join("assets.svg"), "<svg />").unwrap();
        fs::write(
            source.join("skills/driver/SKILL.md"),
            "---\nname: Driver instructions\ndescription: Control the local computer safely.\n---\n# Driver instructions\n\nUse approved computer-control actions.",
        )
        .unwrap();
        fs::write(
            source.join("plugin.json"),
            r#"{
              "manifest_version": 1,
              "plugin": {"id": "driver", "version": "1.0.0", "publisher": "windie"},
              "presentation": {"name": "Driver", "description": "Control the local computer.", "readme": "README.md", "icon": "assets.svg"},
              "components": [{"type": "skill", "id": "driver", "manifest": "skills/driver/SKILL.md"}]
            }"#,
        )
        .unwrap();

        let package_root = std::env::temp_dir().join(format!(
            "windie-skill-package-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let store = PluginStore::new(&package_root);
        let installed = store.install_from_directory(&source).unwrap();
        let skills = installed.skills().unwrap();
        assert_eq!(skills[0].id, "driver");
        assert_eq!(skills[0].description, "Control the local computer safely.");
        assert!(
            installed
                .read_skill("driver")
                .unwrap()
                .contains("approved computer-control actions")
        );

        fs::remove_dir_all(source).unwrap();
        fs::remove_dir_all(package_root).unwrap();
    }

    #[test]
    fn bundled_index_and_parallel_plugin_validate() {
        let index = bundled_index().unwrap();
        assert_eq!(index.plugins.len(), 6);
        assert_eq!(index.plugins[0].id, "parallel-search");

        let root = std::env::temp_dir().join(format!(
            "windie-plugin-test-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let store = PluginStore::new(&root);
        let plugin = store.install_bundled("parallel-search").unwrap();
        let components = crate::mcp::load_components(&plugin).unwrap();

        assert_eq!(plugin.manifest.plugin.id, "parallel-search");
        assert_eq!(components.len(), 1);
        assert_eq!(components[0].component_id, "parallel-search");
        assert_eq!(components[0].manifest.name, "ai.parallel/parallel-search");
        assert_eq!(components[0].manifest.version, "1.0.0");
        assert_eq!(components[0].manifest.remotes.len(), 1);
        assert_eq!(
            components[0].windie.capabilities,
            vec!["web_search", "url_extraction"]
        );
        assert!(components[0].readme.contains("Parallel Search"));
        let crate::mcp::McpTransport::StreamableHttp { ref endpoint } = components[0].transport
        else {
            panic!("Parallel Search should use Streamable HTTP");
        };
        assert_eq!(endpoint.startup_timeout, Duration::from_secs(120));
        assert_eq!(endpoint.call_timeout, Duration::from_secs(60));

        let registry = crate::tool::ToolProviderRegistry::new();
        registry.register_plugin(&plugin).unwrap();
        let provider = registry
            .provider_manifests()
            .into_iter()
            .find(|manifest| manifest.provider_id.as_str() == "parallel-search")
            .unwrap();
        assert_eq!(provider.display_name, "Parallel Search");
        assert_eq!(provider.secrets.len(), 1);
        assert!(!provider.secrets[0].required);

        registry.unregister_plugin(&plugin).unwrap();
        assert!(
            registry
                .provider_manifest(&crate::tool::ToolProviderId::new("parallel-search"))
                .is_none()
        );

        let removed = store.remove_plugin("parallel-search").unwrap();
        assert_eq!(removed.len(), 1);
        assert!(store.installed_plugins().unwrap().is_empty());

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn basic_memory_package_declares_uv_mcpb_and_persistent_setup() {
        let root = std::env::temp_dir().join(format!(
            "windie-basic-memory-package-test-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let store = PluginStore::new(&root);
        let plugin = store.install_bundled("basic-memory").unwrap();
        let component = &plugin.manifest.components[0];

        assert_eq!(component.id, "basic-memory");
        assert_eq!(
            component.windie.local_artifact.as_deref(),
            Some("mcp/basic-memory.mcpb")
        );
        assert!(component.windie.setup.isolated_home);
        assert!(
            component
                .windie
                .setup
                .environment
                .iter()
                .any(|environment| environment.name == "BASIC_MEMORY_HOME")
        );

        let server_path = plugin.root.join(&component.manifest);
        let server = McpServerManifest::parse(&fs::read_to_string(server_path).unwrap()).unwrap();
        assert_eq!(server.packages[0].runtime_hint.as_deref(), Some("uv"));
        assert_eq!(server.packages[0].transport.kind, "stdio");

        store.remove_plugin("basic-memory").unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn blender_package_declares_pinned_uv_mcpb_and_external_bridge_setup() {
        let root = std::env::temp_dir().join(format!(
            "windie-blender-mcp-package-test-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let store = PluginStore::new(&root);
        let plugin = store.install_bundled("blender-mcp").unwrap();
        let components = crate::mcp::load_components(&plugin).unwrap();
        assert_eq!(components.len(), 1);
        assert_eq!(
            components[0].manifest.name,
            "io.github.ahujasid/blender-mcp"
        );
        assert_eq!(components[0].manifest.version, "1.8.3");
        assert!(components[0].windie.setup.isolated_home);
        assert!(
            components[0]
                .windie
                .setup
                .environment
                .iter()
                .any(|environment| environment.name == "BLENDER_PORT")
        );

        let crate::mcp::McpTransport::PackagedStdio { ref command, .. } = components[0].transport
        else {
            panic!("Blender MCP should use a packaged stdio transport");
        };
        assert_eq!(command.program, "uv");
        assert!(
            command
                .args
                .iter()
                .any(|argument| argument == "blender-mcp")
        );
        assert!(
            command
                .env
                .iter()
                .any(|(name, value)| name == "DISABLE_TELEMETRY" && value == "true")
        );

        store.remove_plugin("blender-mcp").unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn chrome_devtools_package_declares_node_mcpb_and_connection_setup() {
        let root = std::env::temp_dir().join(format!(
            "windie-chrome-devtools-package-test-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let store = PluginStore::new(&root);
        let plugin = store.install_bundled("chrome-devtools").unwrap();
        let components = crate::mcp::load_components(&plugin).unwrap();
        assert_eq!(components.len(), 1);
        assert_eq!(
            components[0].manifest.name,
            "io.github.ChromeDevTools/chrome-devtools-mcp"
        );
        assert_eq!(components[0].manifest.version, "1.6.0");
        assert!(components[0].windie.setup.isolated_home);

        let crate::mcp::McpTransport::PackagedStdio { ref command, .. } = components[0].transport
        else {
            panic!("Chrome DevTools should use a packaged stdio transport");
        };
        assert_eq!(command.program, "node");
        assert!(
            command
                .args
                .iter()
                .any(|argument| argument.ends_with("server.js"))
        );
        assert!(command.env.iter().any(|(name, value)| {
            name == "WINDIE_CHROME_CONNECTION_MODE" && value == "managed"
        }));

        store.remove_plugin("chrome-devtools").unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn brightdata_package_declares_node_mcpb_and_deferred_api_key_delivery() {
        let root = std::env::temp_dir().join(format!(
            "windie-brightdata-package-test-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let store = PluginStore::new(&root);
        let plugin = store.install_bundled("brightdata").unwrap();
        let components = crate::mcp::load_components(&plugin).unwrap();
        assert_eq!(components.len(), 1);
        assert_eq!(
            components[0].manifest.name,
            "io.github.brightdata/brightdata-mcp"
        );
        assert_eq!(components[0].manifest.version, "2.11.1");
        assert_eq!(
            components[0].windie.authentication,
            McpAuthentication::ApiKey {
                required: true,
                secret_id: "BRIGHTDATA_API_TOKEN".to_string(),
                setup_url: Some("https://brightdata.com/".to_string()),
                delivery: McpDelivery::Environment {
                    name: "API_TOKEN".to_string(),
                },
            }
        );

        let crate::mcp::McpTransport::PackagedStdio { ref command, .. } = components[0].transport
        else {
            panic!("Bright Data should use a packaged stdio transport");
        };
        assert_eq!(command.program, "node");
        assert!(
            command
                .args
                .iter()
                .any(|argument| argument.ends_with("server.js"))
        );
        assert_eq!(
            command.secret_env,
            vec![(
                "API_TOKEN".to_string(),
                "BRIGHTDATA_API_TOKEN".to_string(),
                true,
            )]
        );

        store.remove_plugin("brightdata").unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn cua_driver_package_declares_native_mcpb_and_isolated_runtime() {
        let root = std::env::temp_dir().join(format!(
            "windie-cua-driver-package-test-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let store = PluginStore::new(&root);
        let plugin = store.install_bundled("cua-driver").unwrap();
        let components = crate::mcp::load_components(&plugin).unwrap();
        assert_eq!(components.len(), 1);
        assert_eq!(components[0].manifest.name, "io.github.trycua/cua-driver");
        assert_eq!(components[0].manifest.version, "0.12.6");
        assert!(components[0].windie.setup.isolated_home);

        let crate::mcp::McpTransport::PackagedStdio { ref command, .. } = components[0].transport
        else {
            panic!("CUA Driver should use a packaged stdio transport");
        };
        assert!(
            command
                .program
                .ends_with("CuaDriver.app/Contents/MacOS/cua-driver")
        );
        assert_eq!(command.args, vec!["mcp"]);

        store.remove_plugin("cua-driver").unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn cua_driver_package_is_rejected_outside_macos() {
        let root = std::env::temp_dir().join(format!(
            "windie-cua-driver-platform-test-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let store = PluginStore::new(&root);
        let plugin = store.install_bundled("cua-driver").unwrap();

        let error = crate::mcp::load_components(&plugin).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("does not support the current platform")
        );

        store.remove_plugin("cua-driver").unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn desktop_commander_plugin_loads_pinned_mcpb_and_isolated_setup() {
        let root = std::env::temp_dir().join(format!(
            "windie-desktop-commander-plugin-test-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let store = PluginStore::new(&root);
        let plugin = store.install_bundled("desktop-commander").unwrap();
        let components = crate::mcp::load_components(&plugin).unwrap();
        assert_eq!(components.len(), 1);
        assert_eq!(
            components[0].manifest.name,
            "io.github.wonderwhy-er/desktop-commander"
        );
        assert_eq!(components[0].manifest.version, "0.2.47");
        assert!(components[0].windie.setup.isolated_home);

        let crate::mcp::McpTransport::PackagedStdio { ref command, .. } = components[0].transport
        else {
            panic!("Desktop Commander should use a packaged stdio transport");
        };
        assert_eq!(command.program, "node");
        assert!(
            command
                .args
                .iter()
                .any(|argument| argument.ends_with("dist/index.js"))
        );
        let home = command
            .env
            .iter()
            .find(|(key, _)| key == "HOME")
            .map(|(_, value)| value)
            .expect("Desktop Commander should receive an isolated HOME");
        let isolated_home_suffix = std::path::Path::new("runtime")
            .join("desktop-commander")
            .join("home");
        assert!(std::path::Path::new(home).ends_with(isolated_home_suffix));
        let config = std::fs::read_to_string(
            std::path::Path::new(home).join(".claude-server-commander/config.json"),
        )
        .unwrap();
        let config: serde_json::Value = serde_json::from_str(&config).unwrap();
        assert_eq!(config["telemetryEnabled"], false);
        assert!(config["blockedCommands"].as_array().unwrap().len() > 10);

        let provider = components[0]
            .manifest
            .provider_manifest(
                &components[0].plugin,
                &components[0].component_id,
                &components[0].windie,
            )
            .unwrap();
        assert_eq!(provider.runtime, crate::tool::ProviderRuntime::Node);
        assert!(
            provider
                .dependencies
                .iter()
                .any(|dependency| dependency.executable == "node")
        );
        assert!(
            provider
                .permissions
                .contains(&crate::tool::ProviderPermission::Filesystem)
        );

        let registry = crate::tool::ToolProviderRegistry::new();
        registry.register_plugin(&plugin).unwrap();
        assert!(
            registry
                .provider_manifest(&crate::tool::ToolProviderId::new("desktop-commander"))
                .is_some()
        );
        registry.unregister_plugin(&plugin).unwrap();
        store.remove_plugin("desktop-commander").unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[ignore = "requires a Windie-managed Node runtime; run explicitly during local MCP migration"]
    fn desktop_commander_mcpb_discovers_real_tools() {
        let root = std::env::temp_dir().join(format!(
            "windie-desktop-commander-live-test-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let store = PluginStore::new(&root);
        let plugin = store.install_bundled("desktop-commander").unwrap();
        let component = crate::mcp::load_components(&plugin).unwrap().remove(0);
        let transport = component.transport;
        let tools = crate::mcp::list_tools_with_transport(transport.clone()).unwrap();
        assert!(tools.iter().any(|tool| tool.name == "read_file"));
        assert!(tools.iter().any(|tool| tool.name == "start_process"));
        let home = match &transport {
            crate::mcp::McpTransport::PackagedStdio { command, .. } => command
                .env
                .iter()
                .find(|(key, _)| key == "HOME")
                .map(|(_, value)| value.clone())
                .unwrap(),
            _ => panic!("Desktop Commander should use a packaged stdio transport"),
        };
        let config_result = crate::mcp::call_tool_with_transport(
            transport,
            "read_file",
            serde_json::json!({
                "path": std::path::Path::new(&home)
                    .join(".claude-server-commander/config.json")
                    .to_string_lossy()
            }),
        )
        .unwrap();
        assert!(!config_result["isError"].as_bool().unwrap_or(false));
        let installed_path = root.join("desktop-commander/0.2.47");
        store.remove_plugin("desktop-commander").unwrap();
        assert!(!installed_path.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn manifest_rejects_parent_traversal() {
        let document = r#"{
            "manifest_version": 1,
            "plugin": {"id": "unsafe", "version": "1.0.0", "publisher": "test"},
            "presentation": {"name": "Unsafe", "description": "", "readme": "README.md", "icon": "icon.svg"},
            "components": [{"type": "mcp", "id": "unsafe", "manifest": "../mcp.json"}]
        }"#;

        assert!(PluginManifest::parse(document).is_err());
    }

    #[test]
    fn manifest_validates_optional_github_repository_url() {
        let valid = r#"{
            "manifest_version": 1,
            "plugin": {"id": "safe", "version": "1.0.0", "publisher": "test"},
            "presentation": {"name": "Safe", "description": "", "readme": "README.md", "icon": "icon.svg", "repository_url": "https://github.com/example/safe"},
            "components": [{"type": "mcp", "id": "safe", "manifest": "mcp.json"}]
        }"#;
        let invalid = valid.replace(
            "https://github.com/example/safe",
            "https://example.com/example/safe",
        );

        assert!(PluginManifest::parse(valid).is_ok());
        assert!(PluginManifest::parse(&invalid).is_err());
    }

    #[test]
    fn marketplace_index_rejects_invalid_repository_url() {
        let index = r#"{
            "index_version": 1,
            "plugins": [{
                "id": "safe",
                "versions": [{
                    "version": "1.0.0",
                    "components": ["mcp"],
                    "capabilities": [],
                    "presentation": {
                        "name": "Safe",
                        "description": "Safe plugin.",
                        "repository_url": "https://example.com/safe"
                    },
                    "manifest_url": "packages/safe/plugin.json",
                    "artifact_url": "packages/safe",
                    "digest": "bundled",
                    "publisher": "test",
                    "status": "verified"
                }]
            }]
        }"#;

        assert!(MarketplaceIndex::parse(index).is_err());
    }
}
