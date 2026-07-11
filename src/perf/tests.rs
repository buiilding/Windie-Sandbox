//! Benchmark metric, report, and comparison tests.

use super::metrics::{REPORT_FORMAT_VERSION, duration_metric};
use super::*;

fn fixed_metric(value: u64) -> DurationMetric {
    DurationMetric {
        min_us: value,
        median_us: value,
        p95_us: value,
        max_us: value,
    }
}

#[test]
fn summarizes_duration_samples() {
    let metric = duration_metric([30, 10, 20, 40].into_iter()).unwrap();

    assert_eq!(metric.min_us, 10);
    assert_eq!(metric.median_us, 30);
    assert_eq!(metric.p95_us, 40);
    assert_eq!(metric.max_us, 40);
}

#[test]
fn reads_legacy_null_metrics() {
    let sample: PerformanceSample = serde_json::from_value(serde_json::json!({
        "store_open_us": 10,
        "first_token_us": null,
        "active_path_messages": 2
    }))
    .unwrap();
    let summary: PerformanceSummary = serde_json::from_value(serde_json::json!({
        "store_open": {
            "min_us": 10,
            "median_us": 10,
            "p95_us": 10,
            "max_us": 10
        },
        "first_token": null
    }))
    .unwrap();

    assert_eq!(sample.durations_us[&MetricName::StoreOpen], 10);
    assert_eq!(sample.counts[&CountName::ActivePathMessages], 2);
    assert!(summary.get(MetricName::StoreOpen).is_some());
    assert!(summary.get(MetricName::FirstToken).is_none());
}

#[test]
fn compares_report_medians() {
    let mut baseline_summary = PerformanceSummary::default();
    baseline_summary.insert(MetricName::ActivePathLoad, fixed_metric(100));
    let baseline = PerformanceReport {
        format_version: REPORT_FORMAT_VERSION,
        mode: BenchmarkMode::Conversation,
        model: "model".to_string(),
        conversation_id: Some("conversation-id".to_string()),
        runs: 2,
        samples: vec![],
        summary: baseline_summary,
    };
    let mut current_summary = baseline.summary.clone();
    current_summary.insert(MetricName::ActivePathLoad, fixed_metric(125));
    let current = PerformanceReport {
        summary: current_summary,
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
    let mut sample = PerformanceSample::default();
    sample.durations_us.insert(MetricName::StoreOpen, 10);
    sample.durations_us.insert(MetricName::ActivePathLoad, 20);
    sample.durations_us.insert(MetricName::ContextBuild, 30);
    sample.counts.insert(CountName::ActivePathMessages, 1);
    sample.counts.insert(CountName::TreeMessages, 1);
    let mut baseline_summary = PerformanceSummary::default();
    baseline_summary.insert(MetricName::StoreOpen, fixed_metric(10));
    baseline_summary.insert(MetricName::ActivePathLoad, fixed_metric(20));
    baseline_summary.insert(MetricName::ContextBuild, fixed_metric(30));
    let baseline = PerformanceReport {
        format_version: REPORT_FORMAT_VERSION,
        mode: BenchmarkMode::Conversation,
        model: "model".to_string(),
        conversation_id: Some("conversation-id".to_string()),
        runs: 1,
        samples: vec![sample],
        summary: baseline_summary,
    };
    let mut current_summary = baseline.summary.clone();
    current_summary.insert(MetricName::ActivePathLoad, fixed_metric(40));
    let current = PerformanceReport {
        summary: current_summary,
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
