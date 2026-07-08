//! Tool provider registry and dispatch.
//!
//! This module is the execution boundary for tool providers. Runtime asks this
//! registry which tools are available and asks it to execute an approved tool
//! call through the provider reference stored on the conversation attachment.
//! Approved MCP servers and future plugins should enter runtime through this
//! same registry shape.

use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, anyhow};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Value, json};

use crate::conversation::{ToolCall, ToolSchemaName, UnsavedImagePart, UnsavedMessagePart};
use crate::error;
use crate::mcp::{self, McpCommand, McpEnv, McpEnvValue, McpSessionPool, McpTool};
use crate::tool::{
    AttachedTool, ProviderToolName, ToolAnnotations, ToolDefinition, ToolExecutionResult,
    ToolPermission, ToolProviderId, ToolProviderKind, ToolProviderRef,
};

const DESKTOP_COMMANDER_HOME_RELATIVE: &str = "mcp/desktop-commander";

#[derive(Debug, Clone)]
/// Registry of tool providers available to this Windie process.
///
/// The registry deliberately exposes provider-neutral operations. Runtime does
/// not branch on shell, MCP, or plugin details; it resolves the conversation's
/// attached tool to a provider reference and calls this registry.
pub struct ToolProviderRegistry {
    mcp_providers: Vec<McpToolProvider>,
    mcp_session_pool: Option<McpSessionPool>,
    catalog_cache: Arc<Mutex<HashMap<ToolProviderId, Vec<ToolDefinition>>>>,
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
            mcp_session_pool: Some(McpSessionPool::new()),
            ..Self::default()
        }
    }

    /// Lists every provider tool that clients may attach to conversations.
    ///
    /// Availability does not grant model access. Clients still need to attach a
    /// returned definition before the model sees the function schema. Provider
    /// catalogs loaded here are cached for later attachment requests in the same
    /// process.
    pub fn list_available_tools(&self) -> Result<Vec<ToolDefinition>> {
        let mut tools = Vec::new();
        for provider in &self.mcp_providers {
            if let Ok(provider_tools) = self.list_provider_tools(provider.id()) {
                tools.extend(provider_tools);
            }
        }

        Ok(tools)
    }

    /// Lists available tools for one provider ID.
    ///
    /// MCP provider catalogs can require starting a provider process for
    /// `tools/list`. The API server keeps one registry for the process, so this
    /// method caches successful catalog loads and lets later attachment
    /// resolution reuse the backend-owned schema copy.
    pub fn list_provider_tools(&self, provider_id: &ToolProviderId) -> Result<Vec<ToolDefinition>> {
        if let Some(tools) = self.cached_provider_tools(provider_id)? {
            return Ok(tools);
        }
        if let Some(provider) = self.mcp_provider(provider_id) {
            let tools = provider.list_tools()?;
            self.cache_provider_tools(provider_id, &tools)?;
            return Ok(tools);
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
            ToolProviderKind::Mcp => {
                let Some(provider) = self.mcp_provider(&attached_tool.provider.provider_id) else {
                    return Err(error::invalid_request(format!(
                        "unknown tool: {}",
                        tool_call.name()
                    )));
                };

                provider
                    .call_tool(attached_tool, tool_call, self.mcp_session_pool.as_ref())
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

    /// Returns a cached provider catalog when this process has already loaded
    /// one.
    fn cached_provider_tools(
        &self,
        provider_id: &ToolProviderId,
    ) -> Result<Option<Vec<ToolDefinition>>> {
        let cache = self
            .catalog_cache
            .lock()
            .map_err(|_| anyhow!("tool provider catalog cache lock was poisoned"))?;

        Ok(cache.get(provider_id).cloned())
    }

    /// Stores one backend-owned provider catalog for reuse by later operations.
    fn cache_provider_tools(
        &self,
        provider_id: &ToolProviderId,
        tools: &[ToolDefinition],
    ) -> Result<()> {
        let mut cache = self
            .catalog_cache
            .lock()
            .map_err(|_| anyhow!("tool provider catalog cache lock was poisoned"))?;
        cache.insert(provider_id.clone(), tools.to_vec());

        Ok(())
    }

    /// Builds a test registry with one fake MCP provider and an already-loaded
    /// catalog.
    ///
    /// Runtime tests use this to exercise provider dispatch without depending
    /// on user-installed MCP binaries.
    #[cfg(test)]
    pub(crate) fn with_test_mcp_provider(
        provider_id: &'static str,
        schema_prefix: &'static str,
        display_name: &'static str,
        command: McpCommand,
        tools: Vec<ToolDefinition>,
    ) -> Self {
        let provider_id_value = ToolProviderId::new(provider_id);
        let catalog_cache = Arc::new(Mutex::new(HashMap::from([(provider_id_value, tools)])));

        Self {
            mcp_providers: vec![McpToolProvider::new(McpProviderDefinition {
                provider_id,
                schema_prefix,
                display_name,
                command,
                shutdown_command: None,
                setup: None,
            })],
            mcp_session_pool: None,
            catalog_cache,
        }
    }
}

impl Default for ToolProviderRegistry {
    fn default() -> Self {
        Self {
            mcp_providers: APPROVED_MCP_PROVIDERS
                .iter()
                .copied()
                .map(McpToolProvider::new)
                .collect(),
            mcp_session_pool: None,
            catalog_cache: Arc::new(Mutex::new(HashMap::new())),
        }
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
    setup: Option<McpProviderSetup>,
}

#[derive(Debug, Clone, Copy)]
/// Provider-specific setup Windie runs before starting an MCP process.
enum McpProviderSetup {
    DesktopCommanderConfig,
}

/// MCP providers Windie is willing to start and execute.
///
/// This is a code-owned allowlist, not user configuration. Provider
/// availability still does not grant model access; conversations must attach
/// individual tools before their schemas are sent to the model.
const APPROVED_MCP_PROVIDERS: &[McpProviderDefinition] = &[
    McpProviderDefinition {
        provider_id: "cua-driver",
        schema_prefix: "cua_driver",
        display_name: "CUA Driver",
        command: McpCommand {
            program: "cua-driver",
            args: &["mcp"],
            env: &[],
        },
        shutdown_command: Some(McpCommand {
            program: "cua-driver",
            args: &["stop"],
            env: &[],
        }),
        setup: None,
    },
    McpProviderDefinition {
        provider_id: "desktop-commander",
        schema_prefix: "desktop_commander",
        display_name: "Desktop Commander",
        command: McpCommand {
            program: "desktop-commander",
            args: &[],
            env: &[McpEnv {
                key: "HOME",
                value: McpEnvValue::WindieDataDir(DESKTOP_COMMANDER_HOME_RELATIVE),
            }],
        },
        shutdown_command: None,
        setup: Some(McpProviderSetup::DesktopCommanderConfig),
    },
    McpProviderDefinition {
        provider_id: "blender-mcp",
        schema_prefix: "blender_mcp",
        display_name: "Blender MCP",
        command: McpCommand {
            program: "uvx",
            args: &["blender-mcp"],
            env: &[
                McpEnv {
                    key: "DISABLE_TELEMETRY",
                    value: McpEnvValue::Literal("true"),
                },
                McpEnv {
                    key: "BLENDER_HOST",
                    value: McpEnvValue::Literal("localhost"),
                },
                McpEnv {
                    key: "BLENDER_PORT",
                    value: McpEnvValue::Literal("9876"),
                },
            ],
        },
        shutdown_command: None,
        setup: None,
    },
];

#[derive(Debug, Clone)]
/// Provider for an approved MCP stdio server.
pub struct McpToolProvider {
    provider_id: ToolProviderId,
    schema_prefix: &'static str,
    display_name: &'static str,
    command: McpCommand,
    shutdown_command: Option<McpCommand>,
    setup: Option<McpProviderSetup>,
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
            setup: definition.setup,
        }
    }

    /// Returns the stable provider ID used by attachments and dispatch.
    fn id(&self) -> &ToolProviderId {
        &self.provider_id
    }

    /// Lists tools from the MCP server and maps them into Windie definitions.
    pub fn list_tools(&self) -> Result<Vec<ToolDefinition>> {
        self.prepare()?;
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
        session_pool: Option<&McpSessionPool>,
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
        self.prepare()?;
        let result = match if let Some(session_pool) = session_pool {
            session_pool.call_tool(
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
        } {
            Ok(result) => result,
            Err(error) => {
                return Ok(mcp_tool_call_failure_result(
                    &self.provider_id,
                    tool_call,
                    &error,
                ));
            }
        };
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

    /// Runs provider-specific setup before Windie starts the MCP process.
    fn prepare(&self) -> Result<()> {
        match self.setup {
            Some(McpProviderSetup::DesktopCommanderConfig) => write_desktop_commander_config(),
            None => Ok(()),
        }
    }
}

/// Converts an MCP `tools/call` operation failure into a model-facing tool
/// result.
///
/// At this point policy has already approved the model's tool call. Returning a
/// failed result lets runtime persist a linked `role: tool` message so the next
/// model turn can observe the failure instead of losing the tool-call contract
/// to an outer operation error.
fn mcp_tool_call_failure_result(
    provider_id: &ToolProviderId,
    tool_call: &ToolCall,
    error: &anyhow::Error,
) -> ToolExecutionResult {
    let content = if let Some(timeout) = mcp::request_timeout_from_error(error) {
        json!({
            "error": "MCP provider timed out",
            "detail": timeout.to_string(),
            "provider": timeout.provider.as_str(),
            "method": timeout.method.as_str(),
            "timeout_ms": timeout.timeout_ms(),
            "timeout_seconds": timeout.timeout_seconds()
        })
    } else {
        json!({
            "error": "MCP provider tool call failed",
            "detail": error.to_string(),
            "provider": provider_id.as_str(),
            "method": "tools/call"
        })
    };

    ToolExecutionResult {
        tool_call_id: tool_call.id.clone(),
        tool_name: tool_call.name().to_string(),
        content: content.to_string(),
        parts: Vec::new(),
        success: false,
    }
}

/// Writes Windie's isolated Desktop Commander config.
///
/// Desktop Commander reads config from `$HOME/.claude-server-commander`, so
/// Windie starts the process with a provider-specific HOME and keeps this
/// config separate from any user-level Desktop Commander install.
fn write_desktop_commander_config() -> Result<()> {
    let config_path = desktop_commander_home()
        .join(".claude-server-commander")
        .join("config.json");
    let config_dir = config_path
        .parent()
        .ok_or_else(|| anyhow!("Desktop Commander config path has no parent"))?;
    fs::create_dir_all(config_dir).with_context(|| {
        format!(
            "failed to create Desktop Commander config directory: {}",
            config_dir.display()
        )
    })?;

    let config = json!({
        "blockedCommands": desktop_commander_blocked_commands(),
        "allowedDirectories": [],
        "telemetryEnabled": false,
        "fileWriteLineLimit": 50,
        "fileReadLineLimit": 1000,
        "pendingWelcomeOnboarding": false
    });
    fs::write(&config_path, serde_json::to_vec_pretty(&config)?).with_context(|| {
        format!(
            "failed to write Desktop Commander config: {}",
            config_path.display()
        )
    })
}

/// Returns the HOME directory Windie assigns to Desktop Commander.
fn desktop_commander_home() -> PathBuf {
    windie_data_dir().join(DESKTOP_COMMANDER_HOME_RELATIVE)
}

/// Returns Windie's per-user data directory.
fn windie_data_dir() -> PathBuf {
    env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".windie")
}

/// Keeps Desktop Commander's default high-risk shell command blocklist.
fn desktop_commander_blocked_commands() -> Vec<&'static str> {
    vec![
        "mkfs", "format", "mount", "umount", "fdisk", "dd", "parted", "diskpart", "sudo", "su",
        "passwd", "adduser", "useradd", "usermod", "groupadd", "chsh", "visudo", "shutdown",
        "reboot", "halt", "poweroff", "init", "iptables", "firewall", "netsh", "sfc", "bcdedit",
        "reg", "net", "sc", "runas", "cipher", "takeown",
    ]
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

    if let Some(structured_content) = result.get("structuredContent")
        && !structured_content.is_null()
    {
        parts.push(UnsavedMessagePart::Text(format!(
            "structuredContent: {structured_content}"
        )));
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

    fn approved_desktop_commander_provider() -> McpToolProvider {
        let definition = APPROVED_MCP_PROVIDERS
            .iter()
            .copied()
            .find(|definition| definition.provider_id == "desktop-commander")
            .unwrap();
        McpToolProvider::new(definition)
    }

    fn approved_blender_mcp_provider() -> McpToolProvider {
        let definition = APPROVED_MCP_PROVIDERS
            .iter()
            .copied()
            .find(|definition| definition.provider_id == "blender-mcp")
            .unwrap();
        McpToolProvider::new(definition)
    }

    fn test_cache() -> Arc<Mutex<HashMap<ToolProviderId, Vec<ToolDefinition>>>> {
        Arc::new(Mutex::new(HashMap::new()))
    }

    fn cached_test_tool(provider_id: &str, tool_name: &str) -> ToolDefinition {
        ToolDefinition {
            schema_name: ToolSchemaName::new(format!("{provider_id}__{tool_name}")),
            display_name: tool_name.to_string(),
            description: format!("{tool_name} description"),
            parameters: json!({"type":"object"}),
            provider: ToolProviderRef::new(
                ToolProviderId::new(provider_id),
                ProviderToolName::new(tool_name),
                ToolProviderKind::Mcp,
            ),
            permissions: vec![ToolPermission::ExternalProcess],
            annotations: ToolAnnotations::default(),
        }
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
    fn desktop_commander_mcp_tools_map_to_provider_backed_definitions() {
        let provider = approved_desktop_commander_provider();
        let definition = provider.definition_from_mcp_tool(McpTool {
            name: "read_file".to_string(),
            description: "Read a file".to_string(),
            input_schema: json!({"type":"object"}),
            annotations: Some(mcp::McpToolAnnotations {
                read_only_hint: Some(true),
            }),
        });

        assert_eq!(
            definition.schema_name.as_str(),
            "desktop_commander__read_file"
        );
        assert_eq!(
            definition.provider.provider_id.as_str(),
            "desktop-commander"
        );
        assert_eq!(definition.provider.tool_name.as_str(), "read_file");
        assert_eq!(definition.provider.kind, ToolProviderKind::Mcp);
        assert_eq!(
            definition.permissions,
            vec![ToolPermission::ExternalProcess]
        );
        assert_eq!(definition.annotations.read_only, Some(true));
    }

    #[test]
    fn blender_mcp_tools_map_to_provider_backed_definitions() {
        let provider = approved_blender_mcp_provider();
        let definition = provider.definition_from_mcp_tool(McpTool {
            name: "get_scene_info".to_string(),
            description: "Get scene info".to_string(),
            input_schema: json!({"type":"object"}),
            annotations: Some(mcp::McpToolAnnotations {
                read_only_hint: Some(true),
            }),
        });

        assert_eq!(
            definition.schema_name.as_str(),
            "blender_mcp__get_scene_info"
        );
        assert_eq!(definition.provider.provider_id.as_str(), "blender-mcp");
        assert_eq!(definition.provider.tool_name.as_str(), "get_scene_info");
        assert_eq!(definition.provider.kind, ToolProviderKind::Mcp);
        assert_eq!(
            definition.permissions,
            vec![ToolPermission::ExternalProcess]
        );
        assert_eq!(definition.annotations.read_only, Some(true));
    }

    #[test]
    fn desktop_commander_config_allows_every_directory() {
        let config = json!({
            "blockedCommands": desktop_commander_blocked_commands(),
            "allowedDirectories": [],
            "telemetryEnabled": false,
            "fileWriteLineLimit": 50,
            "fileReadLineLimit": 1000,
            "pendingWelcomeOnboarding": false
        });

        assert_eq!(config["allowedDirectories"].as_array().unwrap().len(), 0);
        assert_eq!(config["telemetryEnabled"], false);
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
    fn mcp_tool_call_timeout_becomes_failed_tool_result() {
        let error: anyhow::Error = mcp::McpRequestTimeout::new(
            "desktop-commander",
            "tools/call",
            std::time::Duration::from_secs(300),
        )
        .into();
        let tool_call = ToolCall::function("call_123", "desktop_commander__read_file", "{}");

        let result = mcp_tool_call_failure_result(
            &ToolProviderId::new("desktop-commander"),
            &tool_call,
            &error,
        );
        let content = serde_json::from_str::<Value>(&result.content).unwrap();

        assert!(!result.success);
        assert_eq!(result.tool_call_id.as_str(), "call_123");
        assert_eq!(result.tool_name, "desktop_commander__read_file");
        assert_eq!(content["error"], "MCP provider timed out");
        assert_eq!(content["provider"], "desktop-commander");
        assert_eq!(content["method"], "tools/call");
        assert_eq!(content["timeout_ms"], 300_000);
        assert_eq!(content["timeout_seconds"], 300);
    }

    #[test]
    fn mcp_tool_call_process_error_becomes_failed_tool_result() {
        let error = anyhow!("provider exited early");
        let tool_call = ToolCall::function("call_123", "desktop_commander__read_file", "{}");

        let result = mcp_tool_call_failure_result(
            &ToolProviderId::new("desktop-commander"),
            &tool_call,
            &error,
        );
        let content = serde_json::from_str::<Value>(&result.content).unwrap();

        assert!(!result.success);
        assert_eq!(content["error"], "MCP provider tool call failed");
        assert_eq!(content["detail"], "provider exited early");
        assert_eq!(content["provider"], "desktop-commander");
        assert_eq!(content["method"], "tools/call");
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
    fn registry_recognizes_blender_mcp_as_approved_provider() {
        let registry = ToolProviderRegistry::new();
        let attached_tool = AttachedTool {
            schema_name: ToolSchemaName::new("blender_mcp__get_scene_info"),
            description: "Get scene info".to_string(),
            parameters: json!({"type":"object"}),
            provider: ToolProviderRef::new(
                ToolProviderId::new("blender-mcp"),
                ProviderToolName::new("get_scene_info"),
                ToolProviderKind::Mcp,
            ),
            permissions: vec![ToolPermission::ExternalProcess],
            annotations: ToolAnnotations::default(),
        };

        assert!(registry.can_execute(&attached_tool));
    }

    #[test]
    fn registry_finds_tools_from_cached_provider_catalog() {
        let provider_id = ToolProviderId::new("missing-mcp");
        let tool = cached_test_tool(provider_id.as_str(), "cached_tool");
        let catalog_cache = test_cache();
        catalog_cache
            .lock()
            .unwrap()
            .insert(provider_id.clone(), vec![tool.clone()]);
        let registry = ToolProviderRegistry {
            mcp_providers: vec![McpToolProvider::new(McpProviderDefinition {
                provider_id: "missing-mcp",
                schema_prefix: "missing_mcp",
                display_name: "Missing MCP",
                command: McpCommand {
                    program: "windie-missing-mcp-provider",
                    args: &[],
                    env: &[],
                },
                shutdown_command: None,
                setup: None,
            })],
            mcp_session_pool: None,
            catalog_cache,
        };

        let found = registry
            .find_tool(&provider_id, &ProviderToolName::new("cached_tool"))
            .unwrap();

        assert_eq!(found, Some(tool));
    }

    #[test]
    fn unavailable_mcp_provider_does_not_hide_other_provider_tools() {
        let available_provider_id = ToolProviderId::new("available-mcp");
        let available_tool = cached_test_tool(available_provider_id.as_str(), "cached_tool");
        let catalog_cache = test_cache();
        catalog_cache
            .lock()
            .unwrap()
            .insert(available_provider_id, vec![available_tool.clone()]);
        let registry = ToolProviderRegistry {
            mcp_providers: vec![
                McpToolProvider::new(McpProviderDefinition {
                    provider_id: "available-mcp",
                    schema_prefix: "available_mcp",
                    display_name: "Available MCP",
                    command: McpCommand {
                        program: "windie-missing-mcp-provider",
                        args: &[],
                        env: &[],
                    },
                    shutdown_command: None,
                    setup: None,
                }),
                McpToolProvider::new(McpProviderDefinition {
                    provider_id: "missing-mcp",
                    schema_prefix: "missing_mcp",
                    display_name: "Missing MCP",
                    command: McpCommand {
                        program: "windie-missing-mcp-provider",
                        args: &[],
                        env: &[],
                    },
                    shutdown_command: None,
                    setup: None,
                }),
            ],
            mcp_session_pool: None,
            catalog_cache,
        };

        let tools = registry.list_available_tools().unwrap();

        assert_eq!(tools, vec![available_tool]);
    }
}
