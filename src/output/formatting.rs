//! Output formatting helpers.
//!
//! This module converts runtime and persistence data into stable terminal lines
//! and machine-readable report shapes.

use std::collections::HashMap;

use serde::Serialize;

use crate::conversation::{Message, MessageId, MessagePart};
use crate::llm::ModelInfo;
use crate::perf::{DurationMetric, PerformanceComparison, PerformanceReport};
use crate::store::ConversationInfo;
use crate::tool::ToolDefinition;

#[derive(Debug, Serialize)]
/// Machine-readable conversation list used by `windie ls --json`.
pub(crate) struct ConversationListReport {
    conversations: Vec<ConversationSummary>,
}

impl ConversationListReport {
    /// Converts store list rows into the public JSON list shape.
    pub(crate) fn new(conversations: &[ConversationInfo]) -> Self {
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
pub(crate) fn print_lines(lines: &[String]) {
    for line in lines {
        println!("{line}");
    }
}

/// Builds help text as data so output tests can assert exact lines.
pub(crate) fn help_lines() -> Vec<String> {
    vec![
        "windie",
        "",
        "Usage:",
        "  windie",
        "  windie api",
        "  windie inspector",
        "  windie onboard",
        "  windie install <target>",
        "  windie env MCP_KEY=value",
        "  windie env list",
        "  windie env unset <KEY>",
        "  windie env path",
        "  windie tools",
        "  windie models",
        "  windie new",
        "  windie ls",
        "  windie ls --json",
        "  windie show <conversation_id>",
        "  windie tree <conversation_id>",
        "  windie inspect <conversation_id> --json",
        "  windie inspect <conversation_id> --json --head <message_id>",
        "  windie inspect <conversation_id> --json --model <provider/model>",
        "  windie inspect <conversation_id> --json --head <message_id> --model <provider/model>",
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
        "  windie run status <session_id>",
        "  windie session events <session_id>",
        "  windie run approvals <session_id>",
        "  windie run approve <session_id> <tool_call_id>",
        "  windie run deny <session_id> <tool_call_id>",
        "  windie run stop <session_id>",
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
        "  windie onboard configures Bifrost providers and Windie MCP extensions through stdin.",
        "  windie install verifies or installs approved public runtime dependencies.",
        "  windie env edits only ~/.windie/.env and never prints secret values.",
        "  windie tools lists provider tools available to attach to conversations.",
        "  windie models lists models from the currently running Bifrost gateway.",
        "  windie bench measures provider-free local runtime primitives.",
        "  windie bench category flags filter the measured local benchmark report.",
        "  windie bench --json writes a persistent benchmark artifact to stdout.",
        "  windie compare baseline compares the current benchmark run with ~/.windie/benchmarks/baseline.json.",
        "  windie update baseline replaces ~/.windie/benchmarks/baseline.json with the current session.",
        "  windie inspect <conversation_id> --json prints full read-only runtime state.",
        "  windie gateway start starts the Windie-owned local Bifrost binary.",
        "  windie gateway stop stops the local Bifrost gateway.",
        "  windie models requires the local Bifrost gateway to be running.",
        "  windie run start requires the local Bifrost gateway to be running.",
        "  windie run start uses the conversation model unless --model is passed for the session.",
        "  windie run approvals lists pending session-owned tool calls that require user approval.",
        "  windie run approve executes one pending session-owned tool call and continues the session.",
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
pub(crate) fn available_tool_lines(tools: &[ToolDefinition]) -> Vec<String> {
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
pub(crate) fn model_lines(models: &[ModelInfo]) -> Vec<String> {
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
pub(crate) fn performance_report_lines(report: &PerformanceReport) -> Vec<String> {
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
    push_metric_lines(&mut lines, "path load", report.summary.path_load.as_ref());
    push_metric_lines(
        &mut lines,
        "head message lookup",
        report.summary.head_message_lookup.as_ref(),
    );
    push_metric_lines(
        &mut lines,
        "path row load",
        report.summary.path_row_load.as_ref(),
    );
    push_metric_lines(
        &mut lines,
        "path part/image load",
        report.summary.path_part_load.as_ref(),
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
        "context path load",
        report.summary.context_path_load.as_ref(),
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
        report.summary.prepare_head_turn.as_ref(),
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
        "path load 100",
        report.summary.path_load_100.as_ref(),
    );
    push_metric_lines(
        &mut lines,
        "path load 1000",
        report.summary.path_load_1000.as_ref(),
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
pub(super) fn push_metric_lines(
    lines: &mut Vec<String>,
    name: &str,
    metric: Option<&DurationMetric>,
) {
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
pub(crate) fn performance_comparison_lines(comparison: &PerformanceComparison) -> Vec<String> {
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
pub(crate) fn invalid_usage_lines() -> Vec<String> {
    let mut lines = vec!["invalid usage".to_string(), String::new()];
    lines.extend(help_lines());
    lines
}

/// Humanizes a message count for the conversation list.
pub(super) fn message_count(count: i64) -> String {
    if count == 1 {
        "1 message".to_string()
    } else {
        format!("{count} messages")
    }
}

/// Converts conversation summaries into stable CLI lines.
pub(crate) fn conversation_lines(conversations: &[ConversationInfo]) -> Vec<String> {
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
pub(crate) fn message_lines(messages: &[Message]) -> Vec<String> {
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
pub(crate) fn tree_lines(messages: &[Message]) -> Vec<String> {
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
    append_tree_lines(&mut lines, &children_by_parent, None, 0);

    lines
}

/// Recursively appends indented tree lines under one parent message.
pub(super) fn append_tree_lines(
    lines: &mut Vec<String>,
    children_by_parent: &HashMap<Option<String>, Vec<&Message>>,
    parent_id: Option<&str>,
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
        lines.push(format!(
            "{}{}  {}  {}",
            "  ".repeat(depth),
            message.role.as_str(),
            id,
            message_preview(message)
        ));
        append_tree_lines(
            lines,
            children_by_parent,
            message.id.as_ref().map(MessageId::as_str),
            depth + 1,
        );
    }
}

/// Normalizes one message into a compact, Unicode-safe preview.
pub(super) fn message_preview(message: &Message) -> String {
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
pub(crate) fn text_preview(content: &str) -> String {
    let preview = content.split_whitespace().collect::<Vec<_>>().join(" ");

    truncate_preview(&preview)
}

/// Truncates preview text to the terminal display limit.
pub(super) fn truncate_preview(preview: &str) -> String {
    let truncated = preview.chars().take(80).collect::<String>();
    if truncated.len() == preview.len() {
        return preview.to_string();
    }

    format!("{truncated}...")
}

/// Formats durations for human scanning in benchmark output.
pub(crate) fn format_duration(duration: std::time::Duration) -> String {
    if duration.as_secs() > 0 {
        format!("{:.2}s", duration.as_secs_f64())
    } else if duration.as_millis() > 0 {
        format!("{}ms", duration.as_millis())
    } else {
        format!("{}us", duration.as_micros())
    }
}

/// Percent-encodes one URL query value without adding another dependency.
pub(crate) fn encode_query_value(value: &str) -> String {
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
pub(super) fn format_duration_us(micros: u64) -> String {
    format_duration(std::time::Duration::from_micros(micros))
}
