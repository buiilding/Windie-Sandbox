//! Tool provider registry and dispatch.
//!
//! This module is the execution boundary for tool providers. Runtime asks this
//! registry which tools are available and asks it to execute an approved tool
//! call through the provider reference stored on the conversation attachment.
//! Built-ins, MCP servers, and future plugins should enter runtime through this
//! same registry shape.

use anyhow::{Context, Result, anyhow};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Value, json};

use crate::conversation::{ToolCall, ToolSchemaName, UnsavedImagePart, UnsavedMessagePart};
use crate::error;
use crate::mcp::{self, McpCommand, McpTool};
use crate::shell::ShellExecutor;
use crate::tool::{
    AttachedTool, ProviderToolName, ToolAnnotations, ToolDefinition, ToolExecutionResult,
    ToolPermission, ToolProviderId, ToolProviderKind, ToolProviderRef,
};

const WINDIE_PROVIDER_ID: &str = "windie";
const RUN_SHELL_TOOL_NAME: &str = "run_shell";

#[derive(Debug, Clone)]
/// Registry of tool providers available to this Windie process.
///
/// The registry deliberately exposes provider-neutral operations. Runtime does
/// not branch on shell, MCP, or plugin details; it resolves the conversation's
/// attached tool to a provider reference and calls this registry.
pub struct ToolProviderRegistry {
    built_in: BuiltInToolProvider,
    mcp_providers: Vec<McpToolProvider>,
    persistent_mcp_calls: bool,
}

impl ToolProviderRegistry {
    /// Builds the default registry for the local Windie process.
    pub fn new() -> Self {
        Self::default()
    }

    /// Builds a registry whose MCP tool calls reuse persistent provider
    /// sessions.
    ///
    /// The API server uses this shape because it lives long enough for idle
    /// cleanup to matter. CLI commands keep the default short-lived execution
    /// path because each CLI invocation is a separate process.
    pub fn with_persistent_mcp_sessions() -> Self {
        Self {
            persistent_mcp_calls: true,
            ..Self::default()
        }
    }

    /// Lists every provider tool that clients may attach to conversations.
    ///
    /// Availability does not grant model access. Clients still need to attach a
    /// returned definition before the model sees the function schema.
    pub fn list_available_tools(&self) -> Result<Vec<ToolDefinition>> {
        let mut tools = self.built_in.list_tools();
        for provider in &self.mcp_providers {
            if let Ok(provider_tools) = provider.list_tools() {
                tools.extend(provider_tools);
            }
        }

        Ok(tools)
    }

    /// Lists available tools for one provider ID.
    pub fn list_provider_tools(&self, provider_id: &ToolProviderId) -> Result<Vec<ToolDefinition>> {
        if provider_id.as_str() == WINDIE_PROVIDER_ID {
            return Ok(self.built_in.list_tools());
        }
        if let Some(provider) = self.mcp_provider(provider_id) {
            return provider.list_tools();
        }

        Ok(Vec::new())
    }

    /// Finds one available provider tool by provider ID and provider-native
    /// tool name.
    pub fn find_tool(
        &self,
        provider_id: &ToolProviderId,
        tool_name: &ProviderToolName,
    ) -> Result<Option<ToolDefinition>> {
        Ok(self
            .list_provider_tools(provider_id)?
            .into_iter()
            .find(|tool| tool.provider.tool_name == *tool_name))
    }

    /// Returns whether this process has an executor for the attached provider
    /// tool.
    pub fn can_execute(&self, attached_tool: &AttachedTool) -> bool {
        match attached_tool.provider.kind {
            ToolProviderKind::BuiltIn => {
                attached_tool.provider.provider_id.as_str() == WINDIE_PROVIDER_ID
            }
            ToolProviderKind::Mcp => self
                .mcp_provider(&attached_tool.provider.provider_id)
                .is_some(),
            ToolProviderKind::Plugin => false,
        }
    }

