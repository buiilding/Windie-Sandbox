//! Runtime tool execution.
//!
//! This module tracks pending model tool calls, applies execution policy,
//! dispatches approved calls, and persists tool results.

use std::collections::HashSet;

use crate::conversation::{ConversationId, MessageId, ToolCall, ToolCallId};
use crate::error;
use crate::plugin::PluginCatalog;
use crate::store::Store;
use crate::tool::control::{
    AttachmentFacts, ControlRequest, plan_provider_attachment, validate_mcp_membership,
};
use crate::tool::{AttachedTool, ToolExecutionResult, ToolProviderKind, ToolSchemaName};
use crate::tool::{BUILTIN_PROVIDER_ID, ToolProviderRegistry};
use anyhow::Result;

use super::RuntimeMessagePersistence;
pub(crate) use super::progression::{PendingToolCall, active_tool_execution};
use super::progression::{ToolAction, next_tool_action};
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
    plugin_catalog: Option<&PluginCatalog>,
    events: &impl RuntimeMessagePersistence,
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
    let attached_tool =
        load_attached_tool_for_call(store, conversation_id, &pending.tool_call, tools)?;
    let approval_mode = store.tool_approval_mode(conversation_id)?;
    let result = match next_tool_action(
        &pending.tool_call,
        attached_tool.as_ref(),
        attached_tool_can_execute(store, tools, attached_tool.as_ref()),
        approval_mode,
    ) {
        ToolAction::PersistDenied(result) => result,
        ToolAction::Execute => {
            execute_provider_tool_call(
                store,
                conversation_id,
                &pending,
                attached_tool.as_ref(),
                tools,
                plugin_catalog,
            )
            .await?
        }
        ToolAction::RequestApproval { .. } => {
            return Ok(AutomaticToolResolution::WaitingForApproval);
        }
    };

    let message_id = events.save_tool_result(
        store,
        conversation_id,
        &pending.result_parent_message_id,
        &result.tool_call_id,
        &result.content,
        &result.parts,
    )?;
    *head_message_id = Some(message_id.clone());

    Ok(AutomaticToolResolution::Resolved)
}

pub(crate) enum PendingToolExecution {
    Finished(ToolExecutionResult),
    Execute(AttachedTool),
}

