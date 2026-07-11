//! Benchmark report schema, serialization, statistics, and comparison.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::BenchmarkMode;
use crate::conversation::ConversationId;
use crate::llm::ModelName;

pub(super) const REPORT_FORMAT_VERSION: u32 = 6;

macro_rules! metric_catalog {
    ($name:ident; $( $variant:ident => ($key:literal, $label:literal) ),+ $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
        pub enum $name { $( $variant ),+ }

        impl $name {
            pub const ALL: &[Self] = &[$( Self::$variant ),+];

            pub fn key(self) -> &'static str {
                match self { $( Self::$variant => $key ),+ }
            }

            pub fn label(self) -> &'static str {
                match self { $( Self::$variant => $label ),+ }
            }

            fn from_key(key: &str) -> Option<Self> {
                Self::ALL.iter().copied().find(|metric| metric.key() == key)
            }
        }
    };
}

metric_catalog! {
    MetricName;
    StoreOpen => ("store_open", "store open"),
    ActivePathLoad => ("active_path_load", "active path load"),
    ActiveMessageLookup => ("active_message_lookup", "active message lookup"),
    ActivePathRowLoad => ("active_path_row_load", "active path row load"),
    ActivePathPartLoad => ("active_path_part_load", "active path part/image load"),
    TreeLoad => ("tree_load", "tree load"),
    TreeRowLoad => ("tree_row_load", "tree row load"),
    TreePartLoad => ("tree_part_load", "tree part/image load"),
    ToolSchemaLoad => ("tool_schema_load", "tool schema load"),
    ContextBuild => ("context_build", "context build"),
    ContextActivePathLoad => ("context_active_path_load", "context active path load"),
    ContextSystemPromptLoad => ("context_system_prompt_load", "context system prompt load"),
    ContextCompactionLoad => ("context_compaction_load", "context compaction load"),
    ContextFlatten => ("context_flatten", "context flatten"),
    PrepareQueryTurn => ("prepare_query_turn", "prepare query turn"),
    PendingToolApprovalScan => ("pending_tool_approval_scan", "pending tool approval scan"),
    ToolResultInsert => ("tool_result_insert", "tool result insert"),
    DenyToolResultPersist => ("deny_tool_result_persist", "deny tool result persist"),
    SpliceRemove => ("splice_remove", "splice remove"),
    Truncate => ("truncate", "truncate"),
    ContextBuildAfterToolChain => ("context_build_after_tool_chain", "context build after tool chain"),
    ActivePathLoad100 => ("active_path_load_100", "active path load 100"),
    ActivePathLoad1000 => ("active_path_load_1000", "active path load 1000"),
    PendingToolApprovalScanLongPath => ("pending_tool_approval_scan_long_path", "pending tool approval scan long path"),
    PendingToolApprovalScanDeepChain => ("pending_tool_approval_scan_deep_chain", "pending tool approval scan deep chain"),
    PrepareQueryNoTools => ("prepare_query_no_tools", "prepare query no tools"),
    PrepareQueryCompletedToolChain => ("prepare_query_completed_tool_chain", "prepare query completed tool chain"),
    PrepareQueryRequiresApproval => ("prepare_query_requires_approval", "prepare query requires approval"),
    PrepareQueryPolicyDenied => ("prepare_query_policy_denied", "prepare query policy denied"),
    SpliceRemoveBranchPoint => ("splice_remove_branch_point", "splice remove branch point"),
    SpliceRemoveRootManyChildren => ("splice_remove_root_many_children", "splice remove root many children"),
    SpliceRemoveToolGroup => ("splice_remove_tool_group", "splice remove tool group"),
    TruncateLargeSubtree => ("truncate_large_subtree", "truncate large subtree"),
    ContextBuildPlain100 => ("context_build_plain_100", "context build plain 100"),
    ContextBuildPlain1000 => ("context_build_plain_1000", "context build plain 1000"),
    ContextBuildWithSystemPrompt => ("context_build_with_system_prompt", "context build with system prompt"),
    ContextBuildWithCompaction => ("context_build_with_compaction", "context build with compaction"),
    ContextBuildWithImageParts => ("context_build_with_image_parts", "context build with image parts"),
    ProviderToolAttachLoad => ("provider_tool_attach_load", "provider tool attach/load"),
    FakeMcpListCall => ("fake_mcp_list_call", "fake mcp list/call"),
    DurableStreamJournal => ("durable_stream_journal", "durable stream journal (500 events)"),
    InspectionSnapshot1000 => ("inspection_snapshot_1000", "inspection snapshot 1000"),
    ForkConversation1000 => ("fork_conversation_1000", "fork conversation 1000"),
    RunActionLifecycle => ("run_action_lifecycle", "run action lifecycle"),
    RunAdmissionContention => ("run_admission_contention", "run admission contention"),
    FakeMcpCatalogSingleflight => ("fake_mcp_catalog_singleflight", "fake MCP catalog single-flight"),
    GatewayReady => ("gateway_ready", "gateway ready"),
    FirstToken => ("first_token", "first token"),
    FullResponse => ("full_response", "full response"),
}

metric_catalog! {
    CountName;
    ActivePathMessages => ("active_path_messages", "active path messages"),
    TreeMessages => ("tree_messages", "tree messages"),
    RequestedToolCalls => ("requested_tool_calls", "requested tool calls"),
    ResolvedToolResults => ("resolved_tool_results", "resolved tool results"),
    DeletedMessages => ("deleted_messages", "deleted messages"),
    PromotedChildren => ("promoted_children", "promoted children"),
    TruncatedMessages => ("truncated_messages", "truncated messages"),
    ResponseBytes => ("response_bytes", "response bytes"),
    ProviderCatalogStarts => ("provider_catalog_starts", "provider catalog process starts"),
}

