//! Tool provider registry and dispatch.
//!
//! This module is the execution boundary for tool providers. Runtime asks this
//! registry which tools are available and asks it to execute an approved tool
//! call through the provider reference stored on the conversation attachment.
//! Approved MCP servers enter runtime through this shared registry shape.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, anyhow};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Value, json};

use crate::conversation::{ToolCall, ToolSchemaName, UnsavedImagePart, UnsavedMessagePart};
use crate::error;
use crate::image_input::validate_image_input_bytes;
use crate::mcp::{
    self, McpCommand, McpContentBlock, McpEnv, McpEnvValue, McpSessionPool, McpTool, McpToolResult,
};
use crate::paths;
use crate::run::{RunCancellation, is_runtime_cancelled};
use crate::tool::{
    AttachedTool, ProviderToolName, ToolAnnotations, ToolDefinition, ToolExecutionResult,
    ToolPermission, ToolProviderId, ToolProviderKind, ToolProviderRef,
};

const DESKTOP_COMMANDER_HOME_RELATIVE: &str = "mcp/desktop-commander";
const MCP_TOOL_RESULT_MAX_BYTES: usize = 32 * 1024 * 1024;
const TOOL_RESULT_PREVIEW_MAX_BYTES: usize = 4 * 1024;

#[derive(Debug, Clone)]
/// Registry of tool providers available to this Windie process.
///
/// The registry deliberately exposes provider-neutral operations. Runtime does
/// does not know MCP process details; it resolves the conversation's attached
/// tool to a provider reference and calls this registry.
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
        let results = std::thread::scope(|scope| {
            self.mcp_providers
                .iter()
                .map(|provider| {
                    let provider_id = provider.id().clone();
                    (
                        provider_id.clone(),
                        scope.spawn(move || self.list_provider_tools(&provider_id)),
                    )
                })
                .collect::<Vec<_>>()
                .into_iter()
                .map(|(provider_id, worker)| {
                    let result = worker
                        .join()
                        .map_err(|_| anyhow!("provider catalog task panicked"))
                        .and_then(|result| result);
                    (provider_id, result)
                })
                .collect::<Vec<_>>()
        });

        let mut tools = Vec::new();
        let mut errors = Vec::new();
        for (provider_id, result) in results {
            match result {
                Ok(provider_tools) => tools.extend(provider_tools),
                Err(error) => errors.push(format!("{provider_id}: {error:#}")),
            }
        }

        if tools.is_empty() && !errors.is_empty() {
            return Err(anyhow!(
                "failed to load provider catalogs: {}",
                errors.join("; ")
            ));
        }
        for error in errors {
            eprintln!("warning: failed to load provider catalog: {error}");
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
            ToolProviderKind::SchemaOnly => false,
        }
    }

    /// Completes provider-local setup before runtime claims a side effect.
    pub(crate) fn prepare_tool(&self, attached_tool: &AttachedTool) -> Result<()> {
        match attached_tool.provider.kind {
            ToolProviderKind::Mcp => {
                let provider = self
                    .mcp_provider(&attached_tool.provider.provider_id)
                    .ok_or_else(|| {
                        error::invalid_request(format!(
                            "unknown tool provider: {}",
                            attached_tool.provider.provider_id
                        ))
                    })?;
                provider.prepare()
            }
            ToolProviderKind::SchemaOnly => Err(error::invalid_request(format!(
                "tool has no executor: {}",
                attached_tool.schema_name
            ))),
        }
    }

    /// Executes one approved model tool call through its attached provider.
    pub async fn call_tool(
        &self,
        attached_tool: &AttachedTool,
        tool_call: &ToolCall,
        cancellation: &RunCancellation,
    ) -> Result<ToolExecutionResult> {
        match attached_tool.provider.kind {
            ToolProviderKind::Mcp => {
                let Some(provider) = self.mcp_provider(&attached_tool.provider.provider_id) else {
                    return Err(error::invalid_request(format!(
                        "unknown tool: {}",
                        tool_call.name()
                    )));
                };

                let provider = provider.clone();
                let attached_tool = attached_tool.clone();
                let tool_call = tool_call.clone();
                let session_pool = self.mcp_session_pool.clone();
                let cancellation = cancellation.clone();
                tokio::task::spawn_blocking(move || {
                    provider.call_tool(
                        &attached_tool,
                        &tool_call,
                        session_pool.as_ref(),
                        &cancellation,
                    )
                })
                .await
                .context("MCP provider task stopped")?
            }
            ToolProviderKind::SchemaOnly => Err(error::invalid_request(format!(
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
            program: "npx",
            args: &["-y", "@wonderwhy-er/desktop-commander@0.2.44"],
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
            args: &["--python", "3.11", "blender-mcp==1.6.0"],
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
    McpProviderDefinition {
        provider_id: "brightdata",
        schema_prefix: "brightdata",
        display_name: "Bright Data",
        command: McpCommand {
            program: "npx",
            args: &["-y", "@brightdata/mcp"],
            env: &[McpEnv {
                key: "API_TOKEN",
                value: McpEnvValue::UserEnv("BRIGHTDATA_API_TOKEN"),
            }],
        },
        shutdown_command: None,
        setup: None,
    },
    McpProviderDefinition {
        provider_id: "exa",
        schema_prefix: "exa",
        display_name: "Exa",
        command: McpCommand {
            program: "npx",
            args: &["-y", "exa-mcp-server@3.2.1"],
            env: &[McpEnv {
                key: "EXA_API_KEY",
                value: McpEnvValue::UserEnv("EXA_API_KEY"),
            }],
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
    fn call_tool(
        &self,
        attached_tool: &AttachedTool,
        tool_call: &ToolCall,
        session_pool: Option<&McpSessionPool>,
        cancellation: &RunCancellation,
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
        let result = match if let Some(session_pool) = session_pool {
            session_pool.call_tool_cancellable(
                self.provider_id.as_str(),
                self.command,
                self.shutdown_command,
                attached_tool.provider.tool_name.as_str(),
                arguments,
                cancellation,
            )
        } else {
            mcp::call_tool_with_shutdown_cancellable(
                self.command,
                self.shutdown_command,
                attached_tool.provider.tool_name.as_str(),
                arguments,
                cancellation,
            )
        } {
            Ok(result) => result,
            Err(error) => {
                if is_runtime_cancelled(&error) {
                    return Err(error);
                }
                return Ok(mcp_tool_call_failure_result(
                    &self.provider_id,
                    tool_call,
                    &error,
                ));
            }
        };
        let success = !result.is_error;

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
    paths::data_dir().join(DESKTOP_COMMANDER_HOME_RELATIVE)
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
fn mcp_tool_result_parts(result: &McpToolResult) -> Result<Vec<UnsavedMessagePart>> {
    let mut parts = Vec::new();
    let mut result_bytes = 0_usize;

    for block in &result.content {
        match block {
            McpContentBlock::Text(text) => {
                add_mcp_result_bytes(&mut result_bytes, text.len())?;
                parts.push(UnsavedMessagePart::Text(text.clone()));
            }
            McpContentBlock::Image { data, mime_type } => {
                let bytes = STANDARD
                    .decode(data)
                    .context("failed to decode MCP image result")?;
                validate_image_input_bytes(mime_type, &bytes)
                    .context("invalid MCP image result")?;
                add_mcp_result_bytes(&mut result_bytes, bytes.len())?;
                parts.push(UnsavedMessagePart::Image(UnsavedImagePart {
                    mime_type: mime_type.to_string(),
                    bytes,
                }));
            }
            McpContentBlock::Unsupported { kind } => {
                let text = format!("Unsupported MCP content block: {kind}");
                add_mcp_result_bytes(&mut result_bytes, text.len())?;
                parts.push(UnsavedMessagePart::Text(text));
            }
        }
    }

    if let Some(structured_content) = &result.structured_content {
        let text = format!("structuredContent: {structured_content}");
        add_mcp_result_bytes(&mut result_bytes, text.len())?;
        parts.push(UnsavedMessagePart::Text(text));
    }

    if parts.is_empty() {
        let text = result.raw_json.clone();
        add_mcp_result_bytes(&mut result_bytes, text.len())?;
        parts.push(UnsavedMessagePart::Text(text));
    }

    Ok(parts)
}

/// Accounts for decoded tool-result bytes before they enter conversation
/// storage. The JSON-RPC frame is bounded separately in `mcp.rs`; this limit
/// covers the normalized text and decoded image representation.
fn add_mcp_result_bytes(total: &mut usize, added: usize) -> Result<()> {
    *total = total
        .checked_add(added)
        .ok_or_else(|| anyhow!("MCP tool result size overflow"))?;
    if *total > MCP_TOOL_RESULT_MAX_BYTES {
        return Err(anyhow!(
            "MCP tool result exceeds {MCP_TOOL_RESULT_MAX_BYTES} bytes"
        ));
    }
    Ok(())
}

/// Builds the compact visible text stored on the tool message row.
fn tool_result_preview(parts: &[UnsavedMessagePart]) -> String {
    let mut preview = String::new();
    let mut truncated = false;

    for (index, part) in parts.iter().enumerate() {
        if index > 0 {
            truncated |= !push_preview_text(&mut preview, "\n");
        }
        let complete = match part {
            UnsavedMessagePart::Text(text) => push_preview_text(&mut preview, text),
            UnsavedMessagePart::Image(image) => push_preview_text(
                &mut preview,
                &format!("[image: {}, {} bytes]", image.mime_type, image.bytes.len()),
            ),
        };
        if !complete {
            truncated = true;
            break;
        }
    }

    if truncated {
        preview.push_str("\n[truncated]");
    }
    preview
}

fn push_preview_text(preview: &mut String, text: &str) -> bool {
    let remaining = TOOL_RESULT_PREVIEW_MAX_BYTES.saturating_sub(preview.len());
    if text.len() <= remaining {
        preview.push_str(text);
        return true;
    }

    let mut boundary = remaining.min(text.len());
    while !text.is_char_boundary(boundary) {
        boundary -= 1;
    }
    preview.push_str(&text[..boundary]);
    false
}

#[cfg(test)]
mod tests;
