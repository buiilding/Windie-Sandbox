//! Typed metadata for plugins that compose skills with MCP servers.

use serde::{Deserialize, Serialize};

use crate::skills::SkillId;
use crate::tool::ToolProviderId;

/// Describes which extension primitives a package contains.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ExtensionComposition {
    /// A package containing both skills and MCP servers.
    Plugin,
    /// A package containing only skills.
    Skill,
    /// A package containing only MCP servers.
    Mcp,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
/// Stable identity for one Windie plugin.
pub struct PluginId(String);

impl PluginId {
    /// Creates a typed plugin identity.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Returns the stable identifier at runtime and display boundaries.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A model-facing extension attachment target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtensionTarget {
    /// Attach every MCP server declared by one plugin package.
    Plugin(PluginId),
    /// Attach one standalone or package-declared MCP server.
    Mcp(ToolProviderId),
}

impl std::fmt::Display for PluginId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
/// Version of one curated plugin package.
pub struct PluginVersion(String);

impl PluginVersion {
    /// Creates a typed plugin version.
    pub fn new(version: impl Into<String>) -> Self {
        Self(version.into())
    }

    /// Returns the version at display and catalog boundaries.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// Composition metadata for one plugin.
pub struct PluginManifest {
    pub plugin_id: PluginId,
    pub version: PluginVersion,
    pub display_name: String,
    pub description: String,
    /// Skills the plugin makes relevant to the model.
    pub skills: Vec<SkillId>,
    /// MCP servers whose discovered tools the plugin can activate.
    pub mcp_servers: Vec<ToolProviderId>,
}

impl PluginManifest {
    /// Classifies the package for catalog and Inspector presentation.
    pub fn composition(&self) -> ExtensionComposition {
        match (self.skills.is_empty(), self.mcp_servers.is_empty()) {
            (false, false) => ExtensionComposition::Plugin,
            (false, true) => ExtensionComposition::Skill,
            (true, false) => ExtensionComposition::Mcp,
            (true, true) => unreachable!("validated packages cannot be empty"),
        }
    }
}
