//! Tool domain boundary.
//!
//! This module owns the typed contracts shared by tool catalog, path-scoped
//! model exposure, approval policy, runtime approval, and provider execution.
//! Runtime/session code decides when tool calls are pending and where results
//! are stored; this module defines what a tool is and how execution is
//! permitted.

pub mod approval;
pub mod policy;
pub mod provider;
pub mod result;
pub mod schema;

pub use approval::{ToolApprovalMode, ToolApprovalRequest};
pub use policy::{PolicyDecision, ToolPolicy};
pub use provider::{ProviderToolName, ToolProviderId, ToolProviderKind, ToolProviderRef};
pub use result::ToolExecutionResult;
pub use schema::{
    AttachedTool, ToolAnnotations, ToolDefinition, ToolPermission, ToolSchema, ToolSchemaName,
};
