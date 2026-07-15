//! CLI session operation adapter.
//!
//! This module runs the shared session workflows from a terminal process and
//! records the same replayable session events that API-owned sessions expose.

use super::*;

/// Starts and advances a CLI-owned session from a conversation wakeup.
pub async fn start_cli_session(
    conversation_id: ConversationId,
    head_message_id: Option<MessageId>,
    model: Option<ModelName>,
    gateway_url: GatewayUrl,
    base_url: BaseUrl,
) -> Result<()> {
    let mut store = Store::open()?;
    let session = start_session_from_wakeup(
        &mut store,
        ContinueWakeup {
            conversation_id,
            head_message_id,
            model,
            reasoning: None,
        },
    )?;
    let output = TerminalOutput;

    output.created_session(&session.id);
    continue_cli_session(&mut store, &session.id, gateway_url, base_url).await
}

/// Executes one approved CLI session-owned tool call and continues the session.
pub async fn approve_cli_session_tool(
    session_id: SessionId,
    tool_call_id: ToolCallId,
    gateway_url: GatewayUrl,
    base_url: BaseUrl,
) -> Result<()> {
    let mut store = Store::open()?;
    let session = store.load_session(&session_id)?;
    let registry = ToolProviderRegistry::new();
    let runtime = RuntimeDependencies::new(
        gateway_url,
        base_url,
        Some(ModelName::new(session.model)),
        session.reasoning,
        &registry,
    );
    let cli_output = CliSessionOutput::new(session_id.clone());
    let events = CliSessionEvents::new(session_id.clone());
    let outcome = approve_session_tool(
        &cli_output,
        &events,
        &mut store,
        &session.conversation_id,
        session.current_head_message_id.as_ref(),
        &tool_call_id,
        runtime,
    )
    .await?;

    finish_session(&mut store, &session_id, outcome)?;
    Ok(())
}

/// Stores one denied CLI session-owned tool result and continues the session.
pub async fn deny_cli_session_tool(
    session_id: SessionId,
    tool_call_id: ToolCallId,
    gateway_url: GatewayUrl,
    base_url: BaseUrl,
) -> Result<()> {
    let mut store = Store::open()?;
    let session = store.load_session(&session_id)?;
    let registry = ToolProviderRegistry::new();
    let runtime = RuntimeDependencies::new(
        gateway_url,
        base_url,
        Some(ModelName::new(session.model)),
        session.reasoning,
        &registry,
    );
    let cli_output = CliSessionOutput::new(session_id.clone());
    let events = CliSessionEvents::new(session_id.clone());
    let outcome = deny_session_tool(
        &cli_output,
        &events,
        &mut store,
        &session.conversation_id,
        session.current_head_message_id.as_ref(),
        &tool_call_id,
        runtime,
    )
    .await?;

    finish_session(&mut store, &session_id, outcome)?;
    Ok(())
}

/// Cancels one persisted CLI session and returns the updated state.
pub fn cancel_session(session_id: &SessionId) -> Result<Session> {
    let mut store = Store::open()?;

    store.update_session_status(session_id, SessionStatus::Cancelled, None)?;
    store.load_session(session_id)
}

/// Continues a CLI-owned session until it completes or reaches approval.
async fn continue_cli_session(
    store: &mut Store,
    session_id: &SessionId,
    gateway_url: GatewayUrl,
    base_url: BaseUrl,
) -> Result<()> {
    let session = store.load_session(session_id)?;
    store.update_session_status(session_id, SessionStatus::Running, None)?;
    let registry = ToolProviderRegistry::new();
    let runtime = RuntimeDependencies::new(
        gateway_url,
        base_url,
        Some(ModelName::new(session.model)),
        session.reasoning,
        &registry,
    );
    let cli_output = CliSessionOutput::new(session_id.clone());
    let events = CliSessionEvents::new(session_id.clone());
    let outcome = advance_session_until_blocked(
        &cli_output,
        &events,
        store,
        &session.conversation_id,
        session.current_head_message_id.as_ref(),
        runtime,
    )
    .await?;

    finish_session(store, session_id, outcome)?;
    Ok(())
}

/// CLI runtime output that prints to the terminal and appends replayable events.
struct CliSessionOutput {
    session_id: SessionId,
    terminal: TerminalOutput,
}

impl CliSessionOutput {
    fn new(session_id: SessionId) -> Self {
        Self {
            session_id,
            terminal: TerminalOutput,
        }
    }

    fn record(&self, event: SessionEvent) -> Result<()> {
        let mut store = Store::open()?;
        store.append_session_event(&self.session_id, event)?;

        Ok(())
    }
}

impl RuntimeOutput for CliSessionOutput {
    fn start_assistant_message(&self) {
        self.terminal.start_assistant_message();
    }

    fn assistant_delta(&self, text: &str) -> Result<()> {
        self.record(SessionEvent::AssistantDelta {
            text: text.to_string(),
        })?;
        self.terminal.assistant_delta(text)
    }

    fn reasoning_delta(&self, text: &str) -> Result<()> {
        self.record(SessionEvent::ReasoningDelta {
            text: text.to_string(),
        })
    }

    fn tool_call_delta(
        &self,
        index: u16,
        id: Option<&str>,
        name: Option<&str>,
        arguments_delta: Option<&str>,
    ) -> Result<()> {
        self.record(SessionEvent::ToolCallDelta {
            index,
            id: id.map(str::to_string),
            name: name.map(str::to_string),
            arguments_delta: arguments_delta.map(str::to_string),
        })
    }

    fn end_assistant_message(&self) {
        self.terminal.end_assistant_message();
    }

    fn assistant_tool_calls(&self, tool_calls: &[crate::conversation::ToolCall]) {
        self.terminal.assistant_tool_calls(tool_calls);
    }
}

/// CLI runtime sink for durable message events.
struct CliSessionEvents {
    session_id: SessionId,
}

impl CliSessionEvents {
    fn new(session_id: SessionId) -> Self {
        Self { session_id }
    }

    fn record(&self, event: SessionEvent) {
        match Store::open()
            .and_then(|mut store| store.append_session_event(&self.session_id, event))
        {
            Ok(_) => {}
            Err(error) => eprintln!("failed to append runtime event: {error}"),
        }
    }
}

impl RuntimeEventSink for CliSessionEvents {
    fn assistant_message_saved(&self, message_id: &MessageId) {
        self.record(SessionEvent::AssistantMessageSaved {
            message_id: message_id.as_str().to_string(),
        });
    }

    fn tool_result_saved(&self, message_id: &MessageId) {
        self.record(SessionEvent::ToolResultSaved {
            message_id: message_id.as_str().to_string(),
        });
    }
}
