//! Performance measurement and comparison.
//!
//! This module owns lightweight timing for the current local CLI/query path,
//! repeated benchmark reports, JSON benchmark artifacts, and report comparison.
//! Conversation benchmarks avoid provider calls. Live benchmarks are explicit
//! because they send a real provider request.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::context::{ContextBuilder, ContextParts};
use crate::conversation::{
    ConversationId, Message, MessageId, MessageMetadata, Role, ToolCall, ToolCallId,
    UnsavedImagePart, UnsavedMessagePart,
};
use crate::gateway::{BifrostGateway, GatewayUrl};
use crate::llm::{BaseUrl, BifrostClient, ModelName};
use crate::mcp::{self, McpCommand};
use crate::runtime::{deny_tool_call, pending_tool_approvals, prepare_query_turn};
use crate::store::Store;
use crate::tool::{
    ProviderToolName, ToolAnnotations, ToolDefinition, ToolPermission, ToolProviderId,
    ToolProviderKind, ToolProviderRef,
};

const BENCH_PROMPT: &str = "Reply with exactly: ok";
const SCALE_PATH_MESSAGES: usize = 100;
const LARGE_SCALE_PATH_MESSAGES: usize = 1_000;
const TOOL_CHAIN_RESULTS: usize = 10;
const BRANCH_CHILDREN: usize = 100;
const LARGE_TRUNCATE_DESCENDANTS: usize = 100;
const IMAGE_PART_MESSAGES: usize = 10;
const TEST_PROVIDER_ID: &str = "desktop-commander";
const TEST_PROVIDER_TOOL_NAME: &str = "read_file";
const TEST_TOOL_SCHEMA_NAME: &str = "desktop_commander__read_file";
const FAKE_MCP_SCRIPT: &str = r#"while IFS= read -r line; do
case "$line" in
*'"method":"initialize"'*) printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-06-18","capabilities":{},"serverInfo":{"name":"windie-fake-mcp","version":"0"}}}' ;;
*'"method":"tools/list"'*) printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"click","description":"Fake click","inputSchema":{"type":"object","additionalProperties":false,"properties":{}}}]}}' ;;
*'"method":"tools/call"'*) printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"content":[{"type":"text","text":"ok"}],"isError":false}}' ;;
esac
done"#;
const FAKE_MCP_COMMAND: McpCommand = McpCommand {
    program: "/bin/sh",
    args: &["-c", FAKE_MCP_SCRIPT],
    env: &[],
};

const REPORT_FORMAT_VERSION: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
/// Benchmark mode selected by the CLI.
pub enum BenchmarkMode {
    Conversation,
    Runtime,
    Live,
}

impl BenchmarkMode {
    /// Returns the mode label printed in benchmark output.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Conversation => "conversation",
            Self::Runtime => "runtime",
            Self::Live => "live",
        }
    }

    /// Marks benchmark modes that may send a paid provider request.
    pub fn may_call_provider(self) -> bool {
        matches!(self, Self::Live)
    }
}

/// Optional controls for benchmark execution and output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BenchmarkOptions {
    pub runs: usize,
    pub json: bool,
}

impl Default for BenchmarkOptions {
    /// Defaults to one human-readable run to preserve the simple benchmark
    /// behavior.
    fn default() -> Self {
        Self {
            runs: 1,
            json: false,
        }
    }
}

/// Timings collected by one benchmark run.
///
/// Fields are optional because each benchmark mode measures a different path.
pub struct PerformanceBaseline {
    pub mode: BenchmarkMode,
    pub model: ModelName,
    pub conversation_id: Option<ConversationId>,
    pub store_open: Option<Duration>,
    pub conversation_load: Option<Duration>,
    pub active_message_lookup: Option<Duration>,
    pub active_path_row_load: Option<Duration>,
    pub active_path_part_load: Option<Duration>,
    pub tree_load: Option<Duration>,
    pub tree_row_load: Option<Duration>,
    pub tree_part_load: Option<Duration>,
    pub tool_schema_load: Option<Duration>,
    pub context_build: Option<Duration>,
    pub context_active_path_load: Option<Duration>,
    pub context_system_prompt_load: Option<Duration>,
    pub context_compaction_load: Option<Duration>,
    pub context_flatten: Option<Duration>,
    pub prepare_query_turn: Option<Duration>,
    pub pending_tool_approval_scan: Option<Duration>,
    pub tool_result_insert: Option<Duration>,
    pub deny_tool_result_persist: Option<Duration>,
    pub splice_remove: Option<Duration>,
    pub truncate: Option<Duration>,
    pub context_build_after_tool_chain: Option<Duration>,
    pub active_path_load_100: Option<Duration>,
    pub active_path_load_1000: Option<Duration>,
    pub pending_tool_approval_scan_long_path: Option<Duration>,
    pub pending_tool_approval_scan_deep_chain: Option<Duration>,
    pub prepare_query_no_tools: Option<Duration>,
    pub prepare_query_completed_tool_chain: Option<Duration>,
    pub prepare_query_requires_approval: Option<Duration>,
    pub prepare_query_policy_denied: Option<Duration>,
    pub splice_remove_branch_point: Option<Duration>,
    pub splice_remove_root_many_children: Option<Duration>,
    pub splice_remove_tool_group: Option<Duration>,
    pub truncate_large_subtree: Option<Duration>,
    pub context_build_plain_100: Option<Duration>,
    pub context_build_plain_1000: Option<Duration>,
    pub context_build_with_system_prompt: Option<Duration>,
    pub context_build_with_compaction: Option<Duration>,
    pub context_build_with_image_parts: Option<Duration>,
    pub provider_tool_attach_load: Option<Duration>,
    pub fake_mcp_list_call: Option<Duration>,
    pub loaded_messages: Option<usize>,
    pub tree_messages: Option<usize>,
    pub requested_tool_calls: Option<usize>,
    pub resolved_tool_results: Option<usize>,
    pub deleted_messages: Option<usize>,
    pub promoted_children: Option<usize>,
    pub truncated_messages: Option<usize>,
    pub gateway_ready: Option<Duration>,
    pub first_token: Option<Duration>,
    pub full_response: Option<Duration>,
    pub response_bytes: Option<usize>,
}

/// Persistent benchmark artifact written by `windie bench --json`.
///
/// Durations are stored as integer microseconds so JSON stays stable and does
/// not depend on Rust's debug formatting for `Duration`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceReport {
    pub format_version: u32,
    pub mode: BenchmarkMode,
    pub model: String,
    pub conversation_id: Option<String>,
    pub runs: usize,
    pub samples: Vec<PerformanceSample>,
    pub summary: PerformanceSummary,
}

/// One serialized benchmark sample.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PerformanceSample {
    pub store_open_us: Option<u64>,
    pub active_path_load_us: Option<u64>,
    pub active_message_lookup_us: Option<u64>,
    pub active_path_row_load_us: Option<u64>,
    pub active_path_part_load_us: Option<u64>,
    pub tree_load_us: Option<u64>,
    pub tree_row_load_us: Option<u64>,
    pub tree_part_load_us: Option<u64>,
    #[serde(default)]
    pub tool_schema_load_us: Option<u64>,
    pub context_build_us: Option<u64>,
    pub context_active_path_load_us: Option<u64>,
    pub context_system_prompt_load_us: Option<u64>,
    pub context_compaction_load_us: Option<u64>,
    pub context_flatten_us: Option<u64>,
    #[serde(default)]
    pub prepare_query_turn_us: Option<u64>,
    #[serde(default)]
    pub pending_tool_approval_scan_us: Option<u64>,
    #[serde(default)]
    pub tool_result_insert_us: Option<u64>,
    #[serde(default)]
    pub deny_tool_result_persist_us: Option<u64>,
    #[serde(default)]
    pub splice_remove_us: Option<u64>,
    #[serde(default)]
    pub truncate_us: Option<u64>,
    #[serde(default)]
    pub context_build_after_tool_chain_us: Option<u64>,
    #[serde(default)]
    pub active_path_load_100_us: Option<u64>,
    #[serde(default)]
    pub active_path_load_1000_us: Option<u64>,
    #[serde(default)]
    pub pending_tool_approval_scan_long_path_us: Option<u64>,
    #[serde(default)]
    pub pending_tool_approval_scan_deep_chain_us: Option<u64>,
    #[serde(default)]
    pub prepare_query_no_tools_us: Option<u64>,
    #[serde(default)]
    pub prepare_query_completed_tool_chain_us: Option<u64>,
    #[serde(default)]
    pub prepare_query_requires_approval_us: Option<u64>,
    #[serde(default)]
    pub prepare_query_policy_denied_us: Option<u64>,
    #[serde(default)]
    pub splice_remove_branch_point_us: Option<u64>,
    #[serde(default)]
    pub splice_remove_root_many_children_us: Option<u64>,
    #[serde(default)]
    pub splice_remove_tool_group_us: Option<u64>,
    #[serde(default)]
    pub truncate_large_subtree_us: Option<u64>,
    #[serde(default)]
    pub context_build_plain_100_us: Option<u64>,
    #[serde(default)]
    pub context_build_plain_1000_us: Option<u64>,
    #[serde(default)]
    pub context_build_with_system_prompt_us: Option<u64>,
    #[serde(default)]
    pub context_build_with_compaction_us: Option<u64>,
    #[serde(default)]
    pub context_build_with_image_parts_us: Option<u64>,
    #[serde(default)]
    pub provider_tool_attach_load_us: Option<u64>,
    #[serde(default)]
    pub fake_mcp_list_call_us: Option<u64>,
    pub active_path_messages: Option<usize>,
    pub tree_messages: Option<usize>,
    #[serde(default)]
    pub requested_tool_calls: Option<usize>,
    #[serde(default)]
    pub resolved_tool_results: Option<usize>,
    #[serde(default)]
    pub deleted_messages: Option<usize>,
    #[serde(default)]
    pub promoted_children: Option<usize>,
    #[serde(default)]
    pub truncated_messages: Option<usize>,
    pub gateway_ready_us: Option<u64>,
    pub first_token_us: Option<u64>,
    pub full_response_us: Option<u64>,
    pub response_bytes: Option<usize>,
}

