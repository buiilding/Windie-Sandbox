//! Runtime tool execution.
//!
//! This module tracks pending model tool calls, applies execution policy,
//! dispatches approved calls, and persists tool results.

use std::collections::HashSet;

use anyhow::Result;
use serde_json::Value;

use crate::conversation::{ConversationId, Message, MessageId, Role, ToolCall, ToolCallId};
use crate::error;
use crate::store::Store;
use crate::tool::{
    AttachedTool, PolicyDecision, ToolExecutionResult, ToolPolicy, ToolProviderKind, ToolSchemaName,
};
use crate::tool_provider::{
    ATTACH_PROVIDER_TOOL_NAME, BUILTIN_PROVIDER_ID, LIST_PROVIDERS_TOOL_NAME, ToolProviderRegistry,
};

use super::RuntimeEventSink;
use super::turn::load_path_at_head;

pub(crate) enum AutomaticToolResolution {
    Idle,
    WaitingForApproval,
    Resolved,
}

pub(crate) async fn resolve_next_automatic_tool_call_at_head(
    store: &mut Store,
    conversation_id: &ConversationId,
    head_message_id: &mut Option<MessageId>,
    tools: &ToolProviderRegistry,
    events: &impl RuntimeEventSink,
) -> Result<AutomaticToolResolution> {
    let messages = load_path_at_head(store, conversation_id, head_message_id.as_ref())?;
    let Some(execution) = active_tool_execution(&messages) else {
        return Ok(AutomaticToolResolution::Idle);
    };
    let Some(tool_call) = execution.next_pending_tool_call().cloned() else {
        return Ok(AutomaticToolResolution::Idle);
    };

    let pending = PendingToolCall {
        result_parent_message_id: execution.result_parent_message_id,
        tool_call,
    };
    let policy = ToolPolicy;
    let attached_tool =
        load_attached_tool_for_call(store, conversation_id, &pending.tool_call, tools)?;
    let approval_mode = store.tool_approval_mode(conversation_id)?;
    let result = match policy.decide(
        &pending.tool_call,
        attached_tool.as_ref(),
        attached_tool_can_execute(store, tools, attached_tool.as_ref()),
        approval_mode,
    ) {
        PolicyDecision::Deny { reason } => ToolExecutionResult::failure(
            pending.tool_call.id.clone(),
            pending.tool_call.name(),
            reason,
        ),
        PolicyDecision::Allow => {
            execute_provider_tool_call(
                store,
                conversation_id,
                &pending,
                attached_tool.as_ref(),
                tools,
            )
            .await?
        }
        PolicyDecision::Ask { .. } => return Ok(AutomaticToolResolution::WaitingForApproval),
    };

    let message_id = store_pending_tool_result_at_head(store, conversation_id, &pending, &result)?;
    *head_message_id = Some(message_id.clone());
    events.tool_result_saved(&message_id);

    Ok(AutomaticToolResolution::Resolved)
}

pub(crate) struct PendingToolCall {
    pub(crate) result_parent_message_id: MessageId,
    pub(crate) tool_call: ToolCall,
}

pub(crate) enum PendingToolExecution {
    Finished(ToolExecutionResult),
    Execute(AttachedTool),
}

pub(crate) struct ActiveToolExecution {
    pub(crate) assistant_message_id: MessageId,
    pub(crate) result_parent_message_id: MessageId,
    requested_tool_calls: Vec<ToolCall>,
    resolved_tool_call_ids: HashSet<String>,
}

impl ActiveToolExecution {
    pub(crate) fn next_pending_tool_call(&self) -> Option<&ToolCall> {
        self.requested_tool_calls
            .iter()
            .find(|tool_call| !self.resolved_tool_call_ids.contains(tool_call.id.as_str()))
    }

    fn has_requested_tool_call(&self, tool_call_id: &ToolCallId) -> bool {
        self.requested_tool_calls
            .iter()
            .any(|tool_call| &tool_call.id == tool_call_id)
    }

    fn has_tool_result(&self, tool_call_id: &ToolCallId) -> bool {
        self.resolved_tool_call_ids.contains(tool_call_id.as_str())
    }
}