/// Timings and counts collected by one benchmark run.
pub struct PerformanceBaseline {
    pub mode: BenchmarkMode,
    pub model: ModelName,
    pub conversation_id: Option<ConversationId>,
    pub(super) durations: BTreeMap<MetricName, Duration>,
    pub(super) counts: BTreeMap<CountName, usize>,
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
#[derive(Debug, Clone, Default)]
pub struct PerformanceSample {
    pub(super) durations_us: BTreeMap<MetricName, u64>,
    pub(super) counts: BTreeMap<CountName, usize>,
}

/// Aggregated duration metrics across all benchmark samples.
#[derive(Debug, Clone, Default)]
pub struct PerformanceSummary {
    metrics: BTreeMap<MetricName, DurationMetric>,
}

/// Summary of one duration field, in integer microseconds.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DurationMetric {
    pub min_us: u64,
    pub median_us: u64,
    pub p95_us: u64,
    pub max_us: u64,
}

impl Serialize for PerformanceSample {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut values = BTreeMap::new();
        for (name, value) in &self.durations_us {
            values.insert(format!("{}_us", name.key()), *value);
        }
        for (name, value) in &self.counts {
            values.insert(name.key().to_string(), *value as u64);
        }
        values.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for PerformanceSample {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let values = BTreeMap::<String, Option<u64>>::deserialize(deserializer)?;
        let mut sample = Self::default();
        for (key, value) in values {
            let Some(value) = value else { continue };
            if let Some(metric) = key.strip_suffix("_us").and_then(MetricName::from_key) {
                sample.durations_us.insert(metric, value);
            } else if let Some(count) = CountName::from_key(&key) {
                let value = usize::try_from(value).map_err(serde::de::Error::custom)?;
                sample.counts.insert(count, value);
            }
        }
        Ok(sample)
    }
}

impl Serialize for PerformanceSummary {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.metrics
            .iter()
            .map(|(name, metric)| (name.key(), metric))
            .collect::<BTreeMap<_, _>>()
            .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for PerformanceSummary {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let values = BTreeMap::<String, Option<DurationMetric>>::deserialize(deserializer)?;
        Ok(Self {
            metrics: values
                .into_iter()
                .filter_map(|(key, value)| MetricName::from_key(&key).zip(value))
                .collect(),
        })
    }
}

impl PerformanceBaseline {
    pub fn durations(&self) -> impl Iterator<Item = (MetricName, Duration)> + '_ {
        MetricName::ALL.iter().filter_map(|name| {
            self.durations
                .get(name)
                .copied()
                .map(|value| (*name, value))
        })
    }

    pub fn counts(&self) -> impl Iterator<Item = (CountName, usize)> + '_ {
        CountName::ALL
            .iter()
            .filter_map(|name| self.counts.get(name).copied().map(|value| (*name, value)))
    }

    pub(super) fn record(&mut self, name: MetricName, value: Duration) {
        self.durations.insert(name, value);
    }

    pub(super) fn record_optional(&mut self, name: MetricName, value: Option<Duration>) {
        if let Some(value) = value {
            self.record(name, value);
        }
    }

    pub(super) fn count(&mut self, name: CountName, value: usize) {
        self.counts.insert(name, value);
    }
}

impl PerformanceSummary {
    pub fn metrics(&self) -> impl Iterator<Item = (MetricName, &DurationMetric)> {
        MetricName::ALL
            .iter()
            .filter_map(|name| self.metrics.get(name).map(|value| (*name, value)))
    }

    pub fn get(&self, name: MetricName) -> Option<&DurationMetric> {
        self.metrics.get(&name)
    }

    #[cfg(test)]
    pub fn insert(&mut self, name: MetricName, metric: DurationMetric) {
        self.metrics.insert(name, metric);
    }
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
impl PerformanceSample {
    /// Converts one in-memory timing result into JSON-safe primitive values.
    pub(super) fn from_baseline(baseline: &PerformanceBaseline) -> Self {
        Self {
            durations_us: baseline
                .durations()
                .map(|(name, duration)| (name, duration_micros(duration)))
                .collect(),
            counts: baseline.counts().collect(),
        }
    }
}

impl PerformanceSummary {
    /// Aggregates every duration present in at least one sample.
    pub(super) fn from_samples(samples: &[PerformanceSample]) -> Self {
        Self {
            metrics: MetricName::ALL
                .iter()
                .filter_map(|name| {
                    duration_metric(
                        samples
                            .iter()
                            .filter_map(|sample| sample.durations_us.get(name).copied()),
                    )
                    .map(|metric| (*name, metric))
                })
                .collect(),
        }
    }
}

/// Converts a duration to integer microseconds for stable JSON storage.
fn duration_micros(duration: Duration) -> u64 {
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

/// Returns all summary metrics that can be compared in both reports.
fn comparison_rows(
    baseline: &PerformanceSummary,
    current: &PerformanceSummary,
) -> Vec<PerformanceComparisonRow> {
    MetricName::ALL
        .iter()
        .filter_map(|name| {
            let baseline = baseline.get(*name)?;
            let current = current.get(*name)?;
            Some(PerformanceComparisonRow {
                name: name.label(),
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