    /// Executes one approved model tool call through its attached provider.
    pub async fn call_tool(
        &self,
        attached_tool: &AttachedTool,
        tool_call: &ToolCall,
    ) -> Result<ToolExecutionResult> {
        match attached_tool.provider.kind {
            ToolProviderKind::BuiltIn => self.built_in.call_tool(attached_tool, tool_call).await,
            ToolProviderKind::Mcp => {
                let Some(provider) = self.mcp_provider(&attached_tool.provider.provider_id) else {
                    return Err(error::invalid_request(format!(
                        "unknown tool: {}",
                        tool_call.name()
                    )));
                };

                provider
                    .call_tool(attached_tool, tool_call, self.persistent_mcp_calls)
                    .await
            }
            ToolProviderKind::Plugin => Err(error::invalid_request(format!(
                "unknown tool: {}",
                tool_call.name()
            ))),
        }
    }

    /// Finds one approved MCP provider by its stable provider ID.
    fn mcp_provider(&self, provider_id: &ToolProviderId) -> Option<&McpToolProvider> {
        self.mcp_providers
            .iter()
            .find(|provider| provider.id() == provider_id)
    }
}

impl Default for ToolProviderRegistry {
    fn default() -> Self {
        Self {
            built_in: BuiltInToolProvider,
            mcp_providers: APPROVED_MCP_PROVIDERS
                .iter()
                .copied()
                .map(McpToolProvider::new)
                .collect(),
            persistent_mcp_calls: false,
        }
    }
}

#[derive(Debug, Default, Clone, Copy)]
/// Provider for Windie-native tools compiled into the local runtime.
pub struct BuiltInToolProvider;

impl BuiltInToolProvider {
    /// Lists Windie's built-in provider tools.
    pub fn list_tools(&self) -> Vec<ToolDefinition> {
        vec![run_shell_tool_definition()]
    }

    /// Executes one built-in provider tool.
    async fn call_tool(
        &self,
        attached_tool: &AttachedTool,
        tool_call: &ToolCall,
    ) -> Result<ToolExecutionResult> {
        if attached_tool.provider.provider_id.as_str() != WINDIE_PROVIDER_ID
            || attached_tool.provider.tool_name.as_str() != RUN_SHELL_TOOL_NAME
            || tool_call.name() != attached_tool.schema_name.as_str()
        {
            return Err(error::invalid_request(format!(
                "unknown tool: {}",
                tool_call.name()
            )));
        }

        Ok(ShellExecutor::default().execute_tool_call(tool_call).await)
    }
}

/// Builds the provider-backed definition for Windie's bounded shell executor.
fn run_shell_tool_definition() -> ToolDefinition {
    ToolDefinition {
        schema_name: ToolSchemaName::new(RUN_SHELL_TOOL_NAME),
        display_name: "Run shell".to_string(),
        description: "Run a bounded local shell command after explicit user approval.".to_string(),
        parameters: json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "command": {
                    "type": "string",
                    "description": "Shell command to run through the user's default shell."
                },
                "cwd": {
                    "type": "string",
                    "description": "Optional working directory for the command."
                },
                "timeout_ms": {
                    "type": "integer",
                    "description": "Optional timeout in milliseconds, capped by Windie."
                }
            },
            "required": ["command"]
        }),
        provider: ToolProviderRef::new(
            ToolProviderId::new(WINDIE_PROVIDER_ID),
            ProviderToolName::new(RUN_SHELL_TOOL_NAME),
            ToolProviderKind::BuiltIn,
        ),
        permissions: vec![ToolPermission::LocalShell],
        annotations: ToolAnnotations {
            title: Some("Run shell".to_string()),
            read_only: Some(false),
        },
    }
}

#[derive(Debug, Clone, Copy)]
/// Static definition for one code-approved MCP provider.
///
/// This is intentionally data, not runtime state. Adding a future approved MCP
/// provider should add one entry to `APPROVED_MCP_PROVIDERS` while
/// keeping `McpToolProvider` generic.
struct McpProviderDefinition {
    provider_id: &'static str,
    schema_prefix: &'static str,
    display_name: &'static str,
    command: McpCommand,
    shutdown_command: Option<McpCommand>,
}