/// Aggregated duration metrics across all benchmark samples.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PerformanceSummary {
    pub store_open: Option<DurationMetric>,
    pub active_path_load: Option<DurationMetric>,
    pub active_message_lookup: Option<DurationMetric>,
    pub active_path_row_load: Option<DurationMetric>,
    pub active_path_part_load: Option<DurationMetric>,
    pub tree_load: Option<DurationMetric>,
    pub tree_row_load: Option<DurationMetric>,
    pub tree_part_load: Option<DurationMetric>,
    #[serde(default)]
    pub tool_schema_load: Option<DurationMetric>,
    pub context_build: Option<DurationMetric>,
    pub context_active_path_load: Option<DurationMetric>,
    pub context_system_prompt_load: Option<DurationMetric>,
    pub context_compaction_load: Option<DurationMetric>,
    pub context_flatten: Option<DurationMetric>,
    #[serde(default)]
    pub prepare_query_turn: Option<DurationMetric>,
    #[serde(default)]
    pub pending_tool_approval_scan: Option<DurationMetric>,
    #[serde(default)]
    pub tool_result_insert: Option<DurationMetric>,
    #[serde(default)]
    pub deny_tool_result_persist: Option<DurationMetric>,
    #[serde(default)]
    pub splice_remove: Option<DurationMetric>,
    #[serde(default)]
    pub truncate: Option<DurationMetric>,
    #[serde(default)]
    pub context_build_after_tool_chain: Option<DurationMetric>,
    #[serde(default)]
    pub active_path_load_100: Option<DurationMetric>,
    #[serde(default)]
    pub active_path_load_1000: Option<DurationMetric>,
    #[serde(default)]
    pub pending_tool_approval_scan_long_path: Option<DurationMetric>,
    #[serde(default)]
    pub pending_tool_approval_scan_deep_chain: Option<DurationMetric>,
    #[serde(default)]
    pub prepare_query_no_tools: Option<DurationMetric>,
    #[serde(default)]
    pub prepare_query_completed_tool_chain: Option<DurationMetric>,
    #[serde(default)]
    pub prepare_query_requires_approval: Option<DurationMetric>,
    #[serde(default)]
    pub prepare_query_policy_denied: Option<DurationMetric>,
    #[serde(default)]
    pub splice_remove_branch_point: Option<DurationMetric>,
    #[serde(default)]
    pub splice_remove_root_many_children: Option<DurationMetric>,
    #[serde(default)]
    pub splice_remove_tool_group: Option<DurationMetric>,
    #[serde(default)]
    pub truncate_large_subtree: Option<DurationMetric>,
    #[serde(default)]
    pub context_build_plain_100: Option<DurationMetric>,
    #[serde(default)]
    pub context_build_plain_1000: Option<DurationMetric>,
    #[serde(default)]
    pub context_build_with_system_prompt: Option<DurationMetric>,
    #[serde(default)]
    pub context_build_with_compaction: Option<DurationMetric>,
    #[serde(default)]
    pub context_build_with_image_parts: Option<DurationMetric>,
    #[serde(default)]
    pub provider_tool_attach_load: Option<DurationMetric>,
    #[serde(default)]
    pub fake_mcp_list_call: Option<DurationMetric>,
    pub gateway_ready: Option<DurationMetric>,
    pub first_token: Option<DurationMetric>,
    pub full_response: Option<DurationMetric>,
}

/// Summary of one duration field, in integer microseconds.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DurationMetric {
    pub min_us: u64,
    pub median_us: u64,
    pub p95_us: u64,
    pub max_us: u64,
}

/// Difference between two persisted benchmark reports.
#[derive(Debug, Clone, PartialEq)]
pub struct PerformanceComparison {
    pub baseline_mode: BenchmarkMode,
    pub current_mode: BenchmarkMode,
    pub baseline_runs: usize,
    pub current_runs: usize,
    pub rows: Vec<PerformanceComparisonRow>,
}

/// Difference for one comparable summary metric.
#[derive(Debug, Clone, PartialEq)]
pub struct PerformanceComparisonRow {
    pub name: &'static str,
    pub baseline_median_us: u64,
    pub current_median_us: u64,
    pub change_percent: f64,
}

/// Provider-free timings for runtime and write-path primitives.
struct RuntimeBenchmarkTimings {
    prepare_query_turn: Duration,
    pending_tool_approval_scan: Duration,
    tool_result_insert: Duration,
    deny_tool_result_persist: Duration,
    splice_remove: Duration,
    truncate: Duration,
    context_build_after_tool_chain: Duration,
    active_path_load_100: Duration,
    active_path_load_1000: Duration,
    pending_tool_approval_scan_long_path: Duration,
    pending_tool_approval_scan_deep_chain: Duration,
    prepare_query_no_tools: Duration,
    prepare_query_completed_tool_chain: Duration,
    prepare_query_requires_approval: Duration,
    prepare_query_policy_denied: Duration,
    splice_remove_branch_point: Duration,
    splice_remove_root_many_children: Duration,
    splice_remove_tool_group: Duration,
    truncate_large_subtree: Duration,
    context_build_plain_100: Duration,
    context_build_plain_1000: Duration,
    context_build_with_system_prompt: Duration,
    context_build_with_compaction: Duration,
    context_build_with_image_parts: Duration,
    provider_tool_attach_load: Duration,
    fake_mcp_list_call: Duration,
    active_path_messages: usize,
    tree_messages: usize,
    requested_tool_calls: usize,
    resolved_tool_results: usize,
    deleted_messages: usize,
    promoted_children: usize,
    truncated_messages: usize,
}

/// Counts and duration from the context-after-tool-chain scenario.
struct RuntimeContextBenchmark {
    duration: Duration,
    active_path_messages: usize,
    tree_messages: usize,
    requested_tool_calls: usize,
    resolved_tool_results: usize,
}

