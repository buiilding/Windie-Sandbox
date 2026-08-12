//! Plugin composition for Windie's model-facing extension surface.
//!
//! A plugin groups reusable skills with approved MCP servers. Plugins do not
//! implement MCP transport or tool execution; they delegate those concerns to
//! `skills`, `mcp`, and the runtime policy boundary.

mod manifest;
mod registry;

pub use manifest::{PluginId, PluginManifest, PluginVersion};
pub use registry::PluginRegistry;