/// Tree-wide: tool lookup ignores head, same tool set for any branch.
pub(crate) fn prepare_pending_tool_execution(
    store: &Store,
    conversation_id: &ConversationId,
    pending: &PendingToolCall,
    registry: &ToolProviderRegistry,
) -> Result<PendingToolExecution> {
    let attached_tool =
        load_attached_tool_for_call(store, conversation_id, &pending.tool_call, registry)?;
    let approval_mode = store.tool_approval_mode(conversation_id)?;

    match next_tool_action(
        &pending.tool_call,
        attached_tool.as_ref(),
        attached_tool_can_execute(store, registry, attached_tool.as_ref()),
        approval_mode,
    ) {
        ToolAction::PersistDenied(result) => Ok(PendingToolExecution::Finished(result)),
        ToolAction::Execute | ToolAction::RequestApproval { .. } => {
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
    execute_pending_tool_call_with_catalog(
        store,
        conversation_id,
        pending,
        attached_tool,
        registry,
        None,
    )
    .await
}

pub(crate) async fn execute_pending_tool_call_with_catalog(
    store: &mut Store,
    conversation_id: &ConversationId,
    pending: &PendingToolCall,
    attached_tool: &AttachedTool,
    registry: &ToolProviderRegistry,
    plugin_catalog: Option<&PluginCatalog>,
) -> Result<ToolExecutionResult> {
    if attached_tool.provider.kind == ToolProviderKind::Builtin {
        return execute_builtin_tool_call(
            store,
            conversation_id,
            pending,
            attached_tool,
            registry,
            plugin_catalog,
        )
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
    plugin_catalog: Option<&PluginCatalog>,
) -> Result<ToolExecutionResult> {
    let Some(attached_tool) = attached_tool else {
        return Err(error::invalid_request(format!(
            "Tool is not attached: {}",
            pending.tool_call.name()
        )));
    };

    execute_pending_tool_call_with_catalog(
        store,
        conversation_id,
        pending,
        attached_tool,
        registry,
        plugin_catalog,
    )
    .await
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
    plugin_catalog: Option<&PluginCatalog>,
) -> Result<ToolExecutionResult> {
    if attached_tool.provider.provider_id.as_str() != BUILTIN_PROVIDER_ID {
        return Ok(ToolExecutionResult::failure(
            pending.tool_call.id.clone(),
            pending.tool_call.name(),
            "unknown built-in tool",
        ));
    }

    let request = match ControlRequest::parse(
        attached_tool.provider.tool_name.as_str(),
        &pending.tool_call,
    ) {
        Ok(request) => request,
        Err(error) => return Ok(builtin_failure(pending, &error.to_string())),
    };
    match request {
        ControlRequest::ReadSkill {
            plugin_id,
            skill_id,
        } => {
            let Some(plugin_catalog) = plugin_catalog else {
                return Ok(builtin_failure(pending, "plugin catalog is unavailable"));
            };
            match plugin_catalog.read_skill(&plugin_id, &skill_id) {
                Ok(instructions) => Ok(ToolExecutionResult {
                    tool_call_id: pending.tool_call.id.clone(),
                    tool_name: pending.tool_call.name().to_string(),
                    content: instructions,
                    parts: Vec::new(),
                    success: true,
                }),
                Err(error) => Ok(builtin_failure(pending, &error.to_string())),
            }
        }
        ControlRequest::AttachMcp { plugin_id, mcp_id } => {
            let Some(plugin_catalog) = plugin_catalog else {
                return Ok(builtin_failure(pending, "plugin catalog is unavailable"));
            };
            let Some(plugin) = plugin_catalog.plugin_store().installed_plugin(&plugin_id)? else {
                return Ok(builtin_failure(
                    pending,
                    &format!("installed plugin does not exist: {plugin_id}"),
                ));
            };
            if let Err(error) = validate_mcp_membership(
                &plugin_id,
                &mcp_id,
                plugin
                    .manifest
                    .components
                    .iter()
                    .filter(|component| component.kind == crate::plugin::PluginComponentKind::Mcp)
                    .map(|component| component.id.as_str()),
            ) {
                return Ok(builtin_failure(pending, &error.to_string()));
            }
            match attach_provider_to_conversation(
                store,
                conversation_id,
                &crate::tool::ToolProviderId::new(mcp_id),
                registry,
            ) {
                Ok(()) => Ok(ToolExecutionResult {
                    tool_call_id: pending.tool_call.id.clone(),
                    tool_name: pending.tool_call.name().to_string(),
                    content: "MCP attached; its tools are available on the next turn".to_string(),
                    parts: Vec::new(),
                    success: true,
                }),
                Err(error) => Ok(builtin_failure(pending, &error.to_string())),
            }
        }
    }
}

fn builtin_failure(pending: &PendingToolCall, message: &str) -> ToolExecutionResult {
    ToolExecutionResult::failure(
        pending.tool_call.id.clone(),
        pending.tool_call.name(),
        message,
    )
}

/// Validates and attaches every tool from one enabled, healthy provider.
fn attach_provider_to_conversation(
    store: &mut Store,
    conversation_id: &ConversationId,
    provider_id: &crate::tool::ToolProviderId,
    registry: &ToolProviderRegistry,
) -> Result<()> {
    let existing_names = store
        .load_attached_tools(conversation_id)?
        .into_iter()
        .map(|tool| tool.schema_name)
        .collect::<HashSet<_>>();
    let catalog = store.load_provider_tool_catalog(provider_id)?;
    let new_tools = plan_provider_attachment(
        provider_id,
        AttachmentFacts {
            registered: registry.provider_manifest(provider_id).is_some(),
            enabled: store.provider_is_enabled(provider_id)?,
            unavailable: catalog.as_ref().is_some_and(|catalog| {
                catalog.status == crate::store::ProviderCatalogStatus::Unavailable
            }),
            tools: catalog.as_ref().map(|catalog| catalog.tools.as_slice()),
        },
        &existing_names,
    )?;
    store.insert_attached_tools(conversation_id, &new_tools)
}

#[cfg(test)]
pub(crate) fn store_pending_tool_result_at_head(
    store: &mut Store,
    conversation_id: &ConversationId,
    pending: &PendingToolCall,
    result: &ToolExecutionResult,
) -> Result<MessageId> {
    if result.parts.is_empty() {
        store.insert_test_runtime_tool_result(
            conversation_id,
            &pending.result_parent_message_id,
            &result.tool_call_id,
            &result.content,
        )
    } else {
        store.insert_test_runtime_tool_result_with_parts(
            conversation_id,
            &pending.result_parent_message_id,
            &result.tool_call_id,
            &result.content,
            &result.parts,
        )
    }
}