/// Runs the selected benchmark mode.
///
/// Conversation mode is free/local. Live mode requires Bifrost and sends a tiny
/// real model request.
pub async fn run(
    mode: BenchmarkMode,
    conversation_id: Option<ConversationId>,
    gateway_url: GatewayUrl,
    base_url: BaseUrl,
    model: ModelName,
) -> Result<PerformanceBaseline> {
    let mut baseline = PerformanceBaseline {
        mode,
        model,
        conversation_id,
        store_open: None,
        conversation_load: None,
        active_message_lookup: None,
        active_path_row_load: None,
        active_path_part_load: None,
        tree_load: None,
        tree_row_load: None,
        tree_part_load: None,
        tool_schema_load: None,
        context_build: None,
        context_active_path_load: None,
        context_system_prompt_load: None,
        context_compaction_load: None,
        context_flatten: None,
        prepare_query_turn: None,
        pending_tool_approval_scan: None,
        tool_result_insert: None,
        deny_tool_result_persist: None,
        splice_remove: None,
        truncate: None,
        context_build_after_tool_chain: None,
        active_path_load_100: None,
        active_path_load_1000: None,
        pending_tool_approval_scan_long_path: None,
        pending_tool_approval_scan_deep_chain: None,
        prepare_query_no_tools: None,
        prepare_query_completed_tool_chain: None,
        prepare_query_requires_approval: None,
        prepare_query_policy_denied: None,
        splice_remove_branch_point: None,
        splice_remove_root_many_children: None,
        splice_remove_tool_group: None,
        truncate_large_subtree: None,
        context_build_plain_100: None,
        context_build_plain_1000: None,
        context_build_with_system_prompt: None,
        context_build_with_compaction: None,
        context_build_with_image_parts: None,
        provider_tool_attach_load: None,
        fake_mcp_list_call: None,
        loaded_messages: None,
        tree_messages: None,
        requested_tool_calls: None,
        resolved_tool_results: None,
        deleted_messages: None,
        promoted_children: None,
        truncated_messages: None,
        gateway_ready: None,
        first_token: None,
        full_response: None,
        response_bytes: None,
    };

    match mode {
        BenchmarkMode::Conversation => {
            let store_started = Instant::now();
            let store = Store::open()?;
            let store_open = store_started.elapsed();
            let conversation_id = baseline
                .conversation_id
                .as_ref()
                .expect("conversation benchmark requires conversation id");

            let load_started = Instant::now();
            let active_message_lookup_started = Instant::now();
            let active_message_id = store.active_message_id(conversation_id)?;
            let active_message_lookup = active_message_lookup_started.elapsed();
            let active_path = if let Some(active_message_id) = active_message_id.as_ref() {
                let row_started = Instant::now();
                let messages =
                    store.load_path_to_message_rows(conversation_id, active_message_id)?;
                let row_load = row_started.elapsed();

                let part_started = Instant::now();
                let mut messages = messages;
                store
                    .attach_message_parts(&mut messages)
                    .context("failed to load active path parts")?;
                let part_load = part_started.elapsed();

                (messages, row_load, part_load)
            } else {
                (Vec::new(), Duration::ZERO, Duration::ZERO)
            };
            let loaded_messages = active_path.0.len();
            let active_path_row_load = active_path.1;
            let active_path_part_load = active_path.2;
            let conversation_load = load_started.elapsed();

            let tree_started = Instant::now();
            let tree_row_started = Instant::now();
            let mut tree = store.load_message_rows(conversation_id)?;
            let tree_row_load = tree_row_started.elapsed();

            let tree_part_started = Instant::now();
            store
                .attach_message_parts(&mut tree)
                .context("failed to load message tree parts")?;
            let tree_part_load = tree_part_started.elapsed();
            let tree_messages = tree.len();
            let tree_load = tree_started.elapsed();

            let tool_schema_started = Instant::now();
            let _ = store.load_tool_schemas(conversation_id)?;
            let tool_schema_load = tool_schema_started.elapsed();

            let context_started = Instant::now();
            let context_active_path_started = Instant::now();
            let context_active_path = store.load_active_path(conversation_id)?;
            let context_active_path_load = context_active_path_started.elapsed();

            let context_system_prompt_started = Instant::now();
            let context_system_prompt = store.system_prompt(conversation_id)?;
            let context_system_prompt_load = context_system_prompt_started.elapsed();

            let context_compaction_started = Instant::now();
            let context_compaction = store.latest_compaction(conversation_id)?;
            let context_compaction_load = context_compaction_started.elapsed();

            let context_flatten_started = Instant::now();
            let _ = ContextBuilder::flatten(ContextParts {
                active_path: context_active_path,
                system_prompt: context_system_prompt,
                compaction: context_compaction,
            });
            let context_flatten = context_flatten_started.elapsed();
            let context_build = context_started.elapsed();

            baseline.store_open = Some(store_open);
            baseline.conversation_load = Some(conversation_load);
            baseline.active_message_lookup = Some(active_message_lookup);
            baseline.active_path_row_load = Some(active_path_row_load);
            baseline.active_path_part_load = Some(active_path_part_load);
            baseline.tree_load = Some(tree_load);
            baseline.tree_row_load = Some(tree_row_load);
            baseline.tree_part_load = Some(tree_part_load);
            baseline.tool_schema_load = Some(tool_schema_load);
            baseline.context_build = Some(context_build);
            baseline.context_active_path_load = Some(context_active_path_load);
            baseline.context_system_prompt_load = Some(context_system_prompt_load);
            baseline.context_compaction_load = Some(context_compaction_load);
            baseline.context_flatten = Some(context_flatten);
            baseline.loaded_messages = Some(loaded_messages);
            baseline.tree_messages = Some(tree_messages);
        }
        BenchmarkMode::Runtime => {
            let runtime = run_runtime_benchmark()?;
            baseline.prepare_query_turn = Some(runtime.prepare_query_turn);
            baseline.pending_tool_approval_scan = Some(runtime.pending_tool_approval_scan);
            baseline.tool_result_insert = Some(runtime.tool_result_insert);
            baseline.deny_tool_result_persist = Some(runtime.deny_tool_result_persist);
            baseline.splice_remove = Some(runtime.splice_remove);
            baseline.truncate = Some(runtime.truncate);
            baseline.context_build_after_tool_chain = Some(runtime.context_build_after_tool_chain);
            baseline.active_path_load_100 = Some(runtime.active_path_load_100);
            baseline.active_path_load_1000 = Some(runtime.active_path_load_1000);
            baseline.pending_tool_approval_scan_long_path =
                Some(runtime.pending_tool_approval_scan_long_path);
            baseline.pending_tool_approval_scan_deep_chain =
                Some(runtime.pending_tool_approval_scan_deep_chain);
            baseline.prepare_query_no_tools = Some(runtime.prepare_query_no_tools);
            baseline.prepare_query_completed_tool_chain =
                Some(runtime.prepare_query_completed_tool_chain);
            baseline.prepare_query_requires_approval =
                Some(runtime.prepare_query_requires_approval);
            baseline.prepare_query_policy_denied = Some(runtime.prepare_query_policy_denied);
            baseline.splice_remove_branch_point = Some(runtime.splice_remove_branch_point);
            baseline.splice_remove_root_many_children =
                Some(runtime.splice_remove_root_many_children);
            baseline.splice_remove_tool_group = Some(runtime.splice_remove_tool_group);
            baseline.truncate_large_subtree = Some(runtime.truncate_large_subtree);
            baseline.context_build_plain_100 = Some(runtime.context_build_plain_100);
            baseline.context_build_plain_1000 = Some(runtime.context_build_plain_1000);
            baseline.context_build_with_system_prompt =
                Some(runtime.context_build_with_system_prompt);
            baseline.context_build_with_compaction = Some(runtime.context_build_with_compaction);
            baseline.context_build_with_image_parts = Some(runtime.context_build_with_image_parts);
            baseline.provider_tool_attach_load = Some(runtime.provider_tool_attach_load);
            baseline.fake_mcp_list_call = Some(runtime.fake_mcp_list_call);
            baseline.loaded_messages = Some(runtime.active_path_messages);
            baseline.tree_messages = Some(runtime.tree_messages);
            baseline.requested_tool_calls = Some(runtime.requested_tool_calls);
            baseline.resolved_tool_results = Some(runtime.resolved_tool_results);
            baseline.deleted_messages = Some(runtime.deleted_messages);
            baseline.promoted_children = Some(runtime.promoted_children);
            baseline.truncated_messages = Some(runtime.truncated_messages);
        }
        BenchmarkMode::Live => {
            let gateway = BifrostGateway::new(gateway_url);
            let gateway_started = Instant::now();
            gateway.require_running().await?;
            baseline.gateway_ready = Some(gateway_started.elapsed());
            let (first_token, full_response, response_bytes) =
                run_live_request(&base_url, &baseline.model).await?;
            baseline.first_token = first_token;
            baseline.full_response = Some(full_response);
            baseline.response_bytes = Some(response_bytes);
        }
    }

    Ok(baseline)
}

/// Runs the selected benchmark repeatedly and returns a persistent report.
pub async fn run_report(
    mode: BenchmarkMode,
    conversation_id: Option<ConversationId>,
    gateway_url: GatewayUrl,
    base_url: BaseUrl,
    model: ModelName,
    runs: usize,
) -> Result<PerformanceReport> {
    let mut samples = Vec::with_capacity(runs);

    for _ in 0..runs {
        let baseline = run(
            mode,
            conversation_id.clone(),
            gateway_url.clone(),
            base_url.clone(),
            model.clone(),
        )
        .await?;
        samples.push(PerformanceSample::from_baseline(&baseline));
    }

    Ok(PerformanceReport {
        format_version: REPORT_FORMAT_VERSION,
        mode,
        model: model.as_str().to_string(),
        conversation_id: conversation_id.map(|id| id.as_str().to_string()),
        runs,
        summary: PerformanceSummary::from_samples(&samples),
        samples,
    })
}

/// Reads a JSON benchmark report from disk.
pub fn read_report(path: &Path) -> Result<PerformanceReport> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("failed to read benchmark report {}", path.display()))?;
    let report = serde_json::from_str(&text)
        .with_context(|| format!("failed to parse benchmark report {}", path.display()))?;

    Ok(report)
}

/// Compares median duration metrics from two reports.
pub fn compare_reports(
    baseline: &PerformanceReport,
    current: &PerformanceReport,
) -> PerformanceComparison {
    PerformanceComparison {
        baseline_mode: baseline.mode,
        current_mode: current.mode,
        baseline_runs: baseline.runs,
        current_runs: current.runs,
        rows: comparison_rows(&baseline.summary, &current.summary),
    }
}

