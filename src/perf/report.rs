//! Benchmark report data and duration summarization.

use super::*;

/// Timings collected by one benchmark run.
///
/// Fields are optional because each benchmark mode measures a different path.
pub struct PerformanceBaseline {
    pub mode: BenchmarkMode,
    pub model: ModelName,
    pub conversation_id: Option<ConversationId>,
    pub store_open: Option<Duration>,
    pub conversation_load: Option<Duration>,
    pub head_message_lookup: Option<Duration>,
    pub path_row_load: Option<Duration>,
    pub path_part_load: Option<Duration>,
    pub tree_load: Option<Duration>,
    pub tree_row_load: Option<Duration>,
    pub tree_part_load: Option<Duration>,
    pub tool_schema_load: Option<Duration>,
    pub context_build: Option<Duration>,
    pub context_path_load: Option<Duration>,
    pub context_system_prompt_load: Option<Duration>,
    pub context_compaction_load: Option<Duration>,
    pub context_flatten: Option<Duration>,
    pub prepare_head_turn: Option<Duration>,
    pub pending_tool_approval_scan: Option<Duration>,
    pub tool_result_insert: Option<Duration>,
    pub deny_tool_result_persist: Option<Duration>,
    pub splice_remove: Option<Duration>,
    pub truncate: Option<Duration>,
    pub context_build_after_tool_chain: Option<Duration>,
    pub path_load_100: Option<Duration>,
    pub path_load_1000: Option<Duration>,
    pub pending_tool_approval_scan_long_path: Option<Duration>,
    pub pending_tool_approval_scan_deep_chain: Option<Duration>,
    pub prepare_run_head_no_tools: Option<Duration>,
    pub prepare_run_head_completed_tool_chain: Option<Duration>,
    pub prepare_run_head_requires_approval: Option<Duration>,
    pub prepare_run_head_policy_denied: Option<Duration>,
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
    #[serde(default)]
    pub categories: Vec<BenchmarkCategory>,
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
    pub path_load_us: Option<u64>,
    pub head_message_lookup_us: Option<u64>,
    pub path_row_load_us: Option<u64>,
    pub path_part_load_us: Option<u64>,
    pub tree_load_us: Option<u64>,
    pub tree_row_load_us: Option<u64>,
    pub tree_part_load_us: Option<u64>,
    #[serde(default)]
    pub tool_schema_load_us: Option<u64>,
    pub context_build_us: Option<u64>,
    pub context_path_load_us: Option<u64>,
    pub context_system_prompt_load_us: Option<u64>,
    pub context_compaction_load_us: Option<u64>,
    pub context_flatten_us: Option<u64>,
    #[serde(default)]
    pub prepare_head_turn_us: Option<u64>,
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
    pub path_load_100_us: Option<u64>,
    #[serde(default)]
    pub path_load_1000_us: Option<u64>,
    #[serde(default)]
    pub pending_tool_approval_scan_long_path_us: Option<u64>,
    #[serde(default)]
    pub pending_tool_approval_scan_deep_chain_us: Option<u64>,
    #[serde(default)]
    pub prepare_run_head_no_tools_us: Option<u64>,
    #[serde(default)]
    pub prepare_run_head_completed_tool_chain_us: Option<u64>,
    #[serde(default)]
    pub prepare_run_head_requires_approval_us: Option<u64>,
    #[serde(default)]
    pub prepare_run_head_policy_denied_us: Option<u64>,
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
    pub path_messages: Option<usize>,
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
    pub path_load: Option<DurationMetric>,
    pub head_message_lookup: Option<DurationMetric>,
    pub path_row_load: Option<DurationMetric>,
    pub path_part_load: Option<DurationMetric>,
    pub tree_load: Option<DurationMetric>,
    pub tree_row_load: Option<DurationMetric>,
    pub tree_part_load: Option<DurationMetric>,
    #[serde(default)]
    pub tool_schema_load: Option<DurationMetric>,
    pub context_build: Option<DurationMetric>,
    pub context_path_load: Option<DurationMetric>,
    pub context_system_prompt_load: Option<DurationMetric>,
    pub context_compaction_load: Option<DurationMetric>,
    pub context_flatten: Option<DurationMetric>,
    #[serde(default)]
    pub prepare_head_turn: Option<DurationMetric>,
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
    pub path_load_100: Option<DurationMetric>,
    #[serde(default)]
    pub path_load_1000: Option<DurationMetric>,
    #[serde(default)]
    pub pending_tool_approval_scan_long_path: Option<DurationMetric>,
    #[serde(default)]
    pub pending_tool_approval_scan_deep_chain: Option<DurationMetric>,
    #[serde(default)]
    pub prepare_run_head_no_tools: Option<DurationMetric>,
    #[serde(default)]
    pub prepare_run_head_completed_tool_chain: Option<DurationMetric>,
    #[serde(default)]
    pub prepare_run_head_requires_approval: Option<DurationMetric>,
    #[serde(default)]
    pub prepare_run_head_policy_denied: Option<DurationMetric>,
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

impl PerformanceSample {
    /// Converts the in-memory timing result into JSON-safe primitive values.
    pub(super) fn from_baseline(baseline: &PerformanceBaseline) -> Self {
        Self {
            store_open_us: baseline.store_open.map(duration_micros),
            path_load_us: baseline.conversation_load.map(duration_micros),
            head_message_lookup_us: baseline.head_message_lookup.map(duration_micros),
            path_row_load_us: baseline.path_row_load.map(duration_micros),
            path_part_load_us: baseline.path_part_load.map(duration_micros),
            tree_load_us: baseline.tree_load.map(duration_micros),
            tree_row_load_us: baseline.tree_row_load.map(duration_micros),
            tree_part_load_us: baseline.tree_part_load.map(duration_micros),
            tool_schema_load_us: baseline.tool_schema_load.map(duration_micros),
            context_build_us: baseline.context_build.map(duration_micros),
            context_path_load_us: baseline.context_path_load.map(duration_micros),
            context_system_prompt_load_us: baseline.context_system_prompt_load.map(duration_micros),
            context_compaction_load_us: baseline.context_compaction_load.map(duration_micros),
            context_flatten_us: baseline.context_flatten.map(duration_micros),
            prepare_head_turn_us: baseline.prepare_head_turn.map(duration_micros),
            pending_tool_approval_scan_us: baseline.pending_tool_approval_scan.map(duration_micros),
            tool_result_insert_us: baseline.tool_result_insert.map(duration_micros),
            deny_tool_result_persist_us: baseline.deny_tool_result_persist.map(duration_micros),
            splice_remove_us: baseline.splice_remove.map(duration_micros),
            truncate_us: baseline.truncate.map(duration_micros),
            context_build_after_tool_chain_us: baseline
                .context_build_after_tool_chain
                .map(duration_micros),
            path_load_100_us: baseline.path_load_100.map(duration_micros),
            path_load_1000_us: baseline.path_load_1000.map(duration_micros),
            pending_tool_approval_scan_long_path_us: baseline
                .pending_tool_approval_scan_long_path
                .map(duration_micros),
            pending_tool_approval_scan_deep_chain_us: baseline
                .pending_tool_approval_scan_deep_chain
                .map(duration_micros),
            prepare_run_head_no_tools_us: baseline.prepare_run_head_no_tools.map(duration_micros),
            prepare_run_head_completed_tool_chain_us: baseline
                .prepare_run_head_completed_tool_chain
                .map(duration_micros),
            prepare_run_head_requires_approval_us: baseline
                .prepare_run_head_requires_approval
                .map(duration_micros),
            prepare_run_head_policy_denied_us: baseline
                .prepare_run_head_policy_denied
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
            path_messages: baseline.loaded_messages,
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
    pub(super) fn from_samples(samples: &[PerformanceSample]) -> Self {
        Self {
            store_open: duration_metric(samples.iter().filter_map(|sample| sample.store_open_us)),
            path_load: duration_metric(samples.iter().filter_map(|sample| sample.path_load_us)),
            head_message_lookup: duration_metric(
                samples
                    .iter()
                    .filter_map(|sample| sample.head_message_lookup_us),
            ),
            path_row_load: duration_metric(
                samples.iter().filter_map(|sample| sample.path_row_load_us),
            ),
            path_part_load: duration_metric(
                samples.iter().filter_map(|sample| sample.path_part_load_us),
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
            context_path_load: duration_metric(
                samples
                    .iter()
                    .filter_map(|sample| sample.context_path_load_us),
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
            prepare_head_turn: duration_metric(
                samples
                    .iter()
                    .filter_map(|sample| sample.prepare_head_turn_us),
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
            path_load_100: duration_metric(
                samples.iter().filter_map(|sample| sample.path_load_100_us),
            ),
            path_load_1000: duration_metric(
                samples.iter().filter_map(|sample| sample.path_load_1000_us),
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
            prepare_run_head_no_tools: duration_metric(
                samples
                    .iter()
                    .filter_map(|sample| sample.prepare_run_head_no_tools_us),
            ),
            prepare_run_head_completed_tool_chain: duration_metric(
                samples
                    .iter()
                    .filter_map(|sample| sample.prepare_run_head_completed_tool_chain_us),
            ),
            prepare_run_head_requires_approval: duration_metric(
                samples
                    .iter()
                    .filter_map(|sample| sample.prepare_run_head_requires_approval_us),
            ),
            prepare_run_head_policy_denied: duration_metric(
                samples
                    .iter()
                    .filter_map(|sample| sample.prepare_run_head_policy_denied_us),
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
pub(super) fn duration_micros(duration: Duration) -> u64 {
    duration.as_micros().min(u128::from(u64::MAX)) as u64
}

/// Builds min/median/p95/max for a set of microsecond samples.
pub(super) fn duration_metric(values: impl Iterator<Item = u64>) -> Option<DurationMetric> {
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
