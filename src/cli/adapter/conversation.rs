//! Terminal adapters for conversation creation, inspection, and settings.

use anyhow::Result;
use std::sync::Arc;

use crate::conversation::{ConversationId, MessageId};
use crate::llm::ModelName;
use crate::operation;
use crate::output::TerminalOutput;
use crate::plugin::{PluginCatalog, PluginStore, bundled_index};
use crate::store::Store;
use crate::tool::ToolProviderRegistry;

use super::system;

/// Creates an empty persisted conversation and prints only its ID.
pub(crate) async fn new_conversation() -> Result<()> {
    let models = operation::list_models(system::gateway_url(), system::base_url()).await?;
    let model = operation::preferred_model(models).ok_or_else(|| {
        anyhow::anyhow!("no models are available; configure a provider key first")
    })?;
    let store = Store::open()?;
    let conversation_id = operation::create_conversation(&store, &model)?;

    TerminalOutput.created_conversation(&conversation_id);
    Ok(())
}

/// Lists persisted conversations without loading their full message history.
pub(crate) fn list_conversations(json: bool) -> Result<()> {
    let store = Store::open()?;
    let conversations = operation::list_conversations(&store)?;

    if json {
        TerminalOutput.conversations_json(&conversations)?;
    } else {
        TerminalOutput.conversations(&conversations);
    }

    Ok(())
}

/// Loads and prints all messages for one conversation.
pub(crate) fn show_conversation(conversation_id: ConversationId) -> Result<()> {
    let store = Store::open()?;
    let messages = store.load_messages(&conversation_id)?;

    TerminalOutput.conversation_messages(&messages);
    Ok(())
}

/// Loads and prints the full message tree for one conversation.
pub(crate) fn show_tree(conversation_id: ConversationId) -> Result<()> {
    let store = Store::open()?;
    let tree = operation::conversation_tree(&store, &conversation_id)?;

    TerminalOutput.conversation_tree(&tree.messages);
    Ok(())
}

/// Loads full read-only runtime state and prints it as stable JSON.
pub(crate) fn inspect_conversation(
    conversation_id: ConversationId,
    head_message_id: Option<MessageId>,
    model: Option<ModelName>,
) -> Result<()> {
    let store = Store::open()?;
    let tools = ToolProviderRegistry::with_installed_plugins()?;
    let plugin_catalog =
        PluginCatalog::new(Arc::new(PluginStore::default_store()?), bundled_index()?);
    let report = operation::inspect_conversation(
        &store,
        &conversation_id,
        head_message_id.as_ref(),
        model,
        &tools,
        Some(&plugin_catalog),
    )?;

    TerminalOutput.inspection_report_json(&report)
}

/// Creates a new conversation copied through one checkpoint message.
pub(crate) fn fork_conversation(
    conversation_id: ConversationId,
    message_id: MessageId,
) -> Result<()> {
    let mut store = Store::open()?;
    let forked_conversation_id =
        operation::fork_conversation(&mut store, &conversation_id, &message_id)?;

    TerminalOutput.forked_conversation(&forked_conversation_id);
    Ok(())
}

/// Deletes one conversation and all persisted data owned by it.
pub(crate) fn remove_conversation(conversation_id: ConversationId) -> Result<()> {
    let mut store = Store::open()?;
    operation::remove_conversation(&mut store, &conversation_id)?;

    TerminalOutput.removed_conversation(&conversation_id);
    Ok(())
}

/// Persists the default model for future turns in one conversation.
pub(crate) fn set_model(conversation_id: ConversationId, model: ModelName) -> Result<()> {
    let mut store = Store::open()?;
    operation::set_conversation_model(&mut store, &conversation_id, &model)?;

    TerminalOutput.set_model(&conversation_id, &model);
    Ok(())
}