/// Runs local runtime/write benchmarks against temporary fixture databases.
///
/// Each measured operation gets a fresh SQLite database under the OS temp
/// directory. Setup is outside the timing window, so the measured durations are
/// primitive costs rather than fixture construction.
fn run_runtime_benchmark() -> Result<RuntimeBenchmarkTimings> {
    let prepare_query_turn = benchmark_prepare_query_turn()?;
    let pending_tool_approval_scan = benchmark_pending_tool_approval_scan()?;
    let tool_result_insert = benchmark_tool_result_insert()?;
    let deny_tool_result_persist = benchmark_deny_tool_result_persist()?;
    let (splice_remove, deleted_messages, promoted_children) = benchmark_splice_remove()?;
    let (truncate, truncated_messages) = benchmark_truncate()?;
    let context = benchmark_context_after_tool_chain()?;
    let active_path_load_100 = benchmark_active_path_load(SCALE_PATH_MESSAGES)?;
    let active_path_load_1000 = benchmark_active_path_load(LARGE_SCALE_PATH_MESSAGES)?;
    let pending_tool_approval_scan_long_path = benchmark_pending_tool_approval_scan_long_path()?;
    let pending_tool_approval_scan_deep_chain = benchmark_pending_tool_approval_scan_deep_chain()?;
    let prepare_query_no_tools = benchmark_prepare_query_no_tools()?;
    let prepare_query_completed_tool_chain = benchmark_prepare_query_completed_tool_chain()?;
    let prepare_query_requires_approval = benchmark_prepare_query_requires_approval()?;
    let prepare_query_policy_denied = benchmark_prepare_query_policy_denied()?;
    let (splice_remove_branch_point, _branch_deleted_messages, _branch_promoted_children) =
        benchmark_splice_remove_branch_point()?;
    let (splice_remove_root_many_children, _root_deleted_messages, _root_promoted_children) =
        benchmark_splice_remove_root_many_children()?;
    let splice_remove_tool_group = benchmark_splice_remove_tool_group()?;
    let (truncate_large_subtree, _large_truncated_messages) = benchmark_truncate_large_subtree()?;
    let context_build_plain_100 = benchmark_context_plain(SCALE_PATH_MESSAGES)?;
    let context_build_plain_1000 = benchmark_context_plain(LARGE_SCALE_PATH_MESSAGES)?;
    let context_build_with_system_prompt = benchmark_context_with_system_prompt()?;
    let context_build_with_compaction = benchmark_context_with_compaction()?;
    let context_build_with_image_parts = benchmark_context_with_image_parts()?;
    let provider_tool_attach_load = benchmark_provider_tool_attach_load()?;
    let fake_mcp_list_call = benchmark_fake_mcp_list_call()?;

    Ok(RuntimeBenchmarkTimings {
        prepare_query_turn,
        pending_tool_approval_scan,
        tool_result_insert,
        deny_tool_result_persist,
        splice_remove,
        truncate,
        context_build_after_tool_chain: context.duration,
        active_path_load_100,
        active_path_load_1000,
        pending_tool_approval_scan_long_path,
        pending_tool_approval_scan_deep_chain,
        prepare_query_no_tools,
        prepare_query_completed_tool_chain,
        prepare_query_requires_approval,
        prepare_query_policy_denied,
        splice_remove_branch_point,
        splice_remove_root_many_children,
        splice_remove_tool_group,
        truncate_large_subtree,
        context_build_plain_100,
        context_build_plain_1000,
        context_build_with_system_prompt,
        context_build_with_compaction,
        context_build_with_image_parts,
        provider_tool_attach_load,
        fake_mcp_list_call,
        active_path_messages: context.active_path_messages,
        tree_messages: context.tree_messages,
        requested_tool_calls: context.requested_tool_calls,
        resolved_tool_results: context.resolved_tool_results,
        deleted_messages,
        promoted_children,
        truncated_messages,
    })
}

/// Measures query preparation on a minimal ready active path.
fn benchmark_prepare_query_turn() -> Result<Duration> {
    with_runtime_store("prepare-query-turn", |store| {
        let conversation_id = store.create_conversation()?;
        insert_user_message(store, &conversation_id, None, "ready")?;

        let started = Instant::now();
        prepare_query_turn(store, &conversation_id)?;
        Ok(started.elapsed())
    })
}

/// Measures active-path scanning for the next approval-required tool call.
fn benchmark_pending_tool_approval_scan() -> Result<Duration> {
    with_runtime_store("pending-tool-approval-scan", |store| {
        let conversation_id = store.create_conversation()?;
        attach_test_mcp_tool(store, &conversation_id)?;
        let user_id = insert_user_message(store, &conversation_id, None, "use tools")?;
        let metadata = tool_call_metadata(vec![
            tool_call(0, "call_1", "desktop_commander__read_file"),
            tool_call(1, "call_2", "desktop_commander__read_file"),
        ]);
        store.insert_message(
            &conversation_id,
            Some(&user_id),
            Role::Assistant,
            "",
            Some(&metadata),
        )?;

        let started = Instant::now();
        let approvals = pending_tool_approvals(store, &conversation_id)?;
        let duration = started.elapsed();
        debug_assert_eq!(approvals.len(), 1);

        Ok(duration)
    })
}

/// Measures raw tool-result message persistence without shell execution.
fn benchmark_tool_result_insert() -> Result<Duration> {
    with_runtime_store("tool-result-insert", |store| {
        let conversation_id = store.create_conversation()?;
        let user_id = insert_user_message(store, &conversation_id, None, "use a tool")?;
        let call = tool_call(0, "call_1", "desktop_commander__read_file");
        let assistant_id = store.insert_message(
            &conversation_id,
            Some(&user_id),
            Role::Assistant,
            "",
            Some(&tool_call_metadata(vec![call.clone()])),
        )?;
        let started = Instant::now();
        store.insert_tool_result_message(
            &conversation_id,
            &assistant_id,
            &call.id,
            r#"{"stdout":"ok","stderr":"","exit_code":0}"#,
        )?;
        Ok(started.elapsed())
    })
}

/// Measures explicit denial lookup plus result persistence without shell work.
fn benchmark_deny_tool_result_persist() -> Result<Duration> {
    with_runtime_store("deny-tool-result-persist", |store| {
        let conversation_id = store.create_conversation()?;
        let user_id = insert_user_message(store, &conversation_id, None, "use a tool")?;
        let call = tool_call(0, "call_1", "desktop_commander__read_file");
        store.insert_message(
            &conversation_id,
            Some(&user_id),
            Role::Assistant,
            "",
            Some(&tool_call_metadata(vec![call.clone()])),
        )?;

        let started = Instant::now();
        deny_tool_call(store, &conversation_id, &call.id)?;
        Ok(started.elapsed())
    })
}

/// Measures splice delete for a middle message in a chain.
fn benchmark_splice_remove() -> Result<(Duration, usize, usize)> {
    with_runtime_store("splice-remove", |store| {
        let conversation_id = store.create_conversation()?;
        let first_id = insert_user_message(store, &conversation_id, None, "first")?;
        let removed_id = store.insert_message(
            &conversation_id,
            Some(&first_id),
            Role::Assistant,
            "middle",
            None,
        )?;
        let _child_id = insert_user_message(store, &conversation_id, Some(&removed_id), "child")?;

        let started = Instant::now();
        store.remove_message(&conversation_id, &removed_id)?;
        Ok((started.elapsed(), 1, 1))
    })
}

/// Measures descendant deletion below a checkpoint message.
fn benchmark_truncate() -> Result<(Duration, usize)> {
    with_runtime_store("truncate", |store| {
        let conversation_id = store.create_conversation()?;
        let first_id = insert_user_message(store, &conversation_id, None, "first")?;
        let checkpoint_id = store.insert_message(
            &conversation_id,
            Some(&first_id),
            Role::Assistant,
            "keep",
            None,
        )?;
        let child_id = insert_user_message(store, &conversation_id, Some(&checkpoint_id), "child")?;
        let _grandchild_id = store.insert_message(
            &conversation_id,
            Some(&child_id),
            Role::Assistant,
            "leaf",
            None,
        )?;

        let started = Instant::now();
        store.truncate_after_message(&conversation_id, &checkpoint_id)?;
        Ok((started.elapsed(), 2))
    })
}

/// Measures context construction after a completed two-result tool chain.
fn benchmark_context_after_tool_chain() -> Result<RuntimeContextBenchmark> {
    with_runtime_store("context-after-tool-chain", |store| {
        let conversation_id = store.create_conversation()?;
        let user_id = insert_user_message(store, &conversation_id, None, "use tools")?;
        let first_call = tool_call(0, "call_1", "desktop_commander__read_file");
        let second_call = tool_call(1, "call_2", "desktop_commander__read_file");
        let assistant_id = store.insert_message(
            &conversation_id,
            Some(&user_id),
            Role::Assistant,
            "",
            Some(&tool_call_metadata(vec![
                first_call.clone(),
                second_call.clone(),
            ])),
        )?;
        let first_result_id =
            insert_tool_result(store, &conversation_id, &assistant_id, &first_call.id)?;
        let _second_result_id =
            insert_tool_result(store, &conversation_id, &first_result_id, &second_call.id)?;

        let active_path_messages = store.load_active_path(&conversation_id)?.len();
        let tree_messages = store.load_message_tree(&conversation_id)?.len();

        let started = Instant::now();
        let _ = ContextBuilder::build(store, &conversation_id)?;
        let duration = started.elapsed();

        Ok(RuntimeContextBenchmark {
            duration,
            active_path_messages,
            tree_messages,
            requested_tool_calls: 2,
            resolved_tool_results: 2,
        })
    })
}

/// Measures active-path loading for a generated message chain.
fn benchmark_active_path_load(message_count: usize) -> Result<Duration> {
    with_runtime_store(&format!("active-path-load-{message_count}"), |store| {
        let conversation_id = store.create_conversation()?;
        create_message_chain(store, &conversation_id, message_count)?;

        let started = Instant::now();
        let messages = store.load_active_path(&conversation_id)?;
        let duration = started.elapsed();
        debug_assert_eq!(messages.len(), message_count);

        Ok(duration)
    })
}

/// Measures approval scanning when a tool call appears after a long path.
fn benchmark_pending_tool_approval_scan_long_path() -> Result<Duration> {
    with_runtime_store("pending-tool-approval-long-path", |store| {
        let conversation_id = store.create_conversation()?;
        attach_test_mcp_tool(store, &conversation_id)?;
        let parent_id = create_message_chain(store, &conversation_id, SCALE_PATH_MESSAGES)?;
        let metadata =
            tool_call_metadata(vec![tool_call(0, "call_1", "desktop_commander__read_file")]);
        store.insert_message(
            &conversation_id,
            parent_id.as_ref(),
            Role::Assistant,
            "",
            Some(&metadata),
        )?;

        let started = Instant::now();
        let approvals = pending_tool_approvals(store, &conversation_id)?;
        let duration = started.elapsed();
        debug_assert_eq!(approvals.len(), 1);

        Ok(duration)
    })
}

