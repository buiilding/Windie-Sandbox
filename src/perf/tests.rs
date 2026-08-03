//! Performance report tests.

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
fn uses_repository_baseline_path() {
    assert_eq!(
        default_baseline_path().unwrap(),
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("benches")
            .join("baseline.json")
    );
}

#[test]
fn compares_named_scenario_medians() {
    let baseline = PerformanceReport {
        format_version: REPORT_FORMAT_VERSION,
        mode: BenchmarkMode::Local,
        categories: BenchmarkCategory::deterministic(),
        model: "model".to_string(),
        conversation_id: None,
        runs: 2,
        samples: vec![],
        summary: PerformanceSummary {
            scenarios: [(
                "runtime/prepare_plain_completed_turn".to_string(),
                ScenarioSummary {
                    category: BenchmarkCategory::Runtime,
                    layer: "runtime".to_string(),
                    fixture: "100-message path, no tool calls".to_string(),
                    timing: DurationMetric {
                        min_us: 100,
                        median_us: 100,
                        p95_us: 100,
                        max_us: 100,
                    },
                },
            )]
            .into_iter()
            .collect(),
        },
    };
    let current = PerformanceReport {
        summary: PerformanceSummary {
            scenarios: [(
                "runtime/prepare_plain_completed_turn".to_string(),
                ScenarioSummary {
                    category: BenchmarkCategory::Runtime,
                    layer: "runtime".to_string(),
                    fixture: "100-message path, no tool calls".to_string(),
                    timing: DurationMetric {
                        min_us: 125,
                        median_us: 125,
                        p95_us: 125,
                        max_us: 125,
                    },
                },
            )]
            .into_iter()
            .collect(),
        },
        runs: 3,
        ..baseline.clone()
    };

    let comparison = compare_reports(&baseline, &current);

    assert_eq!(comparison.rows.len(), 1);
    assert_eq!(
        comparison.rows[0].name,
        "runtime/prepare_plain_completed_turn"
    );
    assert_eq!(comparison.rows[0].change_percent, 25.0);
}

#[test]
fn aggregates_named_scenarios() {
    let baseline = PerformanceBaseline {
        mode: BenchmarkMode::Local,
        model: ModelName::new("model"),
        conversation_id: None,
        scenarios: vec![ScenarioTiming {
            name: "store_open".to_string(),
            category: BenchmarkCategory::Persistence,
            layer: "storage".to_string(),
            fixture: "fresh database".to_string(),
            duration: Duration::from_micros(10),
        }],
    };
    let sample = PerformanceSample::from_baseline(&baseline);
    let summary = PerformanceSummary::from_samples(&[sample]);

    assert_eq!(
        summary
            .scenarios
            .get("storage/store_open")
            .unwrap()
            .timing
            .median_us,
        10
    );
}
