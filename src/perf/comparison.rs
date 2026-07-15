//! Benchmark report comparison.

use super::*;

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

/// Returns all summary metrics that can be compared in both reports.
pub(super) fn comparison_rows(
    baseline: &PerformanceSummary,
    current: &PerformanceSummary,
) -> Vec<PerformanceComparisonRow> {
    [
        ("store open", &baseline.store_open, &current.store_open),
        ("path load", &baseline.path_load, &current.path_load),
        (
            "head message lookup",
            &baseline.head_message_lookup,
            &current.head_message_lookup,
        ),
        (
            "path row load",
            &baseline.path_row_load,
            &current.path_row_load,
        ),
        (
            "path part/image load",
            &baseline.path_part_load,
            &current.path_part_load,
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
            "context path load",
            &baseline.context_path_load,
            &current.context_path_load,
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
            "prepare run head turn",
            &baseline.prepare_head_turn,
            &current.prepare_head_turn,
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
            "path load 100",
            &baseline.path_load_100,
            &current.path_load_100,
        ),
        (
            "path load 1000",
            &baseline.path_load_1000,
            &current.path_load_1000,
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
            &baseline.prepare_run_head_no_tools,
            &current.prepare_run_head_no_tools,
        ),
        (
            "prepare query completed tool chain",
            &baseline.prepare_run_head_completed_tool_chain,
            &current.prepare_run_head_completed_tool_chain,
        ),
        (
            "prepare query requires approval",
            &baseline.prepare_run_head_requires_approval,
            &current.prepare_run_head_requires_approval,
        ),
        (
            "prepare query policy denied",
            &baseline.prepare_run_head_policy_denied,
            &current.prepare_run_head_policy_denied,
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
pub(super) fn percent_change(baseline: u64, current: u64) -> f64 {
    if baseline == 0 {
        return 0.0;
    }

    ((current as f64 - baseline as f64) / baseline as f64) * 100.0
}
