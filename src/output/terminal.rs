//! Terminal output implementation.
//!
//! This module owns the concrete terminal printer and the runtime streaming
//! adapter used by the CLI.

use std::io::{self, Write};
use std::net::SocketAddr;
use std::path::Path;

use anyhow::{Context, Result};

use crate::conversation::{ConversationId, Message, MessageId, ToolCall};
use crate::llm::{ModelInfo, ModelName};
use crate::local::process::{ManagedComponent, ProcessReport, ProcessState};
use crate::local::{InstallReport, InstallStatus};
use crate::operation::InspectionReport;
use crate::operation::UninstallReport;
use crate::perf::{PerformanceBaseline, PerformanceComparison, PerformanceReport};
use crate::session::{Session, SessionEvent, SessionEventRecord, SessionId};
use crate::store::ConversationInfo;
use crate::tool::{ToolDefinition, ToolSchemaName};

use super::*;

/// Converts a scenario layer into a readable section heading.
fn title_case(value: &str) -> String {
    if value == "api" {
        return "API".to_string();
    }
    if value == "mcp" {
        return "MCP".to_string();
    }

    let mut chars = value.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Minimal output interface needed by runtime flows.
///
/// Tests can implement this trait without depending on terminal stdout.
pub(crate) trait RuntimeOutput {
    fn start_assistant_message(&self);
    fn assistant_delta(&self, text: &str) -> Result<()>;
    /// Receives live reasoning-summary text when a provider streams it.
    ///
    /// The default no-op keeps CLI output unchanged. Streaming clients can
    /// override this to show a separate reasoning lane while the final
    /// persisted assistant metadata remains the source of truth.
    fn reasoning_delta(&self, _text: &str) -> Result<()> {
        Ok(())
    }
    /// Receives live function-call metadata or argument text.
    ///
    /// The default no-op keeps terminal output focused on assistant text.
    /// Developer clients can override it to build a live tool-call lane before
    /// the final assistant message is saved.
    fn tool_call_delta(
        &self,
        _index: u16,
        _id: Option<&str>,
        _name: Option<&str>,
        _arguments_delta: Option<&str>,
    ) -> Result<()> {
        Ok(())
    }
    fn end_assistant_message(&self);
    /// Clears transient output from a failed model attempt before a retry or
    /// terminal failure. Stateful clients use this to discard partial lanes.
    fn assistant_attempt_reset(&self) {}
    fn assistant_tool_calls(&self, tool_calls: &[ToolCall]);
}

/// Concrete stdout/stderr-free terminal printer for the CLI.
pub struct TerminalOutput;

impl TerminalOutput {
    /// Prints the static command help.
    pub fn help(&self) {
        print_lines(&help_lines());
    }

    /// Prints help prefixed by an invalid usage line.
    pub fn invalid_usage(&self) {
        print_lines(&invalid_usage_lines());
    }

    /// Prints the current package version.
    pub fn version(&self) {
        println!("windie {}", env!("CARGO_PKG_VERSION"));
    }

    /// Prints the local API address when the foreground server is ready.
    pub fn api_started(&self, address: &SocketAddr) {
        println!("windie api listening on http://{address}");
    }

    /// Prints one detached component lifecycle result.
    pub fn component_report(&self, report: &ProcessReport) {
        println!(
            "windie {}: {}",
            report.component.as_str(),
            Self::process_state_label(report.state)
        );
        if let Some(pid) = report.pid {
            println!("pid: {pid}");
        }
        println!("output: {}", report.log_file.display());
    }

    /// Prints one component's persisted stdout/stderr output.
    pub fn component_output(&self, component: ManagedComponent, output: &str) {
        if output.is_empty() {
            println!("windie {}: no output", component.as_str());
            return;
        }
        print!("{output}");
        if !output.ends_with('\n') {
            println!();
        }
    }

    /// Prints a complete uninstall plan or result.
    pub fn uninstall_report(&self, report: &UninstallReport) {
        if report.dry_run {
            println!("windie uninstall: dry run");
            println!("would stop: tray, api, inspector, gateway");
            println!("would remove data: {}", report.plan.windie_home.display());
            for binary in &report.plan.binaries {
                println!("would remove binary: {}", binary.display());
            }
            return;
        }

        for process in &report.processes {
            println!(
                "{} {}",
                Self::process_state_label(process.state),
                process.component.as_str()
            );
        }
        if let Some(gateway) = &report.gateway {
            println!(
                "{} {}",
                Self::process_state_label(gateway.state),
                gateway.component.as_str()
            );
        }

        if let Some(cleanup) = &report.cleanup {
            if cleanup.cleanup_scheduled {
                println!("windie uninstall: cleanup scheduled");
            } else {
                println!("windie uninstall: removed Windie data and binaries");
            }
        }
    }

    /// Returns the stable terminal label for one process state.
    fn process_state_label(state: ProcessState) -> &'static str {
        match state {
            ProcessState::Started => "started",
            ProcessState::AlreadyRunning => "already running",
            ProcessState::Stopped => "stopped",
            ProcessState::NotRunning => "not running",
        }
    }

    /// Prints named benchmark scenarios grouped by architectural layer.
    pub fn performance_baseline(&self, baseline: &PerformanceBaseline) {
        println!("performance baseline");
        println!("mode: {}", baseline.mode.as_str());
        if baseline.mode.may_call_provider() {
            println!("warning: live benchmark sent a real provider request and may cost money");
        }
        println!("model: {}", baseline.model);
        if let Some(conversation_id) = baseline.conversation_id.as_ref() {
            println!("conversation: {conversation_id}");
        }
        let mut current_layer = None;
        for scenario in &baseline.scenarios {
            if current_layer.as_deref() != Some(scenario.layer.as_str()) {
                println!();
                println!("{}", title_case(&scenario.layer));
                current_layer = Some(scenario.layer.clone());
            }
            println!("  {}", scenario.name);
            println!("    fixture: {}", scenario.fixture);
            println!("    time: {}", format_duration(scenario.duration));
        }
    }

    /// Prints an aggregated benchmark report from repeated runs.
    pub fn performance_report(&self, report: &PerformanceReport) {
        for line in performance_report_lines(report) {
            println!("{line}");
        }
    }

    /// Prints a benchmark report as stable JSON for shell redirection.
    pub fn performance_report_json(&self, report: &PerformanceReport) -> Result<()> {
        serde_json::to_writer_pretty(io::stdout(), report)
            .context("failed to write benchmark JSON")?;
        println!();

        Ok(())
    }

    /// Prints a full read-only runtime inspection report as stable JSON.
    pub fn inspection_report_json(&self, report: &InspectionReport) -> Result<()> {
        serde_json::to_writer_pretty(io::stdout(), report)
            .context("failed to write inspection JSON")?;
        println!();

        Ok(())
    }

    /// Prints a comparison between two persisted benchmark reports.
    pub fn performance_comparison(&self, comparison: &PerformanceComparison) {
        for line in performance_comparison_lines(comparison) {
            println!("{line}");
        }
    }

    /// Prints the path written by `windie update baseline`.
    pub fn updated_baseline(&self, path: &Path) {
        println!("updated baseline {}", path.display());
    }

    /// Prints one install or verification result.
    pub fn install_report(&self, report: &InstallReport) {
        match report.status {
            InstallStatus::Detected => println!("detected {}", report.target),
            InstallStatus::Installed => println!("installed {}", report.target),
        }
        println!("{}", report.message);
    }

    /// Prints the provider-key environment file path.
    pub fn env_path(&self, path: &Path) {
        println!("{}", path.display());
    }

    /// Confirms that Windie's provider-key environment file changed.
    pub fn env_updated(&self, path: &Path, count: usize) {
        println!("updated {count} env value(s) in {}", path.display());
    }

    /// Prints provider-key names without exposing secret values.
    pub fn env_keys(&self, keys: &[String]) {
        if keys.is_empty() {
            println!("no env values");
            return;
        }
        for key in keys {
            println!("{key}");
        }
    }

    /// Prints the created conversation ID as machine-readable command output.
    pub fn created_conversation(&self, conversation_id: &ConversationId) {
        println!("{conversation_id}");
    }

    /// Prints the inserted message ID as machine-readable command output.
    pub fn inserted_message(&self, message_id: &MessageId) {
        println!("{message_id}");
    }

    /// Confirms that one message was updated.
    pub fn updated_message(&self, message_id: &MessageId) {
        println!("updated message {message_id}");
    }

    /// Confirms that the root-scoped system prompt was set.
    pub fn set_system_prompt(&self, conversation_id: &ConversationId) {
        println!("set systemprompt {conversation_id}");
    }

    /// Confirms that the conversation default model was set.
    pub fn set_model(&self, conversation_id: &ConversationId, model: &ModelName) {
        println!("set model {conversation_id} {model}");
    }

    /// Confirms that the root-scoped system prompt was removed.
    pub fn removed_system_prompt(&self, conversation_id: &ConversationId) {
        println!("removed systemprompt {conversation_id}");
    }

    /// Confirms that one tool schema was inserted.
    pub fn inserted_tool_schema(&self, name: &ToolSchemaName) {
        println!("inserted toolschema {name}");
    }

    /// Confirms that one tool schema was updated.
    pub fn updated_tool_schema(&self, name: &ToolSchemaName) {
        println!("updated toolschema {name}");
    }

    /// Confirms that one tool schema was removed.
    pub fn removed_tool_schema(&self, name: &ToolSchemaName) {
        println!("removed toolschema {name}");
    }

    /// Confirms that one message was selected as active.
    /// Confirms that one conversation was removed.
    pub fn removed_conversation(&self, conversation_id: &ConversationId) {
        println!("removed conversation {conversation_id}");
    }

    /// Confirms that one message was removed.
    pub fn removed_message(&self, message_id: &MessageId) {
        println!("removed message {message_id}");
    }

    /// Confirms that messages after a checkpoint were removed.
    pub fn truncated_conversation(&self, conversation_id: &ConversationId, message_id: &MessageId) {
        println!("truncated conversation {conversation_id} after message {message_id}");
    }

    /// Prints the forked conversation ID as machine-readable command output.
    pub fn forked_conversation(&self, conversation_id: &ConversationId) {
        println!("{conversation_id}");
    }

    /// Prints the local gateway readiness summary.
    pub fn status(&self, gateway_running: bool) {
        println!("status");
        println!(
            "gateway: {}",
            if gateway_running {
                "running"
            } else {
                "not running"
            }
        );
    }

    /// Prints models currently reported by the running Bifrost gateway.
    pub fn models(&self, models: &[ModelInfo]) {
        print_lines(&model_lines(models));
    }

    /// Prints provider tools that can be attached to conversations.
    pub fn available_tools(&self, tools: &[ToolDefinition]) {
        for line in available_tool_lines(tools) {
            println!("{line}");
        }
    }

    /// Prints the conversation list in the CLI format.
    pub fn conversations(&self, conversations: &[ConversationInfo]) {
        for line in conversation_lines(conversations) {
            println!("{line}");
        }
    }

    /// Prints the conversation list as stable JSON for developer tools.
    pub fn conversations_json(&self, conversations: &[ConversationInfo]) -> Result<()> {
        let report = ConversationListReport::new(conversations);

        serde_json::to_writer_pretty(io::stdout(), &report)
            .context("failed to write conversation list JSON")?;
        println!();

        Ok(())
    }

    /// Prints message previews for one conversation.
    pub fn conversation_messages(&self, messages: &[Message]) {
        for line in message_lines(messages) {
            println!("{line}");
        }
    }

    /// Prints the full message tree with indentation and active marker.
    pub fn conversation_tree(&self, messages: &[Message]) {
        for line in tree_lines(messages) {
            println!("{line}");
        }
    }

    /// Starts the assistant stream on a fresh visual line.
    pub fn start_assistant_message(&self) {
        println!();
    }

    /// Prints one streamed assistant delta immediately.
    pub fn assistant_delta(&self, text: &str) -> Result<()> {
        print!("{text}");
        io::stdout()
            .flush()
            .context("failed to flush assistant output")
    }

    /// Ends the assistant stream with spacing before the process exits.
    pub fn end_assistant_message(&self) {
        println!("\n");
    }

    /// Prints model-requested tool calls after the stream is complete.
    pub fn assistant_tool_calls(&self, tool_calls: &[ToolCall]) {
        if tool_calls.is_empty() {
            return;
        }

        println!("tool calls");
        for tool_call in tool_calls {
            println!(
                "{}  {}  {}",
                tool_call.id,
                tool_call.name(),
                text_preview(tool_call.arguments())
            );
        }
        println!();
    }

    /// Prints the created session ID as machine-readable command output.
    pub fn created_session(&self, session_id: &SessionId) {
        println!("{session_id}");
    }

    /// Prints one session's persisted lifecycle state.
    pub fn session_status(&self, session: &Session) {
        println!("session {}", session.id);
        println!("conversation: {}", session.conversation_id);
        println!(
            "start head: {}",
            session
                .start_head_message_id
                .as_ref()
                .map(MessageId::as_str)
                .unwrap_or("(empty)")
        );
        println!(
            "current head: {}",
            session
                .current_head_message_id
                .as_ref()
                .map(MessageId::as_str)
                .unwrap_or("(empty)")
        );
        println!("status: {}", session.status);
        println!("model: {}", session.model);
        if let Some(error) = session.error.as_ref() {
            println!("error: {error}");
        }
    }

    /// Prints a compact list of runtime sessions.
    pub fn sessions(&self, sessions: &[Session]) {
        if sessions.is_empty() {
            println!("no sessions");
            return;
        }

        println!("sessions");
        for session in sessions {
            println!(
                "{}  {}  {}  {}",
                session.id,
                session.status,
                session.conversation_id,
                session
                    .current_head_message_id
                    .as_ref()
                    .map(MessageId::as_str)
                    .unwrap_or("(empty)")
            );
        }
    }

    /// Prints pending session-owned approvals in a compact inspectable format.
    pub fn session_approvals(&self, approvals: &[crate::operation::SessionToolApprovalRequest]) {
        if approvals.is_empty() {
            println!("no pending approvals");
            return;
        }

        println!("pending approvals");
        for approval in approvals {
            let tool_call = &approval.approval.tool_call;
            println!(
                "{}  {}  {}  {}  {}",
                approval.session_id,
                tool_call.id,
                tool_call.name(),
                approval.approval.reason,
                text_preview(tool_call.arguments())
            );
        }
    }

    /// Prints one persisted session event.
    pub fn session_event(&self, event: &SessionEventRecord) {
        match &event.event {
            SessionEvent::InputQueued {
                input_id,
                queue_depth,
            } => println!("input queued {input_id} (depth {queue_depth})"),
            SessionEvent::InputStarted {
                input_id,
                message_id,
            } => println!("input started {input_id} as message {message_id}"),
            SessionEvent::AssistantDelta { text } => print!("{text}"),
            SessionEvent::ReasoningDelta { text } => print!("{text}"),
            SessionEvent::ToolCallDelta {
                index,
                id,
                name,
                arguments_delta,
            } => println!(
                "tool call delta  #{index}  {}  {}  {}",
                id.as_deref().unwrap_or("(no id)"),
                name.as_deref().unwrap_or("(no name)"),
                arguments_delta.as_deref().unwrap_or("")
            ),
            SessionEvent::AssistantAttemptReset => {}
            SessionEvent::AssistantMessageSaved { message_id } => {
                println!("assistant message saved {message_id}");
            }
            SessionEvent::ToolResultSaved { message_id } => {
                println!("tool result saved {message_id}");
            }
            SessionEvent::WaitingForApproval => println!("waiting for approval"),
            SessionEvent::Completed { message_id } => {
                println!("completed {}", message_id.as_deref().unwrap_or("(empty)"))
            }
            SessionEvent::Failed { error, .. } => println!("failed {error}"),
            SessionEvent::Cancelled => println!("cancelled"),
        }
    }
}

impl RuntimeOutput for TerminalOutput {
    fn start_assistant_message(&self) {
        TerminalOutput::start_assistant_message(self);
    }

    fn assistant_delta(&self, text: &str) -> Result<()> {
        TerminalOutput::assistant_delta(self, text)
    }

    fn end_assistant_message(&self) {
        TerminalOutput::end_assistant_message(self);
    }

    fn assistant_tool_calls(&self, tool_calls: &[ToolCall]) {
        TerminalOutput::assistant_tool_calls(self, tool_calls);
    }
}