/// Measures approval scanning after many prior tool results in one execution.
fn benchmark_pending_tool_approval_scan_deep_chain() -> Result<Duration> {
    with_runtime_store("pending-tool-approval-deep-chain", |store| {
        let conversation_id = store.create_conversation()?;
        attach_test_mcp_tool(store, &conversation_id)?;
        let user_id = insert_user_message(store, &conversation_id, None, "use many tools")?;
        let tool_calls = (0..TOOL_CHAIN_RESULTS)
            .map(|index| {
                tool_call(
                    index as u16,
                    &format!("call_{index}"),
                    "desktop_commander__read_file",
                )
            })
            .collect::<Vec<_>>();
        let assistant_id = store.insert_message(
            &conversation_id,
            Some(&user_id),
            Role::Assistant,
            "",
            Some(&tool_call_metadata(tool_calls.clone())),
        )?;
        let mut parent_id = assistant_id;
        for tool_call in tool_calls.iter().take(TOOL_CHAIN_RESULTS - 1) {
            parent_id = insert_tool_result(store, &conversation_id, &parent_id, &tool_call.id)?;
        }

        let started = Instant::now();
        let approvals = pending_tool_approvals(store, &conversation_id)?;
        let duration = started.elapsed();
        debug_assert_eq!(approvals.len(), 1);

        Ok(duration)
    })
}

/// Measures query preparation on a plain completed active path.
fn benchmark_prepare_query_no_tools() -> Result<Duration> {
    with_runtime_store("prepare-query-no-tools", |store| {
        let conversation_id = store.create_conversation()?;
        create_message_chain(store, &conversation_id, SCALE_PATH_MESSAGES)?;

        let started = Instant::now();
        prepare_query_turn(store, &conversation_id)?;
        Ok(started.elapsed())
    })
}

/// Measures query preparation after all requested tool calls have results.
fn benchmark_prepare_query_completed_tool_chain() -> Result<Duration> {
    with_runtime_store("prepare-query-completed-tool-chain", |store| {
        let conversation_id = store.create_conversation()?;
        create_completed_tool_chain(store, &conversation_id, TOOL_CHAIN_RESULTS)?;

        let started = Instant::now();
        prepare_query_turn(store, &conversation_id)?;
        Ok(started.elapsed())
    })
}

/// Measures the rejection path when query is waiting on approval.
fn benchmark_prepare_query_requires_approval() -> Result<Duration> {
    with_runtime_store("prepare-query-requires-approval", |store| {
        let conversation_id = store.create_conversation()?;
        attach_test_mcp_tool(store, &conversation_id)?;
        let user_id = insert_user_message(store, &conversation_id, None, "use a tool")?;
        let metadata =
            tool_call_metadata(vec![tool_call(0, "call_1", "desktop_commander__read_file")]);
        store.insert_message(
            &conversation_id,
            Some(&user_id),
            Role::Assistant,
            "",
            Some(&metadata),
        )?;

        let started = Instant::now();
        let result = prepare_query_turn(store, &conversation_id);
        let duration = started.elapsed();
        debug_assert!(result.is_err());

        Ok(duration)
    })
}

/// Measures preparation when policy-denied tool calls are auto-recorded.
fn benchmark_prepare_query_policy_denied() -> Result<Duration> {
    with_runtime_store("prepare-query-policy-denied", |store| {
        let conversation_id = store.create_conversation()?;
        let user_id = insert_user_message(store, &conversation_id, None, "use a tool")?;
        let metadata = tool_call_metadata(vec![tool_call(0, "denied_call", "unknown_tool")]);
        store.insert_message(
            &conversation_id,
            Some(&user_id),
            Role::Assistant,
            "",
            Some(&metadata),
        )?;

        let started = Instant::now();
        prepare_query_turn(store, &conversation_id)?;
        Ok(started.elapsed())
    })
}

/// Measures splice delete for a branch point with many direct children.
fn benchmark_splice_remove_branch_point() -> Result<(Duration, usize, usize)> {
    with_runtime_store("splice-remove-branch-point", |store| {
        let conversation_id = store.create_conversation()?;
        let root_id = insert_user_message(store, &conversation_id, None, "root")?;
        let branch_id = store.insert_message(
            &conversation_id,
            Some(&root_id),
            Role::Assistant,
            "branch",
            None,
        )?;
        for index in 0..BRANCH_CHILDREN {
            insert_user_message(
                store,
                &conversation_id,
                Some(&branch_id),
                &format!("child {index}"),
            )?;
        }

        let started = Instant::now();
        store.remove_message(&conversation_id, &branch_id)?;
        Ok((started.elapsed(), 1, BRANCH_CHILDREN))
    })
}

/// Measures splice delete for a root node with many direct children.
fn benchmark_splice_remove_root_many_children() -> Result<(Duration, usize, usize)> {
    with_runtime_store("splice-remove-root-many-children", |store| {
        let conversation_id = store.create_conversation()?;
        let root_id = insert_user_message(store, &conversation_id, None, "root")?;
        for index in 0..BRANCH_CHILDREN {
            insert_user_message(
                store,
                &conversation_id,
                Some(&root_id),
                &format!("child {index}"),
            )?;
        }

        let started = Instant::now();
        store.remove_message(&conversation_id, &root_id)?;
        Ok((started.elapsed(), 1, BRANCH_CHILDREN))
    })
}

/// Measures splice delete for an assistant tool-call group.
fn benchmark_splice_remove_tool_group() -> Result<Duration> {
    with_runtime_store("splice-remove-tool-group", |store| {
        let conversation_id = store.create_conversation()?;
        let assistant_id = create_completed_tool_chain(store, &conversation_id, 2)?;
        let second_result_id = store
            .active_message_id(&conversation_id)?
            .expect("tool chain fixture should have active result");
        store.insert_message(
            &conversation_id,
            Some(&second_result_id),
            Role::Assistant,
            "done",
            None,
        )?;

        let started = Instant::now();
        store.remove_message(&conversation_id, &assistant_id)?;
        Ok(started.elapsed())
    })
}

/// Measures truncate for a subtree with many descendants.
fn benchmark_truncate_large_subtree() -> Result<(Duration, usize)> {
    with_runtime_store("truncate-large-subtree", |store| {
        let conversation_id = store.create_conversation()?;
        let checkpoint_id = insert_user_message(store, &conversation_id, None, "checkpoint")?;
        let mut parent_id = checkpoint_id.clone();
        for index in 0..LARGE_TRUNCATE_DESCENDANTS {
            parent_id = store.insert_message(
                &conversation_id,
                Some(&parent_id),
                if index % 2 == 0 {
                    Role::Assistant
                } else {
                    Role::User
                },
                &format!("descendant {index}"),
                None,
            )?;
        }

        let started = Instant::now();
        store.truncate_after_message(&conversation_id, &checkpoint_id)?;
        Ok((started.elapsed(), LARGE_TRUNCATE_DESCENDANTS))
    })
}

/// Measures context build for a plain generated message chain.
fn benchmark_context_plain(message_count: usize) -> Result<Duration> {
    with_runtime_store(&format!("context-plain-{message_count}"), |store| {
        let conversation_id = store.create_conversation()?;
        create_message_chain(store, &conversation_id, message_count)?;

        let started = Instant::now();
        let _ = ContextBuilder::build(store, &conversation_id)?;
        Ok(started.elapsed())
    })
}

/// Measures context build when a conversation system prompt exists.
fn benchmark_context_with_system_prompt() -> Result<Duration> {
    with_runtime_store("context-with-system-prompt", |store| {
        let conversation_id = store.create_conversation()?;
        create_message_chain(store, &conversation_id, SCALE_PATH_MESSAGES)?;
        store.set_system_prompt(&conversation_id, "You are a concise local runtime.")?;

        let started = Instant::now();
        let _ = ContextBuilder::build(store, &conversation_id)?;
        Ok(started.elapsed())
    })
}

/// Measures context build with a compaction summary plus a remaining suffix.
fn benchmark_context_with_compaction() -> Result<Duration> {
    with_runtime_store("context-with-compaction", |store| {
        let conversation_id = store.create_conversation()?;
        let checkpoint_index = SCALE_PATH_MESSAGES / 2;
        let mut parent_id = None;
        let mut checkpoint_id = None;

        for index in 0..SCALE_PATH_MESSAGES {
            let role = if index % 2 == 0 {
                Role::User
            } else {
                Role::Assistant
            };
            let id = store.insert_message(
                &conversation_id,
                parent_id.as_ref(),
                role,
                &format!("message {index}"),
                None,
            )?;
            if index == checkpoint_index {
                checkpoint_id = Some(id.clone());
            }
            parent_id = Some(id);
        }

        let checkpoint_id = checkpoint_id.expect("message chain fixture should have a checkpoint");
        store.save_compaction(&conversation_id, &checkpoint_id, "summary")?;

        let started = Instant::now();
        let _ = ContextBuilder::build(store, &conversation_id)?;
        Ok(started.elapsed())
    })
}

/// Measures context build for image-heavy message parts.
fn benchmark_context_with_image_parts() -> Result<Duration> {
    with_runtime_store("context-with-image-parts", |store| {
        let conversation_id = store.create_conversation()?;
        let image_bytes = tiny_png_bytes();
        let mut parent_id = None;
        for index in 0..IMAGE_PART_MESSAGES {
            let id = store.insert_message_with_parts(
                &conversation_id,
                parent_id.as_ref(),
                Role::User,
                &format!("image message {index}"),
                &[
                    UnsavedMessagePart::Text("image".to_string()),
                    UnsavedMessagePart::Image(UnsavedImagePart {
                        mime_type: "image/png".to_string(),
                        bytes: image_bytes.to_vec(),
                    }),
                ],
                None,
            )?;
            parent_id = Some(id);
        }

        let started = Instant::now();
        let _ = ContextBuilder::build(store, &conversation_id)?;
        Ok(started.elapsed())
    })
}