/// MCP providers Windie is willing to start and execute.
///
/// This is a code-owned allowlist, not user configuration. Provider
/// availability still does not grant model access; conversations must attach
/// individual tools before their schemas are sent to the model.
const APPROVED_MCP_PROVIDERS: &[McpProviderDefinition] = &[McpProviderDefinition {
    provider_id: "cua-driver",
    schema_prefix: "cua_driver",
    display_name: "CUA Driver",
    command: McpCommand {
        program: "cua-driver",
        args: &["mcp"],
    },
    shutdown_command: Some(McpCommand {
        program: "cua-driver",
        args: &["stop"],
    }),
}];

#[derive(Debug, Clone)]
/// Provider for an approved MCP stdio server.
pub struct McpToolProvider {
    provider_id: ToolProviderId,
    schema_prefix: &'static str,
    display_name: &'static str,
    command: McpCommand,
    shutdown_command: Option<McpCommand>,
}

impl McpToolProvider {
    /// Builds a runtime provider from a code-approved provider definition.
    fn new(definition: McpProviderDefinition) -> Self {
        Self {
            provider_id: ToolProviderId::new(definition.provider_id),
            schema_prefix: definition.schema_prefix,
            display_name: definition.display_name,
            command: definition.command,
            shutdown_command: definition.shutdown_command,
        }
    }

    /// Returns the stable provider ID used by attachments and dispatch.
    fn id(&self) -> &ToolProviderId {
        &self.provider_id
    }

    /// Lists tools from the MCP server and maps them into Windie definitions.
    pub fn list_tools(&self) -> Result<Vec<ToolDefinition>> {
        Ok(
            mcp::list_tools_with_shutdown(self.command, self.shutdown_command)?
                .into_iter()
                .map(|tool| self.definition_from_mcp_tool(tool))
                .collect(),
        )
    }

    /// Executes one approved MCP tool call.
    async fn call_tool(
        &self,
        attached_tool: &AttachedTool,
        tool_call: &ToolCall,
        persistent: bool,
    ) -> Result<ToolExecutionResult> {
        if attached_tool.provider.provider_id != self.provider_id
            || tool_call.name() != attached_tool.schema_name.as_str()
        {
            return Err(error::invalid_request(format!(
                "unknown tool: {}",
                tool_call.name()
            )));
        }
        let arguments = match serde_json::from_str::<Value>(tool_call.arguments()) {
            Ok(arguments) => arguments,
            Err(error) => {
                return Ok(ToolExecutionResult::failure(
                    tool_call.id.clone(),
                    tool_call.name(),
                    format!("invalid tool arguments: {error}"),
                ));
            }
        };
        let result = if persistent {
            mcp::call_tool_persistent(
                self.provider_id.as_str(),
                self.command,
                self.shutdown_command,
                attached_tool.provider.tool_name.as_str(),
                arguments,
            )
        } else {
            mcp::call_tool_with_shutdown(
                self.command,
                self.shutdown_command,
                attached_tool.provider.tool_name.as_str(),
                arguments,
            )
        }?;
        let success = !result
            .get("isError")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        let normalized = match mcp_tool_result_parts(&result) {
            Ok(parts) => parts,
            Err(error) => {
                return Ok(ToolExecutionResult::failure(
                    tool_call.id.clone(),
                    tool_call.name(),
                    error.to_string(),
                ));
            }
        };

        let mut execution_result = ToolExecutionResult::success_with_parts(
            tool_call.id.clone(),
            tool_call.name(),
            tool_result_preview(&normalized),
            normalized,
        );
        execution_result.success = success;

        Ok(execution_result)
    }

