//! Tool provider registry and dispatch.
//!
//! This module is the execution boundary for tool providers. Runtime asks this
//! registry which tools are available and asks it to execute an approved tool
//! call through the provider reference stored on the conversation path. MCP is
//! the only implemented backend family today; built-in, skill, and plugin
//! backends can later join through the same registry shape.

mod mcp;
mod registry;

#[cfg(test)]
mod tests;

pub use registry::{ToolProviderRegistry, ToolProviderStatus};
