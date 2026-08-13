//! Trusted plugins compiled into Windie.
//!
//! Curated plugins are explicit product-owned compositions. Their skill
//! assets are embedded by the build, while their MCP servers must still be
//! present in the approved MCP catalog before attachment can succeed.

use crate::skills::SkillId;
use crate::tool::ToolProviderId;

use super::manifest::{PluginId, PluginManifest, PluginVersion};

/// Stable identity of the code-owned CUA Driver plugin.
pub const CUA_DRIVER_PLUGIN_ID: &str = "cua-driver";

/// Stable identity of the code-owned CUA Driver skill.
pub const CUA_DRIVER_SKILL_ID: &str = "cua-driver";

/// Stable identity of the approved CUA Driver MCP server.
pub const CUA_DRIVER_MCP_ID: &str = "cua-driver";

/// Returns the trusted CUA Driver plugin composition.
pub(crate) fn cua_driver() -> PluginManifest {
    PluginManifest {
        plugin_id: PluginId::new(CUA_DRIVER_PLUGIN_ID),
        version: PluginVersion::new("0.1.0"),
        display_name: "CUA Driver".to_string(),
        description:
            "Use approved local computer-control tools through a repeatable driver workflow."
                .to_string(),
        skills: vec![SkillId::new(CUA_DRIVER_SKILL_ID)],
        mcp_servers: vec![ToolProviderId::new(CUA_DRIVER_MCP_ID)],
    }
}
