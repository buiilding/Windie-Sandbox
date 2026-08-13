//! Installation of trusted curated plugins.
//!
//! Curated plugin definitions are Windie-owned trust and installation recipes.
//! Their installed files still use the same package shape as marketplace
//! plugins, so discovery, skill loading, hashing, and MCP composition have one
//! runtime implementation.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow};
use serde_json::json;

use crate::local;
use crate::mcp::{CuaDriverSkillPack, install_cua_driver_skills};
use crate::tool::ToolProviderId;

use super::curated::{CUA_DRIVER_MCP_ID, CUA_DRIVER_PLUGIN_ID};
use super::package::{PluginPackage, copy_directory, install_local_package};

/// Materializes the upstream CUA Driver executable and skill pack as a
/// versioned Windie plugin package.
///
/// The executable is installed by the existing approved MCP installer before
/// this function is called. This function installs the upstream skill pack,
/// validates its directory, creates the package manifests, validates the
/// resulting package through the normal package loader, and copies it into
/// Windie's package store.
pub(crate) fn materialize_cua_driver_plugin() -> Result<PathBuf> {
    let skill_pack = install_cua_driver_skills()?;
    let package_root = staging_root(&skill_pack)?;

    let result = build_cua_driver_package(&package_root, &skill_pack)
        .and_then(|_| install_package(&package_root));
    let cleanup_result = remove_staging_root(&package_root);

    match (result, cleanup_result) {
        (Ok(path), Ok(())) => Ok(path),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(error)) => Err(error),
        (Err(error), Err(cleanup_error)) => Err(error.context(format!(
            "failed to clean up CUA Driver plugin staging directory: {cleanup_error}"
        ))),
    }
}

/// Installs the curated plugin associated with one MCP dependency, if any.
pub(crate) fn install_curated_plugin_for_provider(provider_id: &ToolProviderId) -> Result<bool> {
    if provider_id.as_str() != CUA_DRIVER_MCP_ID {
        return Ok(false);
    }
    materialize_cua_driver_plugin()?;
    Ok(true)
}

/// Removes all Windie-managed installed versions of one curated plugin.
pub(crate) fn remove_curated_plugin(plugin_id: &str) -> Result<()> {
    let plugins_root = local::windie_home_dir()?.join("plugins");
    let plugin_root = plugins_root.join(plugin_id);
    match fs::symlink_metadata(&plugin_root) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => Err(anyhow!(
            "refusing to remove non-directory curated plugin path: {}",
            plugin_root.display()
        )),
        Ok(_) => fs::remove_dir_all(&plugin_root).with_context(|| {
            format!(
                "failed to remove installed curated plugin: {}",
                plugin_root.display()
            )
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| {
            format!(
                "failed to inspect installed curated plugin: {}",
                plugin_root.display()
            )
        }),
    }
}

/// Removes the curated plugin associated with one MCP dependency, if any.
pub(crate) fn remove_curated_plugin_for_provider(provider_id: &ToolProviderId) -> Result<bool> {
    if provider_id.as_str() != CUA_DRIVER_MCP_ID {
        return Ok(false);
    }
    remove_curated_plugin(CUA_DRIVER_PLUGIN_ID)?;
    Ok(true)
}

fn staging_root(skill_pack: &CuaDriverSkillPack) -> Result<PathBuf> {
    let plugins_root = local::windie_home_dir()?.join("plugins");
    fs::create_dir_all(&plugins_root)
        .with_context(|| format!("failed to create plugin store: {}", plugins_root.display()))?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| anyhow!("system clock is before UNIX epoch: {error}"))?
        .as_nanos();
    let version = &skill_pack.driver_version;
    let staging = plugins_root.join(format!(
        ".staging-{CUA_DRIVER_PLUGIN_ID}-{version}-{timestamp}"
    ));
    if staging.exists() {
        return Err(anyhow!(
            "refusing to reuse existing plugin staging directory: {}",
            staging.display()
        ));
    }
    Ok(staging)
}

