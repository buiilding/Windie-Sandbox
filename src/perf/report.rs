//! Benchmark report data and duration summarization.

use super::*;
use std::collections::BTreeMap;

/// Timings collected by one benchmark run.
pub struct PerformanceBaseline {
    pub mode: BenchmarkMode,
    pub model: ModelName,
    pub conversation_id: Option<ConversationId>,
    pub scenarios: Vec<ScenarioTiming>,
}

#[derive(Debug, Clone)]
/// One named benchmark measurement with its architectural layer and fixture.
pub struct ScenarioTiming {
    pub name: String,
    pub category: BenchmarkCategory,
    pub layer: String,
    pub fixture: String,
    pub duration: Duration,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// One named measurement stored in a repeated benchmark sample.
pub struct ScenarioSample {
    pub name: String,
    pub category: BenchmarkCategory,
    pub layer: String,
    pub fixture: String,
    pub duration_us: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Aggregated timing and fixture metadata for one named scenario.
pub struct ScenarioSummary {
    pub category: BenchmarkCategory,
    pub layer: String,
    pub fixture: String,
    pub timing: DurationMetric,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Persistent benchmark artifact written by `windie bench --json`.
pub struct PerformanceReport {
    pub format_version: u32,
    pub mode: BenchmarkMode,
    pub categories: Vec<BenchmarkCategory>,
    pub model: String,
    pub conversation_id: Option<String>,
    pub runs: usize,
    pub samples: Vec<PerformanceSample>,
    pub summary: PerformanceSummary,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
/// One serialized benchmark sample.
pub struct PerformanceSample {
    pub scenarios: Vec<ScenarioSample>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
/// Aggregated duration metrics grouped by architectural layer and scenario.
pub struct PerformanceSummary {
    pub scenarios: BTreeMap<String, ScenarioSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Summary of one duration field, in integer microseconds.
pub struct DurationMetric {
    pub min_us: u64,
    pub median_us: u64,
    pub p95_us: u64,
    pub max_us: u64,
}

impl PerformanceSample {
    /// Converts the in-memory scenario timings into JSON-safe values.
    pub(super) fn from_baseline(baseline: &PerformanceBaseline) -> Self {
        Self {
            scenarios: baseline
                .scenarios
                .iter()
                .map(|scenario| ScenarioSample {
                    name: scenario.name.clone(),
                    category: scenario.category,
                    layer: scenario.layer.clone(),
                    fixture: scenario.fixture.clone(),
                    duration_us: duration_micros(scenario.duration),
                })
                .collect(),
        }
    }
}

impl PerformanceSummary {
    /// Aggregates each named scenario across repeated benchmark samples.
    pub(super) fn from_samples(samples: &[PerformanceSample]) -> Self {
        let mut scenario_values = BTreeMap::<String, Vec<u64>>::new();
        let mut scenario_metadata = BTreeMap::<String, (BenchmarkCategory, String, String)>::new();

        for sample in samples {
            for scenario in &sample.scenarios {
                let key = scenario_name_key(scenario);
                scenario_values
                    .entry(key.clone())
                    .or_default()
                    .push(scenario.duration_us);
                scenario_metadata.entry(key).or_insert_with(|| {
                    (
                        scenario.category,
                        scenario.layer.clone(),
                        scenario.fixture.clone(),
                    )
                });
            }
        }

        let scenarios = scenario_values
            .into_iter()
            .filter_map(|(name, values)| {
                let timing = duration_metric(values.into_iter())?;
                let (category, layer, fixture) = scenario_metadata.remove(&name)?;
                Some((
                    name,
                    ScenarioSummary {
                        category,
                        layer,
                        fixture,
                        timing,
                    },
                ))
            })
            .collect();

        Self { scenarios }
    }
}

/// Uses the stable layer/name pair as the report key.
fn scenario_name_key(scenario: &ScenarioSample) -> String {
    format!("{}/{}", scenario.layer, scenario.name)
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
