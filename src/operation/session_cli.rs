//! CLI session operation adapter.
//!
//! This module runs the shared session workflows from a terminal process and
//! records the same replayable session events that API-owned sessions expose.

use super::*;

use std::sync::Arc;

/// Creates a session branch at a conversation head and advances it to blocked.
pub async fn start_cli_session(
    conversation_id: ConversationId,
    head_message_id: Option<MessageId>,
    model: Option<ModelName>,
    gateway_url: GatewayUrl,
    base_url: BaseUrl,
) -> Result<()> {
    let mut store = Store::open()?;
    let model = match model {
        Some(model) => model,
        None => conversation_model(&store, &conversation_id)?,
    };
    let session = store.create_session(
        &SessionId::fresh(),
        &conversation_id,
        head_message_id.as_ref(),
        model.as_str(),
        None,
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
    let runtime_context = CliRuntimeContext::load()?;
    let runtime = runtime_context.dependencies(&session, gateway_url, base_url);
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
    .await;

    finish_cli_session(&mut store, &session_id, outcome)
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
    let runtime_context = CliRuntimeContext::load()?;
    let runtime = runtime_context.dependencies(&session, gateway_url, base_url);
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
    .await;

    finish_cli_session(&mut store, &session_id, outcome)
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
    let runtime_context = CliRuntimeContext::load()?;
    let runtime = runtime_context.dependencies(&session, gateway_url, base_url);
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
    .await;

    finish_cli_session(store, session_id, outcome)
}

/// Finishes a CLI session and persists a failure when runtime advancement
/// fails, matching the API session manager's failure behavior.
fn finish_cli_session(
    store: &mut Store,
    session_id: &SessionId,
    outcome: Result<RuntimeOutcome>,
) -> Result<()> {
    let result = outcome.and_then(|outcome| finish_session(store, session_id, outcome).map(|_| ()));
    if let Err(error) = result {
        if let Err(failure_error) = record_session_failure(store, session_id, &error) {
            eprintln!("failed to persist cli session failure: {failure_error}");
        }
        return Err(error);
    }

    Ok(())
}

/// Owns the CLI process's provider registry and read-only plugin catalog for
/// one session operation.
struct CliRuntimeContext {
    registry: ToolProviderRegistry,
    plugin_catalog: crate::plugin::PluginCatalog,
}

impl CliRuntimeContext {
    /// Loads the same provider and plugin sources used by API sessions.
    fn load() -> Result<Self> {
        Ok(Self {
            registry: ToolProviderRegistry::with_installed_plugins()?,
            plugin_catalog: crate::plugin::PluginCatalog::new(
                Arc::new(crate::plugin::PluginStore::default_store()?),
                crate::plugin::bundled_index()?,
            ),
        })
    }

    /// Builds runtime dependencies from the durable session record.
    fn dependencies(
        &self,
        session: &Session,
        gateway_url: GatewayUrl,
        base_url: BaseUrl,
    ) -> RuntimeDependencies<'_> {
        RuntimeDependencies::for_session(
            session,
            gateway_url,
            base_url,
            &self.registry,
            Some(&self.plugin_catalog),
        )
    }
}

/// CLI runtime output that prints to the terminal and appends replayable events.
struct CliSessionOutput {
    recorder: SessionEventRecorder,
    terminal: TerminalOutput,
}

impl CliSessionOutput {
    fn new(session_id: SessionId) -> Self {
        Self {
            recorder: SessionEventRecorder::new(None, session_id),
            terminal: TerminalOutput,
        }
    }

    fn record(&self, event: SessionEvent) -> Result<()> {
        self.recorder.record(event)?;
        Ok(())
    }
}

impl RuntimeOutput for CliSessionOutput {
    fn start_assistant_message(&self) {
        self.terminal.start_assistant_message();
    }

    fn assistant_attempt_reset(&self) {
        if let Err(error) = self.record(SessionEvent::AssistantAttemptReset) {
            eprintln!("failed to append assistant attempt reset event: {error}");
        }
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
    recorder: SessionEventRecorder,
}

impl CliSessionEvents {
    fn new(session_id: SessionId) -> Self {
        Self {
            recorder: SessionEventRecorder::new(None, session_id),
        }
    }

    fn record(&self, event: SessionEvent) {
        match self.recorder.record(event) {
            Ok(_) => {}
            Err(error) => eprintln!("failed to append runtime event: {error}"),
        }
    }
}

impl CliSessionEvents {
    fn update_head(&self, message_id: &MessageId) {
        if let Err(error) = self.recorder.update_head(message_id) {
            eprintln!("failed to update cli session head: {error}");
        }
    }
}

impl RuntimeEventSink for CliSessionEvents {
    fn assistant_message_saved(&self, message_id: &MessageId) {
        self.record(SessionEvent::AssistantMessageSaved {
            message_id: message_id.as_str().to_string(),
        });
        self.update_head(message_id);
    }

    fn tool_result_saved(&self, message_id: &MessageId) {
        self.record(SessionEvent::ToolResultSaved {
            message_id: message_id.as_str().to_string(),
        });
        self.update_head(message_id);
    }
}
