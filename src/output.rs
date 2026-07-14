//! Terminal output boundary.
//!
//! This module owns CLI printing for assistant streams and command output.
//! Other modules should pass display data here instead of formatting terminal
//! output themselves.

use std::collections::HashMap;
use std::io::{self, Write};
use std::net::SocketAddr;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Serialize;

use crate::conversation::{
    ConversationId, Message, MessageId, MessagePart, ToolCall, ToolSchemaName,
};
use crate::llm::{ModelInfo, ModelName};
use crate::operation::InspectionReport;
use crate::perf::{DurationMetric, PerformanceBaseline, PerformanceComparison, PerformanceReport};
use crate::run::{Run, RunEvent, RunEventRecord, RunId};
use crate::setup::InstallReport;
use crate::store::ConversationInfo;
use crate::tool::ToolDefinition;

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

    /// Prints the local API address and generated access token at startup.
    pub fn api_started(&self, address: &SocketAddr, api_token: &str) {
        println!("windie api listening on http://{address}");
        println!("windie api token: {api_token}");
        println!("windie inspector: {}", inspector_url(api_token));
    }

    /// Prints the browser inspector URL opened by `windie inspector`.
    pub fn inspector_opened(&self, url: &str, started_server: bool) {
        if started_server {
            println!("windie inspector server: started");
        } else {
            println!("windie inspector server: already running");
        }
        println!("windie inspector: {url}");
    }

    /// Prints all fields measured by a performance baseline.
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
        if let Some(duration) = baseline.store_open {
            println!("store open: {}", format_duration(duration));
        }
        if let Some(duration) = baseline.conversation_load {
            println!("active path load: {}", format_duration(duration));
        }
        if let Some(duration) = baseline.active_message_lookup {
            println!("active message lookup: {}", format_duration(duration));
        }
        if let Some(duration) = baseline.active_path_row_load {
            println!("active path row load: {}", format_duration(duration));
        }
        if let Some(duration) = baseline.active_path_part_load {
            println!("active path part/image load: {}", format_duration(duration));
        }
        if let Some(duration) = baseline.tree_load {
            println!("tree load: {}", format_duration(duration));
        }
        if let Some(duration) = baseline.tree_row_load {
            println!("tree row load: {}", format_duration(duration));
        }
        if let Some(duration) = baseline.tree_part_load {
            println!("tree part/image load: {}", format_duration(duration));
        }
        if let Some(duration) = baseline.tool_schema_load {
            println!("tool schema load: {}", format_duration(duration));
        }
        if let Some(duration) = baseline.context_build {
            println!("context build: {}", format_duration(duration));
        }
        if let Some(duration) = baseline.context_active_path_load {
            println!("context active path load: {}", format_duration(duration));
        }
        if let Some(duration) = baseline.context_system_prompt_load {
            println!("context system prompt load: {}", format_duration(duration));
        }
        if let Some(duration) = baseline.context_compaction_load {
            println!("context compaction load: {}", format_duration(duration));
        }
        if let Some(duration) = baseline.context_flatten {
            println!("context flatten: {}", format_duration(duration));
        }
        if let Some(duration) = baseline.prepare_run_head_turn {
            println!("prepare run head turn: {}", format_duration(duration));
        }
        if let Some(duration) = baseline.pending_tool_approval_scan {
            println!("pending tool approval scan: {}", format_duration(duration));
        }
        if let Some(duration) = baseline.tool_result_insert {
            println!("tool result insert: {}", format_duration(duration));
        }
        if let Some(duration) = baseline.deny_tool_result_persist {
            println!("deny tool result persist: {}", format_duration(duration));
        }
        if let Some(duration) = baseline.splice_remove {
            println!("splice remove: {}", format_duration(duration));
        }
        if let Some(duration) = baseline.truncate {
            println!("truncate: {}", format_duration(duration));
        }
        if let Some(duration) = baseline.context_build_after_tool_chain {
            println!(
                "context build after tool chain: {}",
                format_duration(duration)
            );
        }
        if let Some(duration) = baseline.active_path_load_100 {
            println!("active path load 100: {}", format_duration(duration));
        }
        if let Some(duration) = baseline.active_path_load_1000 {
            println!("active path load 1000: {}", format_duration(duration));
        }
        if let Some(duration) = baseline.pending_tool_approval_scan_long_path {
            println!(
                "pending tool approval scan long path: {}",
                format_duration(duration)
            );
        }
        if let Some(duration) = baseline.pending_tool_approval_scan_deep_chain {
            println!(
                "pending tool approval scan deep chain: {}",
                format_duration(duration)
            );
        }
        if let Some(duration) = baseline.prepare_run_head_no_tools {
            println!("prepare query no tools: {}", format_duration(duration));
        }
        if let Some(duration) = baseline.prepare_run_head_completed_tool_chain {
            println!(
                "prepare query completed tool chain: {}",
                format_duration(duration)
            );
        }
        if let Some(duration) = baseline.prepare_run_head_requires_approval {
            println!(
                "prepare query requires approval: {}",
                format_duration(duration)
            );
        }
        if let Some(duration) = baseline.prepare_run_head_policy_denied {
            println!("prepare query policy denied: {}", format_duration(duration));
        }
        if let Some(duration) = baseline.splice_remove_branch_point {
            println!("splice remove branch point: {}", format_duration(duration));
        }
        if let Some(duration) = baseline.splice_remove_root_many_children {
            println!(
                "splice remove root many children: {}",
                format_duration(duration)
            );
        }
        if let Some(duration) = baseline.splice_remove_tool_group {
            println!("splice remove tool group: {}", format_duration(duration));
        }
        if let Some(duration) = baseline.truncate_large_subtree {
            println!("truncate large subtree: {}", format_duration(duration));
        }
        if let Some(duration) = baseline.context_build_plain_100 {
            println!("context build plain 100: {}", format_duration(duration));
        }
        if let Some(duration) = baseline.context_build_plain_1000 {
            println!("context build plain 1000: {}", format_duration(duration));
        }
        if let Some(duration) = baseline.context_build_with_system_prompt {
            println!(
                "context build with system prompt: {}",
                format_duration(duration)
            );
        }
        if let Some(duration) = baseline.context_build_with_compaction {
            println!(
                "context build with compaction: {}",
                format_duration(duration)
            );
        }
        if let Some(duration) = baseline.context_build_with_image_parts {
            println!(
                "context build with image parts: {}",
                format_duration(duration)
            );
        }
        if let Some(duration) = baseline.provider_tool_attach_load {
            println!("provider tool attach/load: {}", format_duration(duration));
        }
        if let Some(duration) = baseline.fake_mcp_list_call {
            println!("fake mcp list/call: {}", format_duration(duration));
        }
        if let Some(loaded_messages) = baseline.loaded_messages {
            println!("active path messages: {loaded_messages}");
        }
        if let Some(tree_messages) = baseline.tree_messages {
            println!("tree messages: {tree_messages}");
        }
        if let Some(requested_tool_calls) = baseline.requested_tool_calls {
            println!("requested tool calls: {requested_tool_calls}");
        }
        if let Some(resolved_tool_results) = baseline.resolved_tool_results {
            println!("resolved tool results: {resolved_tool_results}");
        }
        if let Some(deleted_messages) = baseline.deleted_messages {
            println!("deleted messages: {deleted_messages}");
        }
        if let Some(promoted_children) = baseline.promoted_children {
            println!("promoted children: {promoted_children}");
        }
        if let Some(truncated_messages) = baseline.truncated_messages {
            println!("truncated messages: {truncated_messages}");
        }
        if let Some(duration) = baseline.gateway_ready {
            println!("gateway ready: {}", format_duration(duration));
        }
        if let Some(duration) = baseline.first_token {
            println!("first token: {}", format_duration(duration));
        }
        if let Some(duration) = baseline.full_response {
            println!("full response: {}", format_duration(duration));
        }
        if let Some(response_bytes) = baseline.response_bytes {
            println!("response bytes: {response_bytes}");
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
        println!("installed {}", report.target);
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

    /// Confirms that the conversation-level system prompt was set.
    pub fn set_system_prompt(&self, conversation_id: &ConversationId) {
        println!("set systemprompt {conversation_id}");
    }

    /// Confirms that the conversation default model was set.
    pub fn set_model(&self, conversation_id: &ConversationId, model: &ModelName) {
        println!("set model {conversation_id} {model}");
    }

    /// Confirms that the conversation-level system prompt was removed.
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
    pub fn activated_message(&self, message_id: &MessageId) {
        println!("activated message {message_id}");
    }

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

    /// Prints gateway lifecycle results.
    pub fn gateway_started(&self) {
        println!("gateway: started");
    }

    /// Prints gateway lifecycle results.
    pub fn gateway_already_running(&self) {
        println!("gateway: already running");
    }

    /// Prints gateway lifecycle results.
    pub fn gateway_stopped(&self) {
        println!("gateway: stopped");
    }

    /// Prints gateway lifecycle results.
    pub fn gateway_not_running(&self) {
        println!("gateway: not running");
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
    pub fn conversation_tree(&self, messages: &[Message], active_message_id: Option<&MessageId>) {
        for line in tree_lines(messages, active_message_id) {
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

    /// Prints the created run ID as machine-readable command output.
    pub fn created_run(&self, run_id: &RunId) {
        println!("{run_id}");
    }

    /// Prints one run's persisted lifecycle state.
    pub fn run_status(&self, run: &Run) {
        println!("run {}", run.id);
        println!("conversation: {}", run.conversation_id);
        println!(
            "start head: {}",
            run.start_head_message_id
                .as_ref()
                .map(MessageId::as_str)
                .unwrap_or("(empty)")
        );
        println!(
            "current head: {}",
            run.current_head_message_id
                .as_ref()
                .map(MessageId::as_str)
                .unwrap_or("(empty)")
        );
        println!("status: {}", run.status);
        println!("model: {}", run.model);
        if let Some(error) = run.error.as_ref() {
            println!("error: {error}");
        }
    }

    /// Prints a compact list of runtime runs.
    pub fn runs(&self, runs: &[Run]) {
        if runs.is_empty() {
            println!("no runs");
            return;
        }

        println!("runs");
        for run in runs {
            println!(
                "{}  {}  {}  {}",
                run.id,
                run.status,
                run.conversation_id,
                run.current_head_message_id
                    .as_ref()
                    .map(MessageId::as_str)
                    .unwrap_or("(empty)")
            );
        }
    }

    /// Prints pending run-owned approvals in a compact inspectable format.
    pub fn run_approvals(&self, approvals: &[crate::operation::RunToolApprovalRequest]) {
        if approvals.is_empty() {
            println!("no pending approvals");
            return;
        }

        println!("pending approvals");
        for approval in approvals {
            let tool_call = &approval.approval.tool_call;
            println!(
                "{}  {}  {}  {}  {}",
                approval.run_id,
                tool_call.id,
                tool_call.name(),
                approval.approval.reason,
                text_preview(tool_call.arguments())
            );
        }
    }

    /// Prints one persisted run event.
    pub fn run_event(&self, event: &RunEventRecord) {
        match &event.event {
            RunEvent::AssistantDelta { text } => print!("{text}"),
            RunEvent::ReasoningDelta { text } => print!("{text}"),
            RunEvent::ToolCallDelta {
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
            RunEvent::AssistantMessageSaved { message_id } => {
                println!("assistant message saved {message_id}");
            }
            RunEvent::ToolResultSaved { message_id } => {
                println!("tool result saved {message_id}");
            }
            RunEvent::WaitingForApproval => println!("waiting for approval"),
            RunEvent::Completed { message_id } => {
                println!("completed {}", message_id.as_deref().unwrap_or("(empty)"))
            }
            RunEvent::Failed { error, .. } => println!("failed {error}"),
            RunEvent::Cancelled => println!("cancelled"),
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

#[derive(Debug, Serialize)]
/// Machine-readable conversation list used by `windie ls --json`.
struct ConversationListReport {
    conversations: Vec<ConversationSummary>,
}

impl ConversationListReport {
    /// Converts store list rows into the public JSON list shape.
    fn new(conversations: &[ConversationInfo]) -> Self {
        Self {
            conversations: conversations
                .iter()
                .map(ConversationSummary::from_info)
                .collect(),
        }
    }
}

#[derive(Debug, Serialize)]
/// Serializable summary for one persisted conversation.
struct ConversationSummary {
    id: String,
    title: Option<String>,
    model: String,
    message_count: i64,
}

impl ConversationSummary {
    /// Copies the public conversation-list fields into JSON-safe strings.
    fn from_info(info: &ConversationInfo) -> Self {
        Self {
            id: info.id.as_str().to_string(),
            title: info.title.clone(),
            model: info.model.clone(),
            message_count: info.message_count,
        }
    }
}

/// Shared line printer for help and invalid usage output.
fn print_lines(lines: &[String]) {
    for line in lines {
        println!("{line}");
    }
}

/// Builds help text as data so output tests can assert exact lines.
fn help_lines() -> Vec<String> {
    vec![
        "windie",
        "",
        "Usage:",
        "  windie",
        "  windie api",
        "  windie inspector",
        "  windie install <target>",
        "  windie env KEY=value",
        "  windie env list",
        "  windie env unset <KEY>",
        "  windie env path",
        "  windie tools",
        "  windie models",
        "  windie new",
        "  windie ls",
        "  windie ls --json",
        "  windie activate <conversation_id> <message_id>",
        "  windie show <conversation_id>",
        "  windie tree <conversation_id>",
        "  windie inspect <conversation_id> --json",
        "  windie inspect <conversation_id> --json --model <provider/model>",
        "  windie attach <conversation_id> tool <provider_id> <tool_name>",
        "  windie detach <conversation_id> tool <schema_name>",
        "  windie insert <conversation_id> message --role user --text \"hello\"",
        "  windie insert <conversation_id> message --role user --text \"first\" --image <path> --text \"second\"",
        "  windie insert <conversation_id> toolschema --name <name> --description <text> --parameters <json>",
        "  windie update <conversation_id> message <message_id> --text \"new text\"",
        "  windie update <conversation_id> toolschema <name> --name <name> --description <text> --parameters <json>",
        "  windie set <conversation_id> systemprompt --text \"system prompt\"",
        "  windie set <conversation_id> model <provider/model>",
        "  windie rm <conversation_id>",
        "  windie rm <conversation_id> message <message_id>",
        "  windie rm <conversation_id> systemprompt",
        "  windie rm <conversation_id> toolschema <name>",
        "  windie truncate <conversation_id> <message_id>",
        "  windie fork <conversation_id> <message_id>",
        "  windie run start <conversation_id>",
        "  windie run start <conversation_id> --head <message_id>",
        "  windie run start <conversation_id> --model <provider/model>",
        "  windie run list",
        "  windie run list <conversation_id>",
        "  windie run status <run_id>",
        "  windie run events <run_id>",
        "  windie run approvals <run_id>",
        "  windie run approve <run_id> <tool_call_id>",
        "  windie run deny <run_id> <tool_call_id>",
        "  windie run stop <run_id>",
        "  windie status",
        "  windie gateway start",
        "  windie gateway stop",
        "  windie bench",
        "  windie bench --persistence --conversation --runtime --tools --mutations --mcp",
        "  windie bench --runs 100 --json",
        "  windie compare baseline",
        "  windie update baseline",
        "",
        "Notes:",
        "  windie exits successfully without runtime action.",
        "  windie api starts the localhost developer API server and prints the inspector URL.",
        "  windie inspector opens the browser inspector with the current API token.",
        "  windie install verifies or installs approved public runtime dependencies.",
        "  windie env edits only ~/.windie/.env and never prints secret values.",
        "  windie tools lists provider tools available to attach to conversations.",
        "  windie models lists models from the currently running Bifrost gateway.",
        "  windie bench measures provider-free local runtime primitives.",
        "  windie bench category flags filter the measured local benchmark report.",
        "  windie bench --json writes a persistent benchmark artifact to stdout.",
        "  windie compare baseline compares the current benchmark run with ~/.windie/benchmarks/baseline.json.",
        "  windie update baseline replaces ~/.windie/benchmarks/baseline.json with the current run.",
        "  windie inspect <conversation_id> --json prints full read-only runtime state.",
        "  windie gateway start starts local Bifrost, or public npx/Docker Bifrost.",
        "  windie gateway stop stops the local Bifrost gateway.",
        "  windie models requires the local Bifrost gateway to be running.",
        "  windie run start requires the local Bifrost gateway to be running.",
        "  windie run start uses the conversation model unless --model is passed for the run.",
        "  windie run approvals lists pending run-owned tool calls that require user approval.",
        "  windie run approve executes one pending run-owned tool call and continues the run.",
        "  windie run deny stores a rejected tool result and continues the run.",
        "  windie attach <conversation_id> tool attaches one provider tool to a conversation.",
        "  windie detach <conversation_id> tool detaches one provider tool schema from a conversation.",
        "  windie set <conversation_id> systemprompt sets or replaces the conversation system prompt.",
        "  windie set <conversation_id> model persists the conversation model.",
        "  windie insert <conversation_id> toolschema adds a raw model-facing tool definition.",
        "",
        "Options:",
        "  -h, --help       Show help",
        "  -V, --version    Show version",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

/// Converts provider tool definitions into compact CLI lines.
fn available_tool_lines(tools: &[ToolDefinition]) -> Vec<String> {
    if tools.is_empty() {
        return vec!["no tools".to_string()];
    }

    let mut lines = vec!["tools".to_string()];
    lines.extend(tools.iter().map(|tool| {
        format!(
            "{}/{}  {}  {}",
            tool.provider.provider_id, tool.provider.tool_name, tool.schema_name, tool.description
        )
    }));

    lines
}

/// Converts Bifrost model metadata into stable CLI lines.
fn model_lines(models: &[ModelInfo]) -> Vec<String> {
    if models.is_empty() {
        return vec!["no models".to_string()];
    }

    let mut ids = models
        .iter()
        .map(|model| model.id.as_str())
        .collect::<Vec<_>>();
    ids.sort_unstable();

    let mut lines = vec!["models".to_string()];
    lines.extend(ids.into_iter().map(str::to_string));

    lines
}

/// Converts a repeated benchmark report into stable human-readable lines.
fn performance_report_lines(report: &PerformanceReport) -> Vec<String> {
    let mut lines = vec![
        "performance report".to_string(),
        format!("mode: {}", report.mode.as_str()),
        format!("runs: {}", report.runs),
        format!("model: {}", report.model),
    ];

    if let Some(conversation_id) = report.conversation_id.as_ref() {
        lines.push(format!("conversation: {conversation_id}"));
    }

    push_metric_lines(&mut lines, "store open", report.summary.store_open.as_ref());
    push_metric_lines(
        &mut lines,
        "active path load",
        report.summary.active_path_load.as_ref(),
    );
    push_metric_lines(
        &mut lines,
        "active message lookup",
        report.summary.active_message_lookup.as_ref(),
    );
    push_metric_lines(
        &mut lines,
        "active path row load",
        report.summary.active_path_row_load.as_ref(),
    );
    push_metric_lines(
        &mut lines,
        "active path part/image load",
        report.summary.active_path_part_load.as_ref(),
    );
    push_metric_lines(&mut lines, "tree load", report.summary.tree_load.as_ref());
    push_metric_lines(
        &mut lines,
        "tree row load",
        report.summary.tree_row_load.as_ref(),
    );
    push_metric_lines(
        &mut lines,
        "tree part/image load",
        report.summary.tree_part_load.as_ref(),
    );
    push_metric_lines(
        &mut lines,
        "tool schema load",
        report.summary.tool_schema_load.as_ref(),
    );
    push_metric_lines(
        &mut lines,
        "context build",
        report.summary.context_build.as_ref(),
    );
    push_metric_lines(
        &mut lines,
        "context active path load",
        report.summary.context_active_path_load.as_ref(),
    );
    push_metric_lines(
        &mut lines,
        "context system prompt load",
        report.summary.context_system_prompt_load.as_ref(),
    );
    push_metric_lines(
        &mut lines,
        "context compaction load",
        report.summary.context_compaction_load.as_ref(),
    );
    push_metric_lines(
        &mut lines,
        "context flatten",
        report.summary.context_flatten.as_ref(),
    );
    push_metric_lines(
        &mut lines,
        "prepare run head turn",
        report.summary.prepare_run_head_turn.as_ref(),
    );
    push_metric_lines(
        &mut lines,
        "pending tool approval scan",
        report.summary.pending_tool_approval_scan.as_ref(),
    );
    push_metric_lines(
        &mut lines,
        "tool result insert",
        report.summary.tool_result_insert.as_ref(),
    );
    push_metric_lines(
        &mut lines,
        "deny tool result persist",
        report.summary.deny_tool_result_persist.as_ref(),
    );
    push_metric_lines(
        &mut lines,
        "splice remove",
        report.summary.splice_remove.as_ref(),
    );
    push_metric_lines(&mut lines, "truncate", report.summary.truncate.as_ref());
    push_metric_lines(
        &mut lines,
        "context build after tool chain",
        report.summary.context_build_after_tool_chain.as_ref(),
    );
    push_metric_lines(
        &mut lines,
        "active path load 100",
        report.summary.active_path_load_100.as_ref(),
    );
    push_metric_lines(
        &mut lines,
        "active path load 1000",
        report.summary.active_path_load_1000.as_ref(),
    );
    push_metric_lines(
        &mut lines,
        "pending tool approval scan long path",
        report.summary.pending_tool_approval_scan_long_path.as_ref(),
    );
    push_metric_lines(
        &mut lines,
        "pending tool approval scan deep chain",
        report
            .summary
            .pending_tool_approval_scan_deep_chain
            .as_ref(),
    );
    push_metric_lines(
        &mut lines,
        "prepare query no tools",
        report.summary.prepare_run_head_no_tools.as_ref(),
    );
    push_metric_lines(
        &mut lines,
        "prepare query completed tool chain",
        report
            .summary
            .prepare_run_head_completed_tool_chain
            .as_ref(),
    );
    push_metric_lines(
        &mut lines,
        "prepare query requires approval",
        report.summary.prepare_run_head_requires_approval.as_ref(),
    );
    push_metric_lines(
        &mut lines,
        "prepare query policy denied",
        report.summary.prepare_run_head_policy_denied.as_ref(),
    );
    push_metric_lines(
        &mut lines,
        "splice remove branch point",
        report.summary.splice_remove_branch_point.as_ref(),
    );
    push_metric_lines(
        &mut lines,
        "splice remove root many children",
        report.summary.splice_remove_root_many_children.as_ref(),
    );
    push_metric_lines(
        &mut lines,
        "splice remove tool group",
        report.summary.splice_remove_tool_group.as_ref(),
    );
    push_metric_lines(
        &mut lines,
        "truncate large subtree",
        report.summary.truncate_large_subtree.as_ref(),
    );
    push_metric_lines(
        &mut lines,
        "context build plain 100",
        report.summary.context_build_plain_100.as_ref(),
    );
    push_metric_lines(
        &mut lines,
        "context build plain 1000",
        report.summary.context_build_plain_1000.as_ref(),
    );
    push_metric_lines(
        &mut lines,
        "context build with system prompt",
        report.summary.context_build_with_system_prompt.as_ref(),
    );
    push_metric_lines(
        &mut lines,
        "context build with compaction",
        report.summary.context_build_with_compaction.as_ref(),
    );
    push_metric_lines(
        &mut lines,
        "context build with image parts",
        report.summary.context_build_with_image_parts.as_ref(),
    );
    push_metric_lines(
        &mut lines,
        "provider tool attach/load",
        report.summary.provider_tool_attach_load.as_ref(),
    );
    push_metric_lines(
        &mut lines,
        "fake mcp list/call",
        report.summary.fake_mcp_list_call.as_ref(),
    );
    push_metric_lines(
        &mut lines,
        "gateway ready",
        report.summary.gateway_ready.as_ref(),
    );
    push_metric_lines(
        &mut lines,
        "first token",
        report.summary.first_token.as_ref(),
    );
    push_metric_lines(
        &mut lines,
        "full response",
        report.summary.full_response.as_ref(),
    );

    lines
}

/// Appends min/median/p95/max lines for one benchmark metric.
fn push_metric_lines(lines: &mut Vec<String>, name: &str, metric: Option<&DurationMetric>) {
    let Some(metric) = metric else {
        return;
    };

    lines.push(format!("{name}:"));
    lines.push(format!("  min: {}", format_duration_us(metric.min_us)));
    lines.push(format!(
        "  median: {}",
        format_duration_us(metric.median_us)
    ));
    lines.push(format!("  p95: {}", format_duration_us(metric.p95_us)));
    lines.push(format!("  max: {}", format_duration_us(metric.max_us)));
}

/// Converts a persisted benchmark comparison into stable CLI lines.
fn performance_comparison_lines(comparison: &PerformanceComparison) -> Vec<String> {
    let mut lines = vec![
        "performance comparison".to_string(),
        format!(
            "baseline: {} ({} runs)",
            comparison.baseline_mode.as_str(),
            comparison.baseline_runs
        ),
        format!(
            "current: {} ({} runs)",
            comparison.current_mode.as_str(),
            comparison.current_runs
        ),
    ];

    if comparison.rows.is_empty() {
        lines.push("no comparable metrics".to_string());
        return lines;
    }

    for row in &comparison.rows {
        lines.push(format!(
            "{}: {} -> {} ({:+.1}%)",
            row.name,
            format_duration_us(row.baseline_median_us),
            format_duration_us(row.current_median_us),
            row.change_percent
        ));
    }

    lines
}

/// Builds invalid usage text from help so both outputs stay in sync.
fn invalid_usage_lines() -> Vec<String> {
    let mut lines = vec!["invalid usage".to_string(), String::new()];
    lines.extend(help_lines());
    lines
}

/// Humanizes a message count for the conversation list.
fn message_count(count: i64) -> String {
    if count == 1 {
        "1 message".to_string()
    } else {
        format!("{count} messages")
    }
}

/// Converts conversation summaries into stable CLI lines.
fn conversation_lines(conversations: &[ConversationInfo]) -> Vec<String> {
    if conversations.is_empty() {
        return vec!["no conversations".to_string()];
    }

    let mut lines = vec!["conversations".to_string()];

    for conversation in conversations {
        let count = message_count(conversation.message_count);

        if let Some(title) = conversation
            .title
            .as_deref()
            .filter(|title| !title.is_empty())
        {
            lines.push(format!("{}  {count}  {title}", conversation.id));
        } else {
            lines.push(format!("{}  {count}", conversation.id));
        }
    }

    lines
}

/// Converts stored messages into stable one-line previews.
fn message_lines(messages: &[Message]) -> Vec<String> {
    if messages.is_empty() {
        return vec!["no messages".to_string()];
    }

    let mut lines = vec!["messages".to_string()];

    for message in messages {
        let id = message
            .id
            .as_ref()
            .map(|id| id.as_str())
            .unwrap_or("<unsaved>");
        lines.push(format!(
            "{}  {}  {}",
            message.role.as_str(),
            id,
            message_preview(message)
        ));
    }

    lines
}

/// Converts a full message tree into indented CLI lines.
fn tree_lines(messages: &[Message], active_message_id: Option<&MessageId>) -> Vec<String> {
    if messages.is_empty() {
        return vec!["no messages".to_string()];
    }

    let mut children_by_parent = HashMap::<Option<String>, Vec<&Message>>::new();
    for message in messages {
        let parent_key = message
            .parent_message_id
            .as_ref()
            .map(|message_id| message_id.as_str().to_string());
        children_by_parent
            .entry(parent_key)
            .or_default()
            .push(message);
    }

    let mut lines = vec!["tree".to_string()];
    append_tree_lines(&mut lines, &children_by_parent, None, active_message_id, 0);

    lines
}

/// Recursively appends indented tree lines under one parent message.
fn append_tree_lines(
    lines: &mut Vec<String>,
    children_by_parent: &HashMap<Option<String>, Vec<&Message>>,
    parent_id: Option<&str>,
    active_message_id: Option<&MessageId>,
    depth: usize,
) {
    let parent_key = parent_id.map(str::to_string);
    let Some(children) = children_by_parent.get(&parent_key) else {
        return;
    };

    for message in children {
        let id = message
            .id
            .as_ref()
            .map(|id| id.as_str())
            .unwrap_or("<unsaved>");
        let active_marker =
            if active_message_id.is_some_and(|active_id| Some(active_id) == message.id.as_ref()) {
                "*"
            } else {
                " "
            };
        lines.push(format!(
            "{}{} {}  {}  {}",
            "  ".repeat(depth),
            active_marker,
            message.role.as_str(),
            id,
            message_preview(message)
        ));
        append_tree_lines(
            lines,
            children_by_parent,
            message.id.as_ref().map(MessageId::as_str),
            active_message_id,
            depth + 1,
        );
    }
}

/// Normalizes one message into a compact, Unicode-safe preview.
fn message_preview(message: &Message) -> String {
    let text = text_preview(&message.content);
    let image_count = message
        .parts
        .iter()
        .filter(|part| matches!(part, MessagePart::Image(_)))
        .count();
    let preview = match (text.is_empty(), image_count) {
        (true, 0) => String::new(),
        (true, 1) => "[image]".to_string(),
        (true, count) => format!("[{count} images]"),
        (false, 0) => text,
        (false, 1) => format!("{text} [image]"),
        (false, count) => format!("{text} [{count} images]"),
    };

    truncate_preview(&preview)
}

/// Normalizes text into a compact, Unicode-safe preview.
fn text_preview(content: &str) -> String {
    let preview = content.split_whitespace().collect::<Vec<_>>().join(" ");

    truncate_preview(&preview)
}

/// Truncates preview text to the terminal display limit.
fn truncate_preview(preview: &str) -> String {
    let truncated = preview.chars().take(80).collect::<String>();
    if truncated.len() == preview.len() {
        return preview.to_string();
    }

    format!("{truncated}...")
}

/// Formats durations for human scanning in benchmark output.
fn format_duration(duration: std::time::Duration) -> String {
    if duration.as_secs() > 0 {
        format!("{:.2}s", duration.as_secs_f64())
    } else if duration.as_millis() > 0 {
        format!("{}ms", duration.as_millis())
    } else {
        format!("{}us", duration.as_micros())
    }
}

/// Builds the local inspector URL with one query-encoded token.
fn inspector_url(api_token: &str) -> String {
    format!(
        "http://localhost:3000?windie_token={}",
        encode_query_value(api_token)
    )
}

/// Percent-encodes one URL query value without adding another dependency.
fn encode_query_value(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                encoded.push(byte as char);
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }

    encoded
}

/// Formats stored microsecond metrics through the same human-readable duration
/// style as live `Duration` values.
fn format_duration_us(micros: u64) -> String {
    format_duration(std::time::Duration::from_micros(micros))
}

#[cfg(test)]
#[path = "output_tests.rs"]
mod tests;
