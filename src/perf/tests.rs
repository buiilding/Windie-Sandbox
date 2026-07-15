//! Performance tests.

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
        mode: BenchmarkMode::Local,
        categories: BenchmarkCategory::all(),
        model: "model".to_string(),
        conversation_id: None,
        runs: 2,
        samples: vec![],
        summary: PerformanceSummary {
            path_load: Some(DurationMetric {
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
            path_load: Some(DurationMetric {
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
    assert_eq!(comparison.rows[0].name, "path load");
    assert_eq!(comparison.rows[0].change_percent, 25.0);
}

#[test]
fn reads_json_report_and_compares_it() {
    let baseline = PerformanceReport {
        format_version: REPORT_FORMAT_VERSION,
        mode: BenchmarkMode::Local,
        categories: BenchmarkCategory::all(),
        model: "model".to_string(),
        conversation_id: None,
        runs: 1,
        samples: vec![PerformanceSample {
            store_open_us: Some(10),
            path_load_us: Some(20),
            tree_load_us: None,
            context_build_us: Some(30),
            path_messages: Some(1),
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
            path_load: Some(DurationMetric {
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
            path_load: Some(DurationMetric {
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
            .any(|row| row.name == "path load" && row.change_percent == 100.0)
    );

    let _ = std::fs::remove_file(baseline_path);
    let _ = std::fs::remove_file(current_path);
}