pub(crate) fn active_tool_execution(messages: &[Message]) -> Option<ActiveToolExecution> {
    let (assistant_index, assistant) = messages.iter().enumerate().rev().find(|(_, message)| {
        message.role == Role::Assistant
            && message
                .metadata
                .as_ref()
                .is_some_and(|metadata| !metadata.tool_calls.is_empty())
    })?;
    let assistant_message_id = assistant.id.as_ref()?.clone();
    let requested_tool_calls = assistant.metadata.as_ref()?.tool_calls.clone();
    let requested_tool_call_ids = requested_tool_calls
        .iter()
        .map(|tool_call| tool_call.id.as_str().to_string())
        .collect::<HashSet<_>>();
    let mut result_parent_message_id = assistant_message_id.clone();
    let mut resolved_tool_call_ids = HashSet::new();

    for message in &messages[assistant_index + 1..] {
        if message.role != Role::Tool {
            break;
        }
        let Some(message_id) = message.id.as_ref() else {
            continue;
        };
        let Some(tool_call_id) = message
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.tool_call_id.as_ref())
        else {
            continue;
        };
        if requested_tool_call_ids.contains(tool_call_id.as_str()) {
            resolved_tool_call_ids.insert(tool_call_id.as_str().to_string());
            result_parent_message_id = message_id.clone();
        }
    }

    Some(ActiveToolExecution {
        assistant_message_id,
        result_parent_message_id,
        requested_tool_calls,
        resolved_tool_call_ids,
    })
}

/// Tree-wide: tool lookup ignores head, same tool set for any branch.
pub(crate) fn prepare_pending_tool_execution(
    store: &Store,
    conversation_id: &ConversationId,
    pending: &PendingToolCall,
    registry: &ToolProviderRegistry,
) -> Result<PendingToolExecution> {
    let policy = ToolPolicy;
    let attached_tool =
        load_attached_tool_for_call(store, conversation_id, &pending.tool_call, registry)?;
    let approval_mode = store.tool_approval_mode(conversation_id)?;

    match policy.decide(
        &pending.tool_call,
        attached_tool.as_ref(),
        attached_tool_can_execute(store, registry, attached_tool.as_ref()),
        approval_mode,
    ) {
        PolicyDecision::Deny { reason } => Ok(PendingToolExecution::Finished(
            ToolExecutionResult::failure(
                pending.tool_call.id.clone(),
                pending.tool_call.name(),
                reason,
            ),
        )),
        PolicyDecision::Allow | PolicyDecision::Ask { .. } => {
            let Some(attached_tool) = attached_tool else {
                return Err(error::invalid_request(format!(
                    "Tool is not attached: {}",
                    pending.tool_call.name()
                )));
            };
            Ok(PendingToolExecution::Execute(attached_tool))
        }
    }
}

pub(crate) async fn execute_pending_tool_call(
    store: &mut Store,
    conversation_id: &ConversationId,
    pending: &PendingToolCall,
    attached_tool: &AttachedTool,
    registry: &ToolProviderRegistry,
) -> Result<ToolExecutionResult> {
    if attached_tool.provider.kind == ToolProviderKind::Builtin {
        return execute_builtin_tool_call(store, conversation_id, pending, attached_tool, registry)
            .await;
    }

    registry.call_tool(attached_tool, &pending.tool_call).await
}

pub(crate) async fn execute_provider_tool_call(
    store: &mut Store,
    conversation_id: &ConversationId,
    pending: &PendingToolCall,
    attached_tool: Option<&AttachedTool>,
    registry: &ToolProviderRegistry,
) -> Result<ToolExecutionResult> {
    let Some(attached_tool) = attached_tool else {
        return Err(error::invalid_request(format!(
            "Tool is not attached: {}",
            pending.tool_call.name()
        )));
    };

    execute_pending_tool_call(store, conversation_id, pending, attached_tool, registry).await
}

pub(crate) fn deny_pending_tool_call(pending: &PendingToolCall) -> ToolExecutionResult {
    ToolExecutionResult::failure(
        pending.tool_call.id.clone(),
        pending.tool_call.name(),
        "tool call rejected by user",
    )
}