/// Measures provider registry lookup plus attached-tool persistence.
fn benchmark_provider_tool_attach_load() -> Result<Duration> {
    with_runtime_store("provider-tool-attach-load", |store| {
        let conversation_id = store.create_conversation()?;
        let definition = test_tool_definition();

        let started = Instant::now();
        store.insert_attached_tool(&conversation_id, &definition.attached_tool())?;
        let attached_tool = store
            .load_attached_tool(&conversation_id, &definition.schema_name)?
            .ok_or_else(|| anyhow::anyhow!("attached test provider tool was not stored"))?;
        let can_execute = attached_tool.provider.kind == ToolProviderKind::Mcp
            && attached_tool.provider.provider_id.as_str() == TEST_PROVIDER_ID;
        let duration = started.elapsed();
        debug_assert!(can_execute);

        Ok(duration)
    })
}

/// Measures a provider-free MCP initialize, tools/list, and tools/call path.
fn benchmark_fake_mcp_list_call() -> Result<Duration> {
    let started = Instant::now();
    let tools = mcp::list_tools(FAKE_MCP_COMMAND)?;
    let result = mcp::call_tool(FAKE_MCP_COMMAND, "click", serde_json::json!({}))?;
    let duration = started.elapsed();
    debug_assert_eq!(tools.len(), 1);
    debug_assert_eq!(tools[0].name, "click");
    debug_assert_eq!(result["isError"], false);

    Ok(duration)
}

/// Creates one temporary benchmark store and removes database files after use.
fn with_runtime_store<T>(scenario: &str, run: impl FnOnce(&mut Store) -> Result<T>) -> Result<T> {
    let path = runtime_database_path(scenario);
    let result = {
        let mut store = Store::open_at(&path)?;
        run(&mut store)
    };
    remove_runtime_database_files(&path);

    result
}

/// Builds a unique SQLite path for one runtime benchmark scenario.
fn runtime_database_path(scenario: &str) -> PathBuf {
    env::temp_dir().join(format!(
        "windie-runtime-bench-{scenario}-{}-{}.db",
        process::id(),
        Uuid::new_v4()
    ))
}

/// Removes SQLite database files created for one benchmark scenario.
fn remove_runtime_database_files(path: &Path) {
    let _ = fs::remove_file(path);
    let _ = fs::remove_file(path.with_extension("db-wal"));
    let _ = fs::remove_file(path.with_extension("db-shm"));
}

/// Inserts a simple user message and returns its generated message ID.
fn insert_user_message(
    store: &mut Store,
    conversation_id: &ConversationId,
    parent_message_id: Option<&MessageId>,
    content: &str,
) -> Result<MessageId> {
    store.insert_message(
        conversation_id,
        parent_message_id,
        Role::User,
        content,
        None,
    )
}

/// Attaches a test MCP provider tool to a benchmark conversation.
///
/// Runtime approval benchmarks need the same provider-backed attachment that
/// real conversations use; otherwise policy would measure the detached-tool
/// denial path instead of the approval path.
fn attach_test_mcp_tool(store: &mut Store, conversation_id: &ConversationId) -> Result<()> {
    store.insert_attached_tool(conversation_id, &test_tool_definition().attached_tool())
}

/// Builds the provider-backed test tool used by runtime benchmarks.
fn test_tool_definition() -> ToolDefinition {
    ToolDefinition {
        schema_name: crate::conversation::ToolSchemaName::new(TEST_TOOL_SCHEMA_NAME),
        display_name: "Desktop Commander read_file".to_string(),
        description: "Read a file through Desktop Commander.".to_string(),
        parameters: serde_json::json!({"type":"object"}),
        provider: ToolProviderRef::new(
            ToolProviderId::new(TEST_PROVIDER_ID),
            ProviderToolName::new(TEST_PROVIDER_TOOL_NAME),
            ToolProviderKind::Mcp,
        ),
        permissions: vec![ToolPermission::ExternalProcess],
        annotations: ToolAnnotations::default(),
    }
}

/// Creates a linear active path with alternating user and assistant messages.
fn create_message_chain(
    store: &mut Store,
    conversation_id: &ConversationId,
    message_count: usize,
) -> Result<Option<MessageId>> {
    let mut parent_id = None;

    for index in 0..message_count {
        let role = if index % 2 == 0 {
            Role::User
        } else {
            Role::Assistant
        };
        let id = store.insert_message(
            conversation_id,
            parent_id.as_ref(),
            role,
            &format!("message {index}"),
            None,
        )?;
        parent_id = Some(id);
    }

    Ok(parent_id)
}

/// Creates one assistant tool-call message with all requested results stored.
fn create_completed_tool_chain(
    store: &mut Store,
    conversation_id: &ConversationId,
    result_count: usize,
) -> Result<MessageId> {
    let user_id = insert_user_message(store, conversation_id, None, "use tools")?;
    let tool_calls = (0..result_count)
        .map(|index| {
            tool_call(
                index as u16,
                &format!("call_{index}"),
                "desktop_commander__read_file",
            )
        })
        .collect::<Vec<_>>();
    let assistant_id = store.insert_message(
        conversation_id,
        Some(&user_id),
        Role::Assistant,
        "",
        Some(&tool_call_metadata(tool_calls.clone())),
    )?;
    let mut parent_id = assistant_id.clone();
    for tool_call in &tool_calls {
        parent_id = insert_tool_result(store, conversation_id, &parent_id, &tool_call.id)?;
    }

    Ok(assistant_id)
}

/// Inserts one model-facing tool result under the requested chain parent.
fn insert_tool_result(
    store: &mut Store,
    conversation_id: &ConversationId,
    parent_message_id: &MessageId,
    tool_call_id: &ToolCallId,
) -> Result<MessageId> {
    store.insert_tool_result_message(
        conversation_id,
        parent_message_id,
        tool_call_id,
        r#"{"stdout":"ok","stderr":"","exit_code":0}"#,
    )
}

/// Builds assistant metadata for requested tool calls.
fn tool_call_metadata(tool_calls: Vec<ToolCall>) -> MessageMetadata {
    MessageMetadata {
        tool_calls,
        ..Default::default()
    }
}

/// Builds a deterministic function tool call for runtime benchmark fixtures.
fn tool_call(index: u16, id: &str, name: &str) -> ToolCall {
    let mut tool_call = ToolCall::function(id, name, r#"{"command":"printf ok"}"#);
    tool_call.index = index;
    tool_call
}

/// Returns tiny deterministic bytes for image-part benchmark fixtures.
fn tiny_png_bytes() -> &'static [u8] {
    &[
        0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n', 0, 0, 0, 0, b'I', b'E', b'N', b'D',
    ]
}

/// Sends the tiny live request and measures first-token and full-response
/// latency.
async fn run_live_request(
    base_url: &BaseUrl,
    model: &ModelName,
) -> Result<(Option<Duration>, Duration, usize)> {
    let llm = BifrostClient::new(base_url.clone(), model.clone());
    let messages = vec![Message {
        id: None,
        parent_message_id: None,
        role: Role::User,
        content: BENCH_PROMPT.to_string(),
        parts: Vec::new(),
        metadata: None,
    }];

    let request_started = Instant::now();
    let mut first_token = None;
    let response = llm
        .stream(&messages, &[], |delta| {
            if first_token.is_none() && !delta.is_empty() {
                first_token = Some(request_started.elapsed());
            }

            Ok(())
        })
        .await?;
    let full_response = request_started.elapsed();

    Ok((first_token, full_response, response.content.len()))
}

