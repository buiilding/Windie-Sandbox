//! Performance measurement and comparison.
//!
//! This module owns lightweight timing for the current local CLI/query path,
//! repeated benchmark reports, JSON benchmark artifacts, and report comparison.
//! Local and conversation benchmarks avoid provider calls. Live benchmarks are
//! explicit because they send a real provider request.

use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::context::{ContextBuilder, ContextParts};
use crate::conversation::{ConversationId, Message, Role};
use crate::gateway::{BifrostGateway, GatewayUrl};
use crate::llm::{BaseUrl, BifrostClient, ModelName};
use crate::store::Store;

const BENCH_PROMPT: &str = "Reply with exactly: ok";

const REPORT_FORMAT_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
/// Benchmark mode selected by the CLI.
pub enum BenchmarkMode {
    Conversation,
    #[serde(rename = "ls")]
    List,
    Local,
    Live,
}

impl BenchmarkMode {
    /// Returns the mode label printed in benchmark output.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Conversation => "conversation",
            Self::List => "ls",
            Self::Local => "local",
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
    pub context_build: Option<Duration>,
    pub context_active_path_load: Option<Duration>,
    pub context_system_prompt_load: Option<Duration>,
    pub context_compaction_load: Option<Duration>,
    pub context_flatten: Option<Duration>,
    pub list_load: Option<Duration>,
    pub loaded_messages: Option<usize>,
    pub tree_messages: Option<usize>,
    pub listed_conversations: Option<usize>,
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
    pub context_build_us: Option<u64>,
    pub context_active_path_load_us: Option<u64>,
    pub context_system_prompt_load_us: Option<u64>,
    pub context_compaction_load_us: Option<u64>,
    pub context_flatten_us: Option<u64>,
    pub conversation_list_load_us: Option<u64>,
    pub active_path_messages: Option<usize>,
    pub tree_messages: Option<usize>,
    pub conversations: Option<usize>,
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
    pub context_build: Option<DurationMetric>,
    pub context_active_path_load: Option<DurationMetric>,
    pub context_system_prompt_load: Option<DurationMetric>,
    pub context_compaction_load: Option<DurationMetric>,
    pub context_flatten: Option<DurationMetric>,
    pub conversation_list_load: Option<DurationMetric>,
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

/// Runs the selected benchmark mode.
///
/// Local and conversation modes are free/local. Live mode requires Bifrost and
/// sends a tiny real model request.
pub async fn run(
    mode: BenchmarkMode,
    conversation_id: Option<ConversationId>,
    gateway_url: GatewayUrl,
    base_url: BaseUrl,
    model: ModelName,
) -> Result<PerformanceBaseline> {
    let (
        store_open,
        conversation_load,
        active_message_lookup,
        active_path_row_load,
        active_path_part_load,
        tree_load,
        tree_row_load,
        tree_part_load,
        context_build,
        context_active_path_load,
        context_system_prompt_load,
        context_compaction_load,
        context_flatten,
        list_load,
        loaded_messages,
        tree_messages,
        listed_conversations,
    ) = match mode {
        BenchmarkMode::Local => {
            let store_started = Instant::now();
            let _store = Store::open()?;

            (
                Some(store_started.elapsed()),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
            )
        }
        BenchmarkMode::Conversation => {
            let store_started = Instant::now();
            let store = Store::open()?;
            let store_open = store_started.elapsed();
            let conversation_id = conversation_id
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

            (
                Some(store_open),
                Some(conversation_load),
                Some(active_message_lookup),
                Some(active_path_row_load),
                Some(active_path_part_load),
                Some(tree_load),
                Some(tree_row_load),
                Some(tree_part_load),
                Some(context_build),
                Some(context_active_path_load),
                Some(context_system_prompt_load),
                Some(context_compaction_load),
                Some(context_flatten),
                None,
                Some(loaded_messages),
                Some(tree_messages),
                None,
            )
        }
        BenchmarkMode::List => {
            let store_started = Instant::now();
            let store = Store::open()?;
            let store_open = store_started.elapsed();

            let list_started = Instant::now();
            let listed_conversations = store.list_conversations()?.len();
            let list_load = list_started.elapsed();

            (
                Some(store_open),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(list_load),
                None,
                None,
                Some(listed_conversations),
            )
        }
        BenchmarkMode::Live => (
            None, None, None, None, None, None, None, None, None, None, None, None, None, None,
            None, None, None,
        ),
    };

    let (gateway_ready, first_token, full_response, response_bytes) = if mode == BenchmarkMode::Live
    {
        let gateway = BifrostGateway::new(gateway_url);
        let gateway_started = Instant::now();
        gateway.require_running().await?;
        let gateway_ready = Some(gateway_started.elapsed());
        let (first_token, full_response, response_bytes) =
            run_live_request(&base_url, &model).await?;
        (
            gateway_ready,
            first_token,
            Some(full_response),
            Some(response_bytes),
        )
    } else {
        (None, None, None, None)
    };

    Ok(PerformanceBaseline {
        mode,
        model,
        conversation_id,
        store_open,
        conversation_load,
        active_message_lookup,
        active_path_row_load,
        active_path_part_load,
        tree_load,
        tree_row_load,
        tree_part_load,
        context_build,
        context_active_path_load,
        context_system_prompt_load,
        context_compaction_load,
        context_flatten,
        list_load,
        loaded_messages,
        tree_messages,
        listed_conversations,
        gateway_ready,
        first_token,
        full_response,
        response_bytes,
    })
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
        .stream(&messages, |delta| {
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
            context_build_us: baseline.context_build.map(duration_micros),
            context_active_path_load_us: baseline.context_active_path_load.map(duration_micros),
            context_system_prompt_load_us: baseline.context_system_prompt_load.map(duration_micros),
            context_compaction_load_us: baseline.context_compaction_load.map(duration_micros),
            context_flatten_us: baseline.context_flatten.map(duration_micros),
            conversation_list_load_us: baseline.list_load.map(duration_micros),
            active_path_messages: baseline.loaded_messages,
            tree_messages: baseline.tree_messages,
            conversations: baseline.listed_conversations,
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
            conversation_list_load: duration_metric(
                samples
                    .iter()
                    .filter_map(|sample| sample.conversation_list_load_us),
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
            "conversation list load",
            &baseline.conversation_list_load,
            &current.conversation_list_load,
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
                conversation_list_load_us: None,
                active_path_messages: Some(1),
                tree_messages: Some(1),
                conversations: None,
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

    #[test]
    fn compares_conversation_list_load() {
        let baseline = PerformanceSummary {
            conversation_list_load: Some(DurationMetric {
                min_us: 50,
                median_us: 50,
                p95_us: 50,
                max_us: 50,
            }),
            ..PerformanceSummary::default()
        };
        let current = PerformanceSummary {
            conversation_list_load: Some(DurationMetric {
                min_us: 75,
                median_us: 75,
                p95_us: 75,
                max_us: 75,
            }),
            ..baseline.clone()
        };

        let rows = comparison_rows(&baseline, &current);

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "conversation list load");
        assert_eq!(rows[0].change_percent, 50.0);
    }
}
