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

/// Difference for one comparable named scenario.
#[derive(Debug, Clone, PartialEq)]
pub struct PerformanceComparisonRow {
    pub name: String,
    pub baseline_median_us: u64,
    pub current_median_us: u64,
    pub change_percent: f64,
}

/// Compares median duration metrics for scenarios present in both reports.
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

/// Returns all named scenarios that can be compared in both reports.
pub(super) fn comparison_rows(
    baseline: &PerformanceSummary,
    current: &PerformanceSummary,
) -> Vec<PerformanceComparisonRow> {
    baseline
        .scenarios
        .iter()
        .filter_map(|(name, baseline_scenario)| {
            let current_scenario = current.scenarios.get(name)?;
            Some(PerformanceComparisonRow {
                name: name.clone(),
                baseline_median_us: baseline_scenario.timing.median_us,
                current_median_us: current_scenario.timing.median_us,
                change_percent: percent_change(
                    baseline_scenario.timing.median_us,
                    current_scenario.timing.median_us,
                ),
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