impl PerformanceSample {
    /// Converts the in-memory timing result into JSON-safe primitive values.
    fn from_baseline(baseline: &PerformanceBaseline) -> Self {
        Self {
            store_open_us: baseline.store_open.map(duration_micros),
            active_path_load_us: baseline.conversation_load.map(duration_micros),
            active_message_lookup_us: baseline.active_message_lookup.map(duration_micros),
            active_path_row_load_us: baseline.active_path_row_load.map(duration_micros),
            active_path_part_load_us: baseline.active_path_part_load.map(duration_micros),
            tree_load_us: baseline.tree_load.map(duration_micros),
            tree_row_load_us: baseline.tree_row_load.map(duration_micros),
            tree_part_load_us: baseline.tree_part_load.map(duration_micros),
            tool_schema_load_us: baseline.tool_schema_load.map(duration_micros),
            context_build_us: baseline.context_build.map(duration_micros),
            context_active_path_load_us: baseline.context_active_path_load.map(duration_micros),
            context_system_prompt_load_us: baseline.context_system_prompt_load.map(duration_micros),
            context_compaction_load_us: baseline.context_compaction_load.map(duration_micros),
            context_flatten_us: baseline.context_flatten.map(duration_micros),
            prepare_query_turn_us: baseline.prepare_query_turn.map(duration_micros),
            pending_tool_approval_scan_us: baseline.pending_tool_approval_scan.map(duration_micros),
            tool_result_insert_us: baseline.tool_result_insert.map(duration_micros),
            deny_tool_result_persist_us: baseline.deny_tool_result_persist.map(duration_micros),
            splice_remove_us: baseline.splice_remove.map(duration_micros),
            truncate_us: baseline.truncate.map(duration_micros),
            context_build_after_tool_chain_us: baseline
                .context_build_after_tool_chain
                .map(duration_micros),
            active_path_load_100_us: baseline.active_path_load_100.map(duration_micros),
            active_path_load_1000_us: baseline.active_path_load_1000.map(duration_micros),
            pending_tool_approval_scan_long_path_us: baseline
                .pending_tool_approval_scan_long_path
                .map(duration_micros),
            pending_tool_approval_scan_deep_chain_us: baseline
                .pending_tool_approval_scan_deep_chain
                .map(duration_micros),
            prepare_query_no_tools_us: baseline.prepare_query_no_tools.map(duration_micros),
            prepare_query_completed_tool_chain_us: baseline
                .prepare_query_completed_tool_chain
                .map(duration_micros),
            prepare_query_requires_approval_us: baseline
                .prepare_query_requires_approval
                .map(duration_micros),
            prepare_query_policy_denied_us: baseline
                .prepare_query_policy_denied
                .map(duration_micros),
            splice_remove_branch_point_us: baseline.splice_remove_branch_point.map(duration_micros),
            splice_remove_root_many_children_us: baseline
                .splice_remove_root_many_children
                .map(duration_micros),
            splice_remove_tool_group_us: baseline.splice_remove_tool_group.map(duration_micros),
            truncate_large_subtree_us: baseline.truncate_large_subtree.map(duration_micros),
            context_build_plain_100_us: baseline.context_build_plain_100.map(duration_micros),
            context_build_plain_1000_us: baseline.context_build_plain_1000.map(duration_micros),
            context_build_with_system_prompt_us: baseline
                .context_build_with_system_prompt
                .map(duration_micros),
            context_build_with_compaction_us: baseline
                .context_build_with_compaction
                .map(duration_micros),
            context_build_with_image_parts_us: baseline
                .context_build_with_image_parts
                .map(duration_micros),
            provider_tool_attach_load_us: baseline.provider_tool_attach_load.map(duration_micros),
            fake_mcp_list_call_us: baseline.fake_mcp_list_call.map(duration_micros),
            active_path_messages: baseline.loaded_messages,
            tree_messages: baseline.tree_messages,
            requested_tool_calls: baseline.requested_tool_calls,
            resolved_tool_results: baseline.resolved_tool_results,
            deleted_messages: baseline.deleted_messages,
            promoted_children: baseline.promoted_children,
            truncated_messages: baseline.truncated_messages,
            gateway_ready_us: baseline.gateway_ready.map(duration_micros),
            first_token_us: baseline.first_token.map(duration_micros),
            full_response_us: baseline.full_response.map(duration_micros),
            response_bytes: baseline.response_bytes,
        }
    }
}

impl PerformanceSummary {
    /// Aggregates all duration fields that are present in the samples.
    fn from_samples(samples: &[PerformanceSample]) -> Self {
        Self {
            store_open: duration_metric(samples.iter().filter_map(|sample| sample.store_open_us)),
            active_path_load: duration_metric(
                samples
                    .iter()
                    .filter_map(|sample| sample.active_path_load_us),
            ),
            active_message_lookup: duration_metric(
                samples
                    .iter()
                    .filter_map(|sample| sample.active_message_lookup_us),
            ),
            active_path_row_load: duration_metric(
                samples
                    .iter()
                    .filter_map(|sample| sample.active_path_row_load_us),
            ),
            active_path_part_load: duration_metric(
                samples
                    .iter()
                    .filter_map(|sample| sample.active_path_part_load_us),
            ),
            tree_load: duration_metric(samples.iter().filter_map(|sample| sample.tree_load_us)),
            tree_row_load: duration_metric(
                samples.iter().filter_map(|sample| sample.tree_row_load_us),
            ),
            tree_part_load: duration_metric(
                samples.iter().filter_map(|sample| sample.tree_part_load_us),
            ),
            tool_schema_load: duration_metric(
                samples
                    .iter()
                    .filter_map(|sample| sample.tool_schema_load_us),
            ),
            context_build: duration_metric(
                samples.iter().filter_map(|sample| sample.context_build_us),
            ),
            context_active_path_load: duration_metric(
                samples
                    .iter()
                    .filter_map(|sample| sample.context_active_path_load_us),
            ),
            context_system_prompt_load: duration_metric(
                samples
                    .iter()
                    .filter_map(|sample| sample.context_system_prompt_load_us),
            ),
            context_compaction_load: duration_metric(
                samples
                    .iter()
                    .filter_map(|sample| sample.context_compaction_load_us),
            ),
            context_flatten: duration_metric(
                samples
                    .iter()
                    .filter_map(|sample| sample.context_flatten_us),
            ),
            prepare_query_turn: duration_metric(
                samples
                    .iter()
                    .filter_map(|sample| sample.prepare_query_turn_us),
            ),
            pending_tool_approval_scan: duration_metric(
                samples
                    .iter()
                    .filter_map(|sample| sample.pending_tool_approval_scan_us),
            ),
            tool_result_insert: duration_metric(
                samples
                    .iter()
                    .filter_map(|sample| sample.tool_result_insert_us),
            ),
            deny_tool_result_persist: duration_metric(
                samples
                    .iter()
                    .filter_map(|sample| sample.deny_tool_result_persist_us),
            ),
            splice_remove: duration_metric(
                samples.iter().filter_map(|sample| sample.splice_remove_us),
            ),
            truncate: duration_metric(samples.iter().filter_map(|sample| sample.truncate_us)),
            context_build_after_tool_chain: duration_metric(
                samples
                    .iter()
                    .filter_map(|sample| sample.context_build_after_tool_chain_us),
            ),
            active_path_load_100: duration_metric(
                samples
                    .iter()
                    .filter_map(|sample| sample.active_path_load_100_us),
            ),
            active_path_load_1000: duration_metric(
                samples
                    .iter()
                    .filter_map(|sample| sample.active_path_load_1000_us),
            ),
            pending_tool_approval_scan_long_path: duration_metric(
                samples
                    .iter()
                    .filter_map(|sample| sample.pending_tool_approval_scan_long_path_us),
            ),
            pending_tool_approval_scan_deep_chain: duration_metric(
                samples
                    .iter()
                    .filter_map(|sample| sample.pending_tool_approval_scan_deep_chain_us),
            ),
            prepare_query_no_tools: duration_metric(
                samples
                    .iter()
                    .filter_map(|sample| sample.prepare_query_no_tools_us),
            ),
            prepare_query_completed_tool_chain: duration_metric(
                samples
                    .iter()
                    .filter_map(|sample| sample.prepare_query_completed_tool_chain_us),
            ),
            prepare_query_requires_approval: duration_metric(
                samples
                    .iter()
                    .filter_map(|sample| sample.prepare_query_requires_approval_us),
            ),
            prepare_query_policy_denied: duration_metric(
                samples
                    .iter()
                    .filter_map(|sample| sample.prepare_query_policy_denied_us),
            ),
            splice_remove_branch_point: duration_metric(
                samples
                    .iter()
                    .filter_map(|sample| sample.splice_remove_branch_point_us),
            ),
            splice_remove_root_many_children: duration_metric(
                samples
                    .iter()
                    .filter_map(|sample| sample.splice_remove_root_many_children_us),
            ),
            splice_remove_tool_group: duration_metric(
                samples
                    .iter()
                    .filter_map(|sample| sample.splice_remove_tool_group_us),
            ),
            truncate_large_subtree: duration_metric(
                samples
                    .iter()
                    .filter_map(|sample| sample.truncate_large_subtree_us),
            ),
            context_build_plain_100: duration_metric(
                samples
                    .iter()
                    .filter_map(|sample| sample.context_build_plain_100_us),
            ),
            context_build_plain_1000: duration_metric(
                samples
                    .iter()
                    .filter_map(|sample| sample.context_build_plain_1000_us),
            ),
            context_build_with_system_prompt: duration_metric(
                samples
                    .iter()
                    .filter_map(|sample| sample.context_build_with_system_prompt_us),
            ),
            context_build_with_compaction: duration_metric(
                samples
                    .iter()
                    .filter_map(|sample| sample.context_build_with_compaction_us),
            ),
            context_build_with_image_parts: duration_metric(
                samples
                    .iter()
                    .filter_map(|sample| sample.context_build_with_image_parts_us),
            ),
            provider_tool_attach_load: duration_metric(
                samples
                    .iter()
                    .filter_map(|sample| sample.provider_tool_attach_load_us),
            ),
            fake_mcp_list_call: duration_metric(
                samples
                    .iter()
                    .filter_map(|sample| sample.fake_mcp_list_call_us),
            ),
            gateway_ready: duration_metric(
                samples.iter().filter_map(|sample| sample.gateway_ready_us),
            ),
            first_token: duration_metric(samples.iter().filter_map(|sample| sample.first_token_us)),
            full_response: duration_metric(
                samples.iter().filter_map(|sample| sample.full_response_us),
            ),
        }
    }
}

/// Converts a duration to integer microseconds for stable JSON storage.
fn duration_micros(duration: Duration) -> u64 {
    duration.as_micros().min(u128::from(u64::MAX)) as u64
}

/// Builds min/median/p95/max for a set of microsecond samples.
fn duration_metric(values: impl Iterator<Item = u64>) -> Option<DurationMetric> {
    let mut values = values.collect::<Vec<_>>();
    if values.is_empty() {
        return None;
    }

    values.sort_unstable();
    let p95_index = (values.len() * 95).div_ceil(100).saturating_sub(1);

    Some(DurationMetric {
        min_us: values[0],
        median_us: values[values.len() / 2],
        p95_us: values[p95_index],
        max_us: values[values.len() - 1],
    })
}

