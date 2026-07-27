//! Tool provider registry and dispatch.
//!
//! This module is the execution boundary for tool providers. Runtime asks this
//! registry which tools are available and asks it to execute an approved tool
//! call through the provider reference stored on the conversation path. MCP is
//! the only implemented backend family today; built-in, skill, and plugin
//! backends can later join through the same registry shape.

mod builtin;
mod lifecycle;
mod manifest;
mod mcp;
mod registry;

#[cfg(test)]
mod tests;

pub(crate) use builtin::{
    ATTACH_PROVIDER_TOOL_NAME, BUILTIN_PROVIDER_ID, LIST_PROVIDERS_TOOL_NAME,
};
pub use lifecycle::ProviderInstallState;
pub use manifest::{
    ProviderAuthentication, ProviderDependency, ProviderManifest, ProviderPermission,
    ProviderPlatform, ProviderScope, ProviderSecret,
};
pub use registry::{ToolProviderRegistry, ToolProviderStatus};
