//! Terminal adapters for provider-tool and conversation tool-schema commands.

use anyhow::Result;

use crate::conversation::ConversationId;
use crate::operation;
use crate::output::TerminalOutput;
use crate::store::Store;
use crate::tool::{ProviderToolName, ToolProviderId, ToolSchema, ToolSchemaName};

/// Lists provider tools without mutating any conversation.
pub(crate) fn list_tools(provider_id: Option<ToolProviderId>) -> Result<()> {
    let tools = provider_id
        .as_ref()
        .map(operation::available_provider_tools)
        .unwrap_or_else(operation::available_tools)?;

    TerminalOutput.available_tools(&tools);
    Ok(())
}

/// Attaches one provider tool to a conversation.
pub(crate) fn attach_tool(
    conversation_id: ConversationId,
    provider_id: ToolProviderId,
    tool_name: ProviderToolName,
) -> Result<()> {
    let mut store = Store::open()?;
    let schema_name =
        operation::attach_tool(&mut store, &conversation_id, &provider_id, &tool_name)?;

    TerminalOutput.inserted_tool_schema(&schema_name);
    Ok(())
}

/// Inserts one root-scoped tool schema.
pub(crate) fn insert_tool_schema(
    conversation_id: ConversationId,
    tool_schema: &ToolSchema,
) -> Result<()> {
    let mut store = Store::open()?;
    operation::insert_tool_schema(&mut store, &conversation_id, tool_schema)?;

    TerminalOutput.inserted_tool_schema(&tool_schema.name);
    Ok(())
}

/// Updates one root-scoped tool schema.
pub(crate) fn update_tool_schema(
    conversation_id: ConversationId,
    current_name: ToolSchemaName,
    tool_schema: &ToolSchema,
) -> Result<()> {
    let mut store = Store::open()?;
    operation::update_tool_schema(&mut store, &conversation_id, &current_name, tool_schema)?;

    TerminalOutput.updated_tool_schema(&tool_schema.name);
    Ok(())
}

/// Removes one root-scoped tool schema.
pub(crate) fn remove_tool_schema(
    conversation_id: ConversationId,
    name: ToolSchemaName,
) -> Result<()> {
    let mut store = Store::open()?;
    operation::remove_tool_schema(&mut store, &conversation_id, &name)?;

    TerminalOutput.removed_tool_schema(&name);
    Ok(())
}

/// Detaches one provider-backed tool schema from a conversation.
pub(crate) fn detach_tool(
    conversation_id: ConversationId,
    schema_name: ToolSchemaName,
) -> Result<()> {
    let mut store = Store::open()?;
    operation::detach_tool(&mut store, &conversation_id, &schema_name)?;

    TerminalOutput.removed_tool_schema(&schema_name);
    Ok(())
}