/// Returns all summary metrics that can be compared in both reports.
fn comparison_rows(
    baseline: &PerformanceSummary,
    current: &PerformanceSummary,
) -> Vec<PerformanceComparisonRow> {
    [
        ("store open", &baseline.store_open, &current.store_open),
        (
            "active path load",
            &baseline.active_path_load,
            &current.active_path_load,
        ),
        (
            "active message lookup",
            &baseline.active_message_lookup,
            &current.active_message_lookup,
        ),
        (
            "active path row load",
            &baseline.active_path_row_load,
            &current.active_path_row_load,
        ),
        (
            "active path part/image load",
            &baseline.active_path_part_load,
            &current.active_path_part_load,
        ),
        ("tree load", &baseline.tree_load, &current.tree_load),
        (
            "tree row load",
            &baseline.tree_row_load,
            &current.tree_row_load,
        ),
        (
            "tree part/image load",
            &baseline.tree_part_load,
            &current.tree_part_load,
        ),
        (
            "tool schema load",
            &baseline.tool_schema_load,
            &current.tool_schema_load,
        ),
        (
            "context build",
            &baseline.context_build,
            &current.context_build,
        ),
        (
            "context active path load",
            &baseline.context_active_path_load,
            &current.context_active_path_load,
        ),
        (
            "context system prompt load",
            &baseline.context_system_prompt_load,
            &current.context_system_prompt_load,
        ),
        (
            "context compaction load",
            &baseline.context_compaction_load,
            &current.context_compaction_load,
        ),
        (
            "context flatten",
            &baseline.context_flatten,
            &current.context_flatten,
        ),
        (
            "prepare query turn",
            &baseline.prepare_query_turn,
            &current.prepare_query_turn,
        ),
        (
            "pending tool approval scan",
            &baseline.pending_tool_approval_scan,
            &current.pending_tool_approval_scan,
        ),
        (
            "tool result insert",
            &baseline.tool_result_insert,
            &current.tool_result_insert,
        ),
        (
            "deny tool result persist",
            &baseline.deny_tool_result_persist,
            &current.deny_tool_result_persist,
        ),
        (
            "splice remove",
            &baseline.splice_remove,
            &current.splice_remove,
        ),
        ("truncate", &baseline.truncate, &current.truncate),
        (
            "context build after tool chain",
            &baseline.context_build_after_tool_chain,
            &current.context_build_after_tool_chain,
        ),
        (
            "active path load 100",
            &baseline.active_path_load_100,
            &current.active_path_load_100,
        ),
        (
            "active path load 1000",
            &baseline.active_path_load_1000,
            &current.active_path_load_1000,
        ),
        (
            "pending tool approval scan long path",
            &baseline.pending_tool_approval_scan_long_path,
            &current.pending_tool_approval_scan_long_path,
        ),
        (
            "pending tool approval scan deep chain",
            &baseline.pending_tool_approval_scan_deep_chain,
            &current.pending_tool_approval_scan_deep_chain,
        ),
        (
            "prepare query no tools",
            &baseline.prepare_query_no_tools,
            &current.prepare_query_no_tools,
        ),
        (
            "prepare query completed tool chain",
            &baseline.prepare_query_completed_tool_chain,
            &current.prepare_query_completed_tool_chain,
        ),
        (
            "prepare query requires approval",
            &baseline.prepare_query_requires_approval,
            &current.prepare_query_requires_approval,
        ),
        (
            "prepare query policy denied",
            &baseline.prepare_query_policy_denied,
            &current.prepare_query_policy_denied,
        ),
        (
            "splice remove branch point",
            &baseline.splice_remove_branch_point,
            &current.splice_remove_branch_point,
        ),
        (
            "splice remove root many children",
            &baseline.splice_remove_root_many_children,
            &current.splice_remove_root_many_children,
        ),
        (
            "splice remove tool group",
            &baseline.splice_remove_tool_group,
            &current.splice_remove_tool_group,
        ),
        (
            "truncate large subtree",
            &baseline.truncate_large_subtree,
            &current.truncate_large_subtree,
        ),
        (
            "context build plain 100",
            &baseline.context_build_plain_100,
            &current.context_build_plain_100,
        ),
        (
            "context build plain 1000",
            &baseline.context_build_plain_1000,
            &current.context_build_plain_1000,
        ),
        (
            "context build with system prompt",
            &baseline.context_build_with_system_prompt,
            &current.context_build_with_system_prompt,
        ),
        (
            "context build with compaction",
            &baseline.context_build_with_compaction,
            &current.context_build_with_compaction,
        ),
        (
            "context build with image parts",
            &baseline.context_build_with_image_parts,
            &current.context_build_with_image_parts,
        ),
        (
            "provider tool attach/load",
            &baseline.provider_tool_attach_load,
            &current.provider_tool_attach_load,
        ),
        (
            "fake mcp list/call",
            &baseline.fake_mcp_list_call,
            &current.fake_mcp_list_call,
        ),
        (
            "gateway ready",
            &baseline.gateway_ready,
            &current.gateway_ready,
        ),
        ("first token", &baseline.first_token, &current.first_token),
        (
            "full response",
            &baseline.full_response,
            &current.full_response,
        ),
    ]
    .into_iter()
    .filter_map(|(name, baseline, current)| {
        let baseline = baseline.as_ref()?;
        let current = current.as_ref()?;

        Some(PerformanceComparisonRow {
            name,
            baseline_median_us: baseline.median_us,
            current_median_us: current.median_us,
            change_percent: percent_change(baseline.median_us, current.median_us),
        })
    })
    .collect()
}

/// Calculates percentage change from baseline to current.
fn percent_change(baseline: u64, current: u64) -> f64 {
    if baseline == 0 {
        return 0.0;
    }

    ((current as f64 - baseline as f64) / baseline as f64) * 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarizes_duration_samples() {
        let metric = duration_metric([30, 10, 20, 40].into_iter()).unwrap();

        assert_eq!(metric.min_us, 10);
        assert_eq!(metric.median_us, 30);
        assert_eq!(metric.p95_us, 40);
        assert_eq!(metric.max_us, 40);
    }

    #[test]
    fn compares_report_medians() {
        let baseline = PerformanceReport {
            format_version: REPORT_FORMAT_VERSION,
            mode: BenchmarkMode::Conversation,
            model: "model".to_string(),
            conversation_id: Some("conversation-id".to_string()),
            runs: 2,
            samples: vec![],
            summary: PerformanceSummary {
                active_path_load: Some(DurationMetric {
                    min_us: 100,
                    median_us: 100,
                    p95_us: 100,
                    max_us: 100,
                }),
                ..PerformanceSummary::default()
            },
        };
        let current = PerformanceReport {
            summary: PerformanceSummary {
                active_path_load: Some(DurationMetric {
                    min_us: 125,
                    median_us: 125,
                    p95_us: 125,
                    max_us: 125,
                }),
                ..baseline.summary.clone()
            },
            runs: 3,
            ..baseline.clone()
        };

        let comparison = compare_reports(&baseline, &current);

        assert_eq!(comparison.rows.len(), 1);
        assert_eq!(comparison.rows[0].name, "active path load");
        assert_eq!(comparison.rows[0].change_percent, 25.0);
    }

    #[test]
    fn reads_json_report_and_compares_it() {
        let baseline = PerformanceReport {
            format_version: REPORT_FORMAT_VERSION,
            mode: BenchmarkMode::Conversation,
            model: "model".to_string(),
            conversation_id: Some("conversation-id".to_string()),
            runs: 1,
            samples: vec![PerformanceSample {
                store_open_us: Some(10),
                active_path_load_us: Some(20),
                tree_load_us: None,
                context_build_us: Some(30),
                active_path_messages: Some(1),
                tree_messages: Some(1),
                gateway_ready_us: None,
                first_token_us: None,
                full_response_us: None,
                response_bytes: None,
                ..PerformanceSample::default()
            }],
            summary: PerformanceSummary {
                store_open: Some(DurationMetric {
                    min_us: 10,
                    median_us: 10,
                    p95_us: 10,
                    max_us: 10,
                }),
                active_path_load: Some(DurationMetric {
                    min_us: 20,
                    median_us: 20,
                    p95_us: 20,
                    max_us: 20,
                }),
                tree_load: None,
                context_build: Some(DurationMetric {
                    min_us: 30,
                    median_us: 30,
                    p95_us: 30,
                    max_us: 30,
                }),
                ..PerformanceSummary::default()
            },
        };
        let current = PerformanceReport {
            summary: PerformanceSummary {
                active_path_load: Some(DurationMetric {
                    min_us: 40,
                    median_us: 40,
                    p95_us: 40,
                    max_us: 40,
                }),
                ..baseline.summary.clone()
            },
            ..baseline.clone()
        };
        let baseline_path = std::env::temp_dir().join(format!(
            "windie-baseline-{}-{}.json",
            std::process::id(),
            "read"
        ));
        let current_path = std::env::temp_dir().join(format!(
            "windie-current-{}-{}.json",
            std::process::id(),
            "read"
        ));

        std::fs::write(&baseline_path, serde_json::to_string(&baseline).unwrap()).unwrap();
        std::fs::write(&current_path, serde_json::to_string(&current).unwrap()).unwrap();

        let baseline = read_report(&baseline_path).unwrap();
        let current = read_report(&current_path).unwrap();
        let comparison = compare_reports(&baseline, &current);

        assert_eq!(comparison.rows.len(), 3);
        assert!(
            comparison
                .rows
                .iter()
                .any(|row| row.name == "active path load" && row.change_percent == 100.0)
        );

        let _ = std::fs::remove_file(baseline_path);
        let _ = std::fs::remove_file(current_path);
    }
}
