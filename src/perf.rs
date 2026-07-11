//! Performance benchmark facade.
//!
//! Scenario setup and execution live in `scenarios`; stable report types,
//! serialization, statistics, and comparisons live in `report`.

mod metrics;
mod scenarios;

use serde::{Deserialize, Serialize};

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

#[cfg(test)]
pub use metrics::PerformanceComparisonRow;
pub use metrics::{
    CountName, DurationMetric, MetricName, PerformanceBaseline, PerformanceComparison,
    PerformanceReport, PerformanceSample, PerformanceSummary, compare_reports, read_report,
};
pub use scenarios::{run, run_report};

#[cfg(test)]
mod tests;
