//! Tool domain boundary.
//!
//! This module owns the typed contracts shared by tool catalog, path-scoped
//! model exposure, approval policy, runtime approval, and provider execution.
//! Runtime/session code decides when tool calls are pending and where results
//! are stored; this module defines what a tool is and how execution is
//! permitted.

pub mod approval;
mod builtin;
mod lifecycle;
mod manifest;
pub mod policy;
pub mod provider;
mod registry;
pub mod result;
pub mod schema;

#[cfg(test)]
mod tests;

pub use approval::{ToolApprovalMode, ToolApprovalRequest};
pub(crate) use builtin::{ATTACH_MCP_TOOL_NAME, BUILTIN_PROVIDER_ID, READ_SKILL_TOOL_NAME};
pub use lifecycle::{ProviderInstallState, ProviderReadiness};
pub use manifest::{
    ProviderAuthentication, ProviderDependency, ProviderLaunch, ProviderManifest, ProviderPackage,
    ProviderPackageManager, ProviderPermission, ProviderPlatform, ProviderRuntime, ProviderScope,
    ProviderSecret, ProviderTransport,
};
pub use policy::{PolicyDecision, ToolPolicy};
pub use provider::{ProviderToolName, ToolProviderId, ToolProviderKind, ToolProviderRef};
pub use registry::ToolProviderRegistry;
pub use result::ToolExecutionResult;
pub use schema::{
    AttachedTool, ToolAnnotations, ToolDefinition, ToolPermission, ToolSchema, ToolSchemaName,
};