pub(crate) fn load_pending_tool_call_at_head(
    store: &Store,
    conversation_id: &ConversationId,
    head_message_id: Option<&MessageId>,
    tool_call_id: &ToolCallId,
) -> Result<PendingToolCall> {
    let messages = load_path_at_head(store, conversation_id, head_message_id)?;
    let Some(execution) = active_tool_execution(&messages) else {
        return Err(error::not_found(format!(
            "pending tool call does not exist: {tool_call_id}"
        )));
    };
    if execution.has_tool_result(tool_call_id) {
        return Err(error::invalid_request(format!(
            "tool call already has a result: {tool_call_id}"
        )));
    }
    let Some(next_tool_call) = execution.next_pending_tool_call().cloned() else {
        return Err(error::not_found(format!(
            "pending tool call does not exist: {tool_call_id}"
        )));
    };
    if next_tool_call.id != *tool_call_id {
        if execution.has_requested_tool_call(tool_call_id) {
            return Err(error::invalid_request(format!(
                "tool call must be resolved after previous tool call: {}",
                next_tool_call.id
            )));
        }

        return Err(error::not_found(format!(
            "pending tool call does not exist: {tool_call_id}"
        )));
    }

    Ok(PendingToolCall {
        result_parent_message_id: execution.result_parent_message_id,
        tool_call: next_tool_call,
    })
}

pub(crate) fn load_attached_tool_for_call(
    store: &Store,
    conversation_id: &ConversationId,
    tool_call: &ToolCall,
    registry: &ToolProviderRegistry,
) -> Result<Option<AttachedTool>> {
    let schema_name = ToolSchemaName::new(tool_call.name());
    if let Some(attached_tool) = store.load_attached_tool(conversation_id, &schema_name)? {
        return Ok(Some(attached_tool));
    }

    Ok(registry
        .builtin_tool(&schema_name)
        .map(|definition| definition.attached_tool()))
}

pub(crate) fn attached_tool_can_execute(
    store: &Store,
    registry: &ToolProviderRegistry,
    attached_tool: Option<&AttachedTool>,
) -> bool {
    attached_tool.is_some_and(|attached_tool| {
        if attached_tool.provider.kind == ToolProviderKind::Builtin {
            return registry.can_execute(attached_tool);
        }

        store
            .provider_is_enabled(&attached_tool.provider.provider_id)
            .unwrap_or(false)
            && registry.can_execute(attached_tool)
    })
}

/// Executes one Windie-owned control tool and returns its compact model result.
async fn execute_builtin_tool_call(
    store: &mut Store,
    conversation_id: &ConversationId,
    pending: &PendingToolCall,
    attached_tool: &AttachedTool,
    registry: &ToolProviderRegistry,
) -> Result<ToolExecutionResult> {
    if attached_tool.provider.provider_id.as_str() != BUILTIN_PROVIDER_ID {
        return Ok(ToolExecutionResult::failure(
            pending.tool_call.id.clone(),
            pending.tool_call.name(),
            "unknown built-in tool",
        ));
    }

    match attached_tool.provider.tool_name.as_str() {
        LIST_PROVIDERS_TOOL_NAME => Ok(ToolExecutionResult {
            tool_call_id: pending.tool_call.id.clone(),
            tool_name: pending.tool_call.name().to_string(),
            content: list_attachable_providers(
                registry,
                enabled_provider_manifests(store, registry)?,
            )
            .await?,
            parts: Vec::new(),
            success: true,
        }),
        ATTACH_PROVIDER_TOOL_NAME => {
            let arguments = match serde_json::from_str::<Value>(pending.tool_call.arguments()) {
                Ok(arguments) => arguments,
                Err(error) => {
                    return Ok(ToolExecutionResult::failure(
                        pending.tool_call.id.clone(),
                        pending.tool_call.name(),
                        format!("invalid tool arguments: {error}"),
                    ));
                }
            };
            let Some(provider_id) = arguments.get("provider_id").and_then(Value::as_str) else {
                return Ok(ToolExecutionResult::failure(
                    pending.tool_call.id.clone(),
                    pending.tool_call.name(),
                    "provider_id is required",
                ));
            };

            let attachment = attach_provider_to_conversation(
                store,
                conversation_id,
                &crate::tool::ToolProviderId::new(provider_id),
                registry,
            )
            .await;

            let Err(error) = attachment else {
                return Ok(ToolExecutionResult {
                    tool_call_id: pending.tool_call.id.clone(),
                    tool_name: pending.tool_call.name().to_string(),
                    content: "provider attached".to_string(),
                    parts: Vec::new(),
                    success: true,
                });
            };

            Ok(ToolExecutionResult::failure(
                pending.tool_call.id.clone(),
                pending.tool_call.name(),
                error.to_string(),
            ))
        }
        _ => Ok(ToolExecutionResult::failure(
            pending.tool_call.id.clone(),
            pending.tool_call.name(),
            "unknown built-in tool",
        )),
    }
}

