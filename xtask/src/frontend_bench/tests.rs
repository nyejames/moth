use moth::benchmarking::{
    FrontendBenchmarkCounter, FrontendBenchmarkReport, FrontendBenchmarkStage,
};

use crate::case_parser::BenchmarkCase;
use crate::frontend_bench::{report_to_observations, run_one_frontend_case};

#[test]
fn report_to_observations_converts_stages_and_counters() {
    let report = FrontendBenchmarkReport {
        total_ms: 42.0,
        stages: vec![
            FrontendBenchmarkStage {
                name: "ast_ms".to_string(),
                duration_ms: 10.0,
            },
            FrontendBenchmarkStage {
                name: "hir_ms".to_string(),
                duration_ms: 5.0,
            },
        ],
        counters: vec![FrontendBenchmarkCounter {
            name: "foo".to_string(),
            value: 7.0,
        }],
    };

    let observations = report_to_observations(&report);

    assert_eq!(observations.stage_timings.len(), 2);
    assert_eq!(observations.counters.len(), 1);

    let ast = observations
        .stage_timings
        .iter()
        .find(|m| m.name == "ast_ms")
        .expect("ast stage should exist");
    assert!((ast.value - 10.0).abs() < 0.001);

    let counter = observations
        .counters
        .iter()
        .find(|m| m.name == "foo")
        .expect("foo counter should exist");
    assert!((counter.value - 7.0).abs() < 0.001);
}

#[test]
fn report_to_observations_handles_empty_stages_and_counters() {
    let report = FrontendBenchmarkReport {
        total_ms: 1.0,
        stages: Vec::new(),
        counters: Vec::new(),
    };

    let observations = report_to_observations(&report);

    assert!(observations.stage_timings.is_empty());
    assert!(observations.counters.is_empty());
}

#[test]
fn frontend_case_requires_one_path_argument() {
    let case = BenchmarkCase {
        name: "frontend_bad".to_string(),
        group_name: "core".to_string(),
        command: "frontend".to_string(),
        args: vec!["a.moth".to_string(), "b.moth".to_string()],
    };

    let error = run_one_frontend_case(&case).expect_err("extra path should fail");
    assert!(error.contains("exactly one path argument"));
}
