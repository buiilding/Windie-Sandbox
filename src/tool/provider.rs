//! Tool provider identity types.
//!
//! Provider types describe the concrete executor behind a model-facing tool
//! schema. They do not load provider catalogs or execute calls; that behavior
//! belongs to `tool_provider`.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
/// Stable identifier for one tool provider.
///
/// Examples are `cua-driver`, `desktop-commander`, or a future plugin package
/// ID. The ID names the execution provider, not the model-facing function name.
pub struct ToolProviderId(String);

impl ToolProviderId {
    /// Wraps provider ID text so provider identity stays type-distinct from
    /// schema names and provider tool names.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Exposes the provider ID at persistence, API, and display boundaries.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ToolProviderId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
/// Provider-native name for one executable tool.
///
/// This can differ from the model-facing schema name when a provider's tools
/// need a namespace to avoid collisions.
pub struct ProviderToolName(String);

impl ProviderToolName {
    /// Wraps provider-native tool name text.
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    /// Exposes the provider-native name at dispatch boundaries.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ProviderToolName {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Kind of execution provider behind an attached tool.
pub enum ToolProviderKind {
    Mcp,
    Plugin,
}

impl ToolProviderKind {
    /// Converts persisted provider kind text into the typed enum.
    pub fn from_storage(value: &str) -> Option<Self> {
        match value {
            "mcp" => Some(Self::Mcp),
            "plugin" => Some(Self::Plugin),
            _ => None,
        }
    }

    /// Returns the stable storage representation.
    pub fn as_storage(self) -> &'static str {
        match self {
            Self::Mcp => "mcp",
            Self::Plugin => "plugin",
        }
    }
}

impl std::fmt::Display for ToolProviderKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_storage())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// Pointer from a model-facing tool schema to the provider tool that executes
/// it.
pub struct ToolProviderRef {
    pub provider_id: ToolProviderId,
    pub tool_name: ProviderToolName,
    pub kind: ToolProviderKind,
}

impl ToolProviderRef {
    /// Creates a provider reference from typed provider identity parts.
    pub fn new(
        provider_id: ToolProviderId,
        tool_name: ProviderToolName,
        kind: ToolProviderKind,
    ) -> Self {
        Self {
            provider_id,
            tool_name,
            kind,
        }
    }
}