/// Formats the attachable provider list exactly as model-facing plain text.
async fn list_attachable_providers(
    registry: &ToolProviderRegistry,
    manifests: Vec<crate::tool_provider::ProviderManifest>,
) -> Result<String> {
    let mut lines = vec!["provider_id, description".to_string()];
    for manifest in manifests {
        let Some(status) = registry.provider_status_async(&manifest.provider_id).await else {
            continue;
        };
        if status.available {
            lines.push(format!(
                "{}, {}",
                manifest.provider_id.as_str(),
                manifest.description
            ));
        }
    }

    Ok(lines.join("\n"))
}

/// Loads the enabled provider manifests before entering the async catalog
/// lookup path. SQLite connections are intentionally not held across awaits.
fn enabled_provider_manifests(
    store: &Store,
    registry: &ToolProviderRegistry,
) -> Result<Vec<crate::tool_provider::ProviderManifest>> {
    let mut manifests = Vec::new();
    for manifest in registry.provider_manifests() {
        if store.provider_is_enabled(&manifest.provider_id)? {
            manifests.push(manifest);
        }
    }
    Ok(manifests)
}

/// Validates and attaches every tool from one enabled, healthy provider.
async fn attach_provider_to_conversation(
    store: &mut Store,
    conversation_id: &ConversationId,
    provider_id: &crate::tool::ToolProviderId,
    registry: &ToolProviderRegistry,
) -> Result<()> {
    if registry.provider_manifest(provider_id).is_none() {
        return Err(error::not_found(format!(
            "provider does not exist: {provider_id}"
        )));
    }
    if !store.provider_is_enabled(provider_id)? {
        return Err(error::invalid_request(format!(
            "provider is not installed, enabled, and healthy: {provider_id}"
        )));
    }
    let Some(status) = registry.provider_status_async(provider_id).await else {
        return Err(error::not_found(format!(
            "provider does not exist: {provider_id}"
        )));
    };
    if !status.available {
        return Err(error::invalid_request(format!(
            "provider is not healthy: {provider_id}"
        )));
    }

    let existing_names = store
        .load_attached_tools(conversation_id)?
        .into_iter()
        .map(|tool| tool.schema_name)
        .collect::<HashSet<_>>();
    let new_tools = registry
        .list_provider_tools_async(provider_id)
        .await?
        .into_iter()
        .filter(|tool| !existing_names.contains(&tool.schema_name))
        .map(|tool| tool.attached_tool())
        .collect::<Vec<_>>();
    store.insert_attached_tools(conversation_id, &new_tools)
}

pub(crate) fn store_pending_tool_result_at_head(
    store: &mut Store,
    conversation_id: &ConversationId,
    pending: &PendingToolCall,
    result: &ToolExecutionResult,
) -> Result<MessageId> {
    if result.parts.is_empty() {
        store.insert_run_tool_result_message(
            conversation_id,
            &pending.result_parent_message_id,
            &result.tool_call_id,
            &result.content,
        )
    } else {
        store.insert_run_tool_result_message_with_parts(
            conversation_id,
            &pending.result_parent_message_id,
            &result.tool_call_id,
            &result.content,
            &result.parts,
        )
    }
}
