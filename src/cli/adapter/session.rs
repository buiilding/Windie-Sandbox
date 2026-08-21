//! Terminal adapters for durable session commands.

use anyhow::Result;

use crate::conversation::{ConversationId, MessageId, ToolCallId};
use crate::llm::ModelName;
use crate::operation;
use crate::output::TerminalOutput;
use crate::session::SessionId;
use crate::store::Store;
use crate::tool::ToolProviderRegistry;

use super::system;

/// Starts and advances one session from an explicit or default conversation head.
pub(crate) async fn start(
    conversation_id: ConversationId,
    head_message_id: Option<MessageId>,
    model: Option<ModelName>,
) -> Result<()> {
    operation::start_cli_session(
        conversation_id,
        head_message_id,
        model,
        system::gateway_url(),
        system::base_url(),
    )
    .await
}

/// Lists persisted sessions, optionally limited to one conversation.
pub(crate) fn list(conversation_id: Option<ConversationId>) -> Result<()> {
    let store = Store::open()?;
    let sessions = match conversation_id {
        Some(conversation_id) => store.list_conversation_sessions(&conversation_id)?,
        None => store.list_sessions()?,
    };

    TerminalOutput.sessions(&sessions);
    Ok(())
}

/// Prints one persisted session status.
pub(crate) fn status(session_id: SessionId) -> Result<()> {
    let store = Store::open()?;
    let session = store.load_session(&session_id)?;

    TerminalOutput.session_status(&session);
    Ok(())
}

/// Prints persisted session events.
pub(crate) fn events(session_id: SessionId) -> Result<()> {
    let store = Store::open()?;

    for event in store.load_session_events_after(&session_id, None)? {
        TerminalOutput.session_event(&event);
    }

    Ok(())
}

/// Lists session-owned approvals for one session.
pub(crate) fn approvals(session_id: SessionId) -> Result<()> {
    let store = Store::open()?;
    let registry = ToolProviderRegistry::with_installed_plugins()?;
    let session = store.load_session(&session_id)?;
    let approvals = operation::list_session_approvals_with_registry(&store, &session, &registry)?;

    TerminalOutput.session_approvals(&approvals);
    Ok(())
}

/// Executes one approved session-owned tool call and continues that session.
pub(crate) async fn approve(session_id: SessionId, tool_call_id: ToolCallId) -> Result<()> {
    operation::approve_cli_session_tool(
        session_id,
        tool_call_id,
        system::gateway_url(),
        system::base_url(),
    )
    .await
}

/// Stores one denied session-owned tool result and continues that session.
pub(crate) async fn deny(session_id: SessionId, tool_call_id: ToolCallId) -> Result<()> {
    operation::deny_cli_session_tool(
        session_id,
        tool_call_id,
        system::gateway_url(),
        system::base_url(),
    )
    .await
}

/// Cancels one persisted session.
pub(crate) fn stop(session_id: SessionId) -> Result<()> {
    let mut store = Store::open()?;
    let (session, _) = operation::cancel_session(&mut store, &session_id)?;

    TerminalOutput.session_status(&session);
    Ok(())
}
