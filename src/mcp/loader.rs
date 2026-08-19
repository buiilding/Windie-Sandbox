//! MCP component loading from installed plugin packages.
//!
//! The plugin store owns package files and versioned installation. This loader
//! is the MCP-specific boundary that turns an installed package component into
//! a transport ready for discovery and execution.

use std::fs;

use anyhow::{Result, anyhow, bail};

use crate::mcp::{McpOwnedCommand, McpTransport};
use crate::plugin::InstalledPlugin;
use crate::plugin::manifest::{
    McpServerManifest, PluginComponentKind, PluginManifest, WindieMcpMetadata, resolve_package_path,
};

use super::mcpb::McpbRuntime;

#[derive(Debug, Clone)]
/// One validated MCP component loaded from an installed plugin package.
pub(crate) struct LoadedMcpComponent {
    pub plugin: PluginManifest,
    pub component_id: String,
    pub manifest: McpServerManifest,
    pub windie: WindieMcpMetadata,
    pub transport: McpTransport,
    pub package_command: Option<McpOwnedCommand>,
    pub readme: String,
}

/// Loads all MCP components from one installed plugin.
pub(crate) fn load_components(plugin: &InstalledPlugin) -> Result<Vec<LoadedMcpComponent>> {
    plugin
        .manifest
        .components
        .iter()
        .filter(|component| component.kind == PluginComponentKind::Mcp)
        .map(|component| {
            let path = resolve_package_path(&plugin.root, &component.manifest)?;
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
                    let artifact_path = resolve_package_path(&plugin.root, artifact)?;
                    let runtime_root = plugin.root.join("runtime").join(&component.id);
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
                resolve_package_path(&plugin.root, &plugin.manifest.presentation.readme)?;
            let readme = fs::read_to_string(readme_path).unwrap_or_default();
            Ok(LoadedMcpComponent {
                plugin: plugin.manifest.clone(),
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
