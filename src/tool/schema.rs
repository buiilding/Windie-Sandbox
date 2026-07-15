//! Model-facing tool schema and conversation exposure types.
//!
//! A tool schema is what the model can see and call. An attached tool is the
//! same model-facing schema plus the provider pointer and permission metadata
//! needed to execute it after policy allows the call.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{ProviderToolName, ToolProviderId, ToolProviderKind, ToolProviderRef};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
/// Stable name for one path-scoped tool schema.
///
/// This is the model-facing name used in LLM tool calls. It is distinct from a
/// provider's native tool name because attached tools can be namespaced before
/// they are exposed to the model.
pub struct ToolSchemaName(String);

impl ToolSchemaName {
    /// Wraps raw tool schema name text so tool schema identity stays
    /// type-distinct from general strings.
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    /// Exposes the tool schema name as plain text at persistence, request, and
    /// display boundaries.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Returns whether the name is valid for OpenAI-compatible function tool
    /// schemas.
    ///
    /// Windie keeps this rule on the typed name so CLI parsing, persistence,
    /// and future clients can share one contract.
    pub fn is_valid(&self) -> bool {
        let name = self.as_str();

        !name.is_empty()
            && name.len() <= 64
            && name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    }
}

impl std::fmt::Display for ToolSchemaName {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl std::ops::Deref for ToolSchemaName {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
/// Tool definition that can be sent to the model.
///
/// This is only the schema. It does not execute the tool and does not grant any
/// permission. Execution must go through explicit runtime boundaries.
pub struct ToolSchema {
    pub name: ToolSchemaName,
    pub description: String,
    pub parameters: Value,
}

impl ToolSchema {
    /// Returns whether the human-facing description carries meaningful text.
    pub fn has_valid_description(&self) -> bool {
        !self.description.trim().is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Permission lane used by policy before a provider execution.
pub enum ToolPermission {
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
/// Availability alone does not grant model access. A client must expose this
/// definition on a conversation path before the schema is sent to Bifrost.
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
    /// Converts an available provider tool into the persisted path exposure.
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
/// One conversation path's explicit exposure of a provider tool.
///
/// This is the permission boundary for model visibility. Runtime sends the
/// schema subset to the model and uses the provider reference to dispatch an
/// approved tool call.
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

    /// Builds a path-exposed tool from a raw schema.
    ///
    /// This keeps the existing low-level schema insert primitive as a
    /// developer escape hatch while still storing provider metadata. Because no
    /// registered provider owns `manual`, runtime policy treats it as an
    /// unknown executor.
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
