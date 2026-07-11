//! Terminal output boundary.
//!
//! This module owns CLI printing for assistant streams and command output.
//! Other modules should pass display data here instead of formatting terminal
//! output themselves.
use std::collections::HashMap;
use std::io::{self, Write};
use std::net::SocketAddr;

use anyhow::{Context, Result};
use serde::Serialize;

use crate::conversation::{
    ConversationId, Message, MessageId, MessagePart, ToolCall, ToolSchemaName,
};
use crate::doctor::DoctorReport;
use crate::llm::{ModelInfo, ModelName};
use crate::operation::InspectionReport;
use crate::perf::{DurationMetric, PerformanceBaseline, PerformanceComparison, PerformanceReport};
use crate::store::ConversationInfo;
use crate::tool::{ToolApprovalRequest, ToolDefinition, ToolExecutionResult};

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

    /// Prints stable paths and install guidance without mutating the machine.
    pub fn doctor(&self, report: &DoctorReport) {
        println!("windie doctor");
        println!("executable: {}", report.executable.display());
        println!("data: {}", report.data_dir.display());
        println!("config: {}", report.config_dir.display());
        println!("provider env: {}", report.provider_env.display());
        println!("integrations");
        for integration in &report.integrations {
            println!(
                "  {}: {}",
                integration.name,
                if integration.available {
                    "ready"
                } else {
                    "missing prerequisite"
                }
            );
            println!("    runtime: {}", integration.runtime);
            if !integration.available {
                println!("    setup: {}", integration.install);
            }
        }
    }

    /// Prints the local API address and generated access token at startup.
    pub fn api_started(&self, address: &SocketAddr, api_token: &str) {
        println!("windie api listening on http://{address}");
        println!("windie api token: {api_token}");
        println!("windie operator UI: http://{address}/?windie_token={api_token}");
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
        for (name, duration) in baseline.durations() {
            println!("{}: {}", name.label(), format_duration(duration));
        }
        for (name, count) in baseline.counts() {
            println!("{}: {count}", name.label());
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

    /// Prints pending tool approvals in a compact inspectable format.
    pub fn tool_approvals(&self, approvals: &[ToolApprovalRequest]) {
        if approvals.is_empty() {
            println!("no pending approvals");
            return;
        }

        println!("pending approvals");
        for approval in approvals {
            println!(
                "{}  {}  {}  {}",
                approval.tool_call.id,
                approval.tool_call.name(),
                approval.reason,
                text_preview(approval.tool_call.arguments())
            );
        }
    }

    /// Prints one completed tool execution result summary.
    pub fn tool_execution_result(&self, result: &ToolExecutionResult) {
        println!(
            "tool result  {}  {}  {}",
            result.tool_call_id,
            result.tool_name,
            if result.success { "success" } else { "error" }
        );
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
    model: String,
    message_count: i64,
}

impl ConversationSummary {
    /// Copies the public conversation-list fields into JSON-safe strings.
    fn from_info(info: &ConversationInfo) -> Self {
        Self {
            id: info.id.as_str().to_string(),
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
        "  windie doctor",
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
        "  windie approvals <conversation_id>",
        "  windie approve <conversation_id> <tool_call_id>",
        "  windie deny <conversation_id> <tool_call_id>",
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
        "  windie query <conversation_id>",
        "  windie query <conversation_id> --model <provider/model>",
        "  windie status",
        "  windie gateway start",
        "  windie gateway stop",
        "  windie bench <conversation_id>",
        "  windie bench <conversation_id> --runs 100 --json",
        "  windie bench runtime",
        "  windie bench compare <baseline.json> <current.json>",
        "  windie bench live",
        "",
        "Notes:",
        "  windie exits successfully without runtime action.",
        "  windie api starts the localhost developer API server.",
        "  windie doctor inspects paths and optional integration prerequisites.",
        "  windie tools lists provider tools available to attach to conversations.",
        "  windie models lists models from the currently running Bifrost gateway.",
        "  windie bench <conversation_id> measures active path, tree, tool schema load, and context build.",
        "  windie bench runtime measures provider-free runtime and write-path primitives.",
        "  windie bench <conversation_id> --json writes a persistent benchmark artifact to stdout.",
        "  windie bench compare compares two JSON benchmark artifacts.",
        "  windie inspect <conversation_id> --json prints full read-only runtime state.",
        "  windie gateway start starts local Bifrost, or public npx/Docker Bifrost.",
        "  windie gateway stop stops the local Bifrost gateway.",
        "  windie models requires the local Bifrost gateway to be running.",
        "  windie query requires the local Bifrost gateway to be running.",
        "  windie query uses the conversation model unless --model is passed for one request.",
        "  windie approvals lists pending tool calls that require user approval.",
        "  windie approve executes one pending tool call and stores its tool result.",
        "  windie deny stores a rejected tool result for one pending tool call.",
        "  windie attach <conversation_id> tool attaches one provider tool to a conversation.",
        "  windie detach <conversation_id> tool detaches one provider tool schema from a conversation.",
        "  windie set <conversation_id> systemprompt sets or replaces the conversation system prompt.",
        "  windie set <conversation_id> model persists the conversation model.",
        "  windie insert <conversation_id> toolschema adds a raw model-facing tool definition.",
        "  windie bench live sends a real provider request and may cost money.",
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

    for (name, metric) in report.summary.metrics() {
        push_metric_lines(&mut lines, name.label(), metric);
    }

    lines
}

/// Appends min/median/p95/max lines for one benchmark metric.
fn push_metric_lines(lines: &mut Vec<String>, name: &str, metric: &DurationMetric) {
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
        lines.push(format!("{}  {count}", conversation.id));
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

/// Formats stored microsecond metrics through the same human-readable duration
/// style as live `Duration` values.
fn format_duration_us(micros: u64) -> String {
    format_duration(std::time::Duration::from_micros(micros))
}

#[cfg(test)]
#[path = "output_tests.rs"]
mod tests;
