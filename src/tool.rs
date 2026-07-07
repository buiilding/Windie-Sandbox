//! Tool domain types.
//!
//! This module owns the typed contracts shared by tool catalog, attachment,
//! policy, runtime approval, and provider execution. A tool provider can be a
//! Windie built-in, an MCP server, or a future plugin. Runtime code should pass
//! through these types instead of branching on one concrete executor.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::conversation::{
    MessageId, ToolCall, ToolCallId, ToolSchema, ToolSchemaName, UnsavedMessagePart,
};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
/// Stable identifier for one tool provider.
///
/// Examples are `windie`, `cua-driver`, or a future plugin package ID. The ID
/// names the execution provider, not the model-facing function name.
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
    BuiltIn,
    Mcp,
    Plugin,
}

impl ToolProviderKind {
    /// Converts persisted provider kind text into the typed enum.
    pub fn from_storage(value: &str) -> Option<Self> {
        match value {
            "built_in" => Some(Self::BuiltIn),
            "mcp" => Some(Self::Mcp),
            "plugin" => Some(Self::Plugin),
            _ => None,
        }
    }

    /// Returns the stable storage representation.
    pub fn as_storage(self) -> &'static str {
        match self {
            Self::BuiltIn => "built_in",
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Conversation-level default for tool execution approval.
///
/// The mode only applies after the requested model-facing tool is attached to
/// the conversation and backed by a registered provider. Unattached tools and
/// missing executors are still denied by policy.
pub enum ToolApprovalMode {
    Manual,
    AutoApproveAttached,
}

impl ToolApprovalMode {
    /// Converts persisted approval mode text into the typed enum.
    pub fn from_storage(value: &str) -> Option<Self> {
        match value {
            "manual" => Some(Self::Manual),
            "auto_approve_attached" => Some(Self::AutoApproveAttached),
            _ => None,
        }
    }

    /// Returns the stable storage/API representation.
    pub fn as_storage(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::AutoApproveAttached => "auto_approve_attached",
        }
    }
}

impl std::fmt::Display for ToolApprovalMode {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Permission lane used by policy before a provider execution.
pub enum ToolPermission {
    LocalShell,
    ExternalProcess,
    PluginCode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
/// Optional UI/runtime metadata for an available tool.
pub struct ToolAnnotations {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_only: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
/// One provider-owned tool available to attach to a conversation.
///
/// Availability alone does not grant model access. A client must attach this
/// definition to a conversation before the schema is sent to Bifrost.
pub struct ToolDefinition {
    #[serde(rename = "name")]
    pub schema_name: ToolSchemaName,
    pub display_name: String,
    pub description: String,
    pub parameters: Value,
    pub provider: ToolProviderRef,
    pub permissions: Vec<ToolPermission>,
    pub annotations: ToolAnnotations,
}

impl ToolDefinition {
    /// Converts an available provider tool into the persisted attachment shape.
    pub fn attached_tool(&self) -> AttachedTool {
        AttachedTool {
            schema_name: self.schema_name.clone(),
            description: self.description.clone(),
            parameters: self.parameters.clone(),
            provider: self.provider.clone(),
            permissions: self.permissions.clone(),
            annotations: self.annotations.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
/// One conversation's explicit opt-in to a provider tool.
///
/// Attached tools are the permission boundary for model visibility. Runtime
/// sends their schema subset to the model and uses the provider reference to
/// dispatch an approved tool call.
pub struct AttachedTool {
    #[serde(rename = "name")]
    pub schema_name: ToolSchemaName,
    pub description: String,
    pub parameters: Value,
    pub provider: ToolProviderRef,
    pub permissions: Vec<ToolPermission>,
    pub annotations: ToolAnnotations,
}

impl AttachedTool {
    /// Returns the model-facing schema subset sent with a Responses request.
    pub fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.schema_name.clone(),
            description: self.description.clone(),
            parameters: self.parameters.clone(),
        }
    }

    /// Builds an attached tool from a raw schema. This keeps the existing
    /// low-level schema insert primitive as a developer escape hatch while
    /// still storing provider metadata. Because no registered provider owns
    /// `manual`, runtime policy treats it as an unknown executor.
    pub fn manual(schema: ToolSchema) -> Self {
        Self {
            schema_name: schema.name.clone(),
            description: schema.description,
            parameters: schema.parameters,
            provider: ToolProviderRef::new(
                ToolProviderId::new("manual"),
                ProviderToolName::new(schema.name.as_str()),
                ToolProviderKind::Plugin,
            ),
            permissions: Vec::new(),
            annotations: ToolAnnotations::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
/// One pending model-requested tool call that requires user approval.
///
/// The assistant message ID identifies the assistant turn that requested the
/// call. Runtime decides the exact parent for the later `role: tool` result so
/// multi-tool results can stay on one linear active path.
pub struct ToolApprovalRequest {
    pub assistant_message_id: MessageId,
    pub tool_call: ToolCall,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
/// Result of one tool execution ready to be stored as a `role: tool` message.
pub struct ToolExecutionResult {
    pub tool_call_id: ToolCallId,
    pub tool_name: String,
    /// Compact visible preview stored on the message row.
    pub content: String,
    /// Ordered model-facing content parts. Text-only tools can leave this empty
    /// and use `content`; rich tools such as MCP screenshot providers store
    /// text/image parts here so Responses can replay them without base64 text
    /// bloat.
    #[serde(skip)]
    pub parts: Vec<UnsavedMessagePart>,
    pub success: bool,
}

impl ToolExecutionResult {
    /// Creates a successful tool result with model-facing content.
    pub fn success(
        tool_call_id: ToolCallId,
        tool_name: impl Into<String>,
        content: String,
    ) -> Self {
        Self {
            tool_call_id,
            tool_name: tool_name.into(),
            content,
            parts: Vec::new(),
            success: true,
        }
    }

    /// Creates a successful rich tool result with ordered model-facing parts.
    pub fn success_with_parts(
        tool_call_id: ToolCallId,
        tool_name: impl Into<String>,
        content: String,
        parts: Vec<UnsavedMessagePart>,
    ) -> Self {
        Self {
            tool_call_id,
            tool_name: tool_name.into(),
            content,
            parts,
            success: true,
        }
    }

    /// Creates a failed tool result with a short model-facing error.
    pub fn failure(
        tool_call_id: ToolCallId,
        tool_name: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            tool_call_id,
            tool_name: tool_name.into(),
            content: serde_json::json!({ "error": reason.into() }).to_string(),
            parts: Vec::new(),
            success: false,
        }
    }
}