fn build_cua_driver_package(root: &Path, skill_pack: &CuaDriverSkillPack) -> Result<()> {
    let manifest_root = root.join(".codex-plugin");
    let skill_root = root.join("skills").join(CUA_DRIVER_PLUGIN_ID);
    fs::create_dir_all(&manifest_root).with_context(|| {
        format!(
            "failed to create plugin manifest directory: {}",
            manifest_root.display()
        )
    })?;
    fs::create_dir_all(&skill_root).with_context(|| {
        format!(
            "failed to create plugin skill directory: {}",
            skill_root.display()
        )
    })?;

    copy_directory(&skill_pack.root, &skill_root)?;
    if !skill_root.join("SKILL.md").is_file() {
        return Err(anyhow!(
            "upstream CUA Driver skill pack is missing SKILL.md: {}",
            skill_pack.root.display()
        ));
    }

    let plugin_manifest = json!({
        "name": CUA_DRIVER_PLUGIN_ID,
        "version": skill_pack.driver_version,
        "description": "Use approved local computer-control tools through a repeatable driver workflow.",
        "author": {
            "name": "TryCua",
            "url": "https://cua.ai/docs/cua-driver"
        },
        "skills": "./skills/",
        "mcpServers": "./.mcp.json",
        "interface": { "displayName": "CUA Driver" }
    });
    write_json(&manifest_root.join("plugin.json"), &plugin_manifest)?;

    let mcp_manifest = json!({
        "mcpServers": {
            CUA_DRIVER_MCP_ID: {
                "command": "cua-driver",
                "args": ["mcp"],
                "cwd": "."
            }
        }
    });
    write_json(&root.join(".mcp.json"), &mcp_manifest)?;

    let provenance = json!({
        "source": "https://github.com/trycua/cua",
        "skill_source": "cua-driver skills install --all-platforms",
        "driver_version": skill_pack.driver_version,
        "skill_root": skill_pack.root,
        "installed_by": "windie-curated-plugin"
    });
    write_json(&root.join("windie-provenance.json"), &provenance)?;
    Ok(())
}

fn write_json(path: &Path, value: &serde_json::Value) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(value).context("failed to serialize plugin metadata")?;
    fs::write(path, bytes)
        .with_context(|| format!("failed to write plugin metadata: {}", path.display()))?;
    Ok(())
}

fn install_package(staging_root: &Path) -> Result<PathBuf> {
    let package = PluginPackage::load(staging_root)
        .context("generated curated CUA Driver package failed validation")?;
    let destination_root = local::windie_home_dir()?.join("plugins");
    install_local_package(package.root(), destination_root)
}

fn remove_staging_root(root: &Path) -> Result<()> {
    match fs::symlink_metadata(root) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => Err(anyhow!(
            "refusing to remove non-directory plugin staging path: {}",
            root.display()
        )),
        Ok(_) => fs::remove_dir_all(root).with_context(|| {
            format!(
                "failed to remove plugin staging directory: {}",
                root.display()
            )
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| {
            format!(
                "failed to inspect plugin staging directory: {}",
                root.display()
            )
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "windie-curated-plugin-{label}-{}",
            std::process::id()
        ))
    }

    #[test]
    fn builds_a_valid_codex_shaped_cua_package_from_an_upstream_skill_root() {
        let source = fixture_root("source");
        let package = fixture_root("package");
        fs::create_dir_all(&source).unwrap();
        fs::write(
            source.join("SKILL.md"),
            "---\ndescription: CUA workflow\n---\nRead MACOS.md.",
        )
        .unwrap();
        fs::write(source.join("MACOS.md"), "macOS instructions").unwrap();

        let skill_pack = CuaDriverSkillPack {
            driver_version: "1.2.3".to_string(),
            root: source.clone(),
        };
        build_cua_driver_package(&package, &skill_pack).unwrap();
        let loaded = PluginPackage::load(&package).unwrap();

        assert_eq!(loaded.plugin_id().as_str(), CUA_DRIVER_PLUGIN_ID);
        assert_eq!(loaded.version().as_str(), "1.2.3");
        assert!(
            loaded
                .read_skill(&crate::skills::SkillId::new(CUA_DRIVER_PLUGIN_ID), None)
                .unwrap()
                .available_files
                .iter()
                .any(|path| path.as_str() == "MACOS.md")
        );
        assert!(package.join(".codex-plugin/plugin.json").is_file());
        assert!(package.join(".mcp.json").is_file());
        assert!(package.join("windie-provenance.json").is_file());

        let _ = fs::remove_dir_all(source);
        let _ = fs::remove_dir_all(package);
    }
}