    /// Converts one MCP tool into Windie's provider-backed tool definition.
    fn definition_from_mcp_tool(&self, tool: McpTool) -> ToolDefinition {
        let read_only = tool
            .annotations
            .as_ref()
            .and_then(|annotations| annotations.read_only_hint)
            .unwrap_or(false);

        ToolDefinition {
            schema_name: ToolSchemaName::new(mcp_schema_name(self.schema_prefix, &tool.name)),
            display_name: format!("{} {}", self.display_name, tool.name),
            description: tool.description,
            parameters: tool.input_schema,
            provider: ToolProviderRef::new(
                self.provider_id.clone(),
                ProviderToolName::new(tool.name),
                ToolProviderKind::Mcp,
            ),
            permissions: vec![ToolPermission::ExternalProcess],
            annotations: ToolAnnotations {
                title: None,
                read_only: Some(read_only),
            },
        }
    }
}

/// Builds the model-facing schema name for one MCP provider tool.
fn mcp_schema_name(schema_prefix: &str, tool_name: &str) -> String {
    format!(
        "{schema_prefix}__{}",
        tool_name
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() || character == '_' || character == '-' {
                    character
                } else {
                    '_'
                }
            })
            .collect::<String>()
    )
}

/// Converts an MCP `tools/call` result into Windie's model-facing message
/// parts.
///
/// MCP can return text and binary images in the same content array. Windie
/// stores those images through `message_parts` and `image_assets` so the
/// Responses request can replay them as image blocks instead of base64 text.
fn mcp_tool_result_parts(result: &Value) -> Result<Vec<UnsavedMessagePart>> {
    let mut parts = Vec::new();

    if let Some(content) = result.get("content").and_then(Value::as_array) {
        for item in content {
            match item.get("type").and_then(Value::as_str) {
                Some("text") => {
                    if let Some(text) = item.get("text").and_then(Value::as_str) {
                        parts.push(UnsavedMessagePart::Text(text.to_string()));
                    }
                }
                Some("image") => {
                    let data = item
                        .get("data")
                        .and_then(Value::as_str)
                        .ok_or_else(|| anyhow!("MCP image result did not include data"))?;
                    let mime_type = item
                        .get("mimeType")
                        .or_else(|| item.get("mime_type"))
                        .and_then(Value::as_str)
                        .unwrap_or("image/png");
                    let bytes = STANDARD
                        .decode(data)
                        .context("failed to decode MCP image result")?;
                    parts.push(UnsavedMessagePart::Image(UnsavedImagePart {
                        mime_type: mime_type.to_string(),
                        bytes,
                    }));
                }
                Some(other) => parts.push(UnsavedMessagePart::Text(format!(
                    "Unsupported MCP content block: {other}"
                ))),
                None => {}
            }
        }
    }

    if let Some(structured_content) = result.get("structuredContent") {
        if !structured_content.is_null() {
            parts.push(UnsavedMessagePart::Text(format!(
                "structuredContent: {structured_content}"
            )));
        }
    }

    if parts.is_empty() {
        parts.push(UnsavedMessagePart::Text(result.to_string()));
    }

    Ok(parts)
}

