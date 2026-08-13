//! Plugin composition for Windie's model-facing extension surface.
//!
//! A plugin groups reusable skills with approved MCP servers. Plugins do not
//! implement MCP transport or tool execution; they delegate those concerns to
//! `skills`, `mcp`, and the runtime policy boundary.

mod curated;
mod installation;
mod manifest;
mod marketplace;
mod package;
mod registry;

pub(crate) use installation::{
    install_curated_plugin_for_provider, remove_curated_plugin_for_provider,
};
pub use manifest::{
    ExtensionComposition, ExtensionTarget, PluginId, PluginManifest, PluginVersion,
};
pub use marketplace::{MarketplaceManifest, MarketplacePlugin, MarketplaceSource};
pub use package::{
    PackageMcpServer, PluginPackage, install_local_package, install_local_package_into_windie,
};
pub use registry::PluginRegistry;