/// Builds the compact visible text stored on the tool message row.
fn tool_result_preview(parts: &[UnsavedMessagePart]) -> String {
    let mut lines = Vec::new();

    for part in parts {
        match part {
            UnsavedMessagePart::Text(text) => lines.push(text.clone()),
            UnsavedMessagePart::Image(image) => lines.push(format!(
                "[image: {}, {} bytes]",
                image.mime_type,
                image.bytes.len()
            )),
        }
    }

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approved_cua_provider() -> McpToolProvider {
        let definition = APPROVED_MCP_PROVIDERS
            .iter()
            .copied()
            .find(|definition| definition.provider_id == "cua-driver")
            .unwrap();
        McpToolProvider::new(definition)
    }

    #[test]
    fn mcp_schema_names_are_provider_prefixed() {
        assert_eq!(mcp_schema_name("cua_driver", "click"), "cua_driver__click");
        assert_eq!(
            mcp_schema_name("cua_driver", "type text"),
            "cua_driver__type_text"
        );
    }

    #[test]
    fn cua_mcp_tools_map_to_provider_backed_definitions() {
        let provider = approved_cua_provider();
        let definition = provider.definition_from_mcp_tool(McpTool {
            name: "click".to_string(),
            description: "Click somewhere".to_string(),
            input_schema: json!({"type":"object"}),
            annotations: Some(mcp::McpToolAnnotations {
                read_only_hint: Some(false),
            }),
        });

        assert_eq!(definition.schema_name.as_str(), "cua_driver__click");
        assert_eq!(definition.provider.provider_id.as_str(), "cua-driver");
        assert_eq!(definition.provider.tool_name.as_str(), "click");
        assert_eq!(definition.provider.kind, ToolProviderKind::Mcp);
        assert_eq!(
            definition.permissions,
            vec![ToolPermission::ExternalProcess]
        );
        assert_eq!(definition.annotations.read_only, Some(false));
    }

    #[test]
    fn mcp_tool_result_parts_decode_text_images_and_structured_content() {
        let result = json!({
            "content": [
                {"type": "text", "text": "desktop screenshot"},
                {"type": "image", "mimeType": "image/png", "data": "AQID"}
            ],
            "structuredContent": {
                "screen_width": 1710
            }
        });

        let parts = mcp_tool_result_parts(&result).unwrap();

        assert_eq!(parts.len(), 3);
        assert!(
            matches!(&parts[0], UnsavedMessagePart::Text(text) if text == "desktop screenshot")
        );
        assert!(matches!(&parts[1], UnsavedMessagePart::Image(image)
            if image.mime_type == "image/png" && image.bytes == vec![1, 2, 3]));
        assert!(matches!(&parts[2], UnsavedMessagePart::Text(text)
            if text == "structuredContent: {\"screen_width\":1710}"));
        assert_eq!(
            tool_result_preview(&parts),
            "desktop screenshot\n[image: image/png, 3 bytes]\nstructuredContent: {\"screen_width\":1710}"
        );
    }

    #[test]
    fn registry_executes_only_approved_mcp_provider_ids() {
        let registry = ToolProviderRegistry::new();
        let attached_tool = AttachedTool {
            schema_name: ToolSchemaName::new("other__click"),
            description: "Click somewhere".to_string(),
            parameters: json!({"type":"object"}),
            provider: ToolProviderRef::new(
                ToolProviderId::new("other-mcp"),
                ProviderToolName::new("click"),
                ToolProviderKind::Mcp,
            ),
            permissions: vec![ToolPermission::ExternalProcess],
            annotations: ToolAnnotations::default(),
        };

        assert!(!registry.can_execute(&attached_tool));
    }

    #[test]
    fn registry_recognizes_cua_driver_as_approved_mcp_provider() {
        let registry = ToolProviderRegistry::new();
        let attached_tool = AttachedTool {
            schema_name: ToolSchemaName::new("cua_driver__click"),
            description: "Click somewhere".to_string(),
            parameters: json!({"type":"object"}),
            provider: ToolProviderRef::new(
                ToolProviderId::new("cua-driver"),
                ProviderToolName::new("click"),
                ToolProviderKind::Mcp,
            ),
            permissions: vec![ToolPermission::ExternalProcess],
            annotations: ToolAnnotations::default(),
        };

        assert!(registry.can_execute(&attached_tool));
    }

    #[test]
    fn unavailable_mcp_provider_does_not_hide_builtin_tools() {
        let registry = ToolProviderRegistry {
            built_in: BuiltInToolProvider,
            mcp_providers: vec![McpToolProvider::new(McpProviderDefinition {
                provider_id: "missing-mcp",
                schema_prefix: "missing_mcp",
                display_name: "Missing MCP",
                command: McpCommand {
                    program: "windie-missing-mcp-provider",
                    args: &[],
                },
                shutdown_command: None,
            })],
            persistent_mcp_calls: false,
        };

        let tools = registry.list_available_tools().unwrap();

        assert!(tools.iter().any(|tool| {
            tool.provider.provider_id.as_str() == "windie"
                && tool.provider.tool_name.as_str() == "run_shell"
        }));
    }
}
