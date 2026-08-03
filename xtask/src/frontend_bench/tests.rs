use crate::bench_types::BenchmarkGroup;

use moth::benchmarking::{
    FrontendBenchmarkCounter, FrontendBenchmarkReport, FrontendBenchmarkStage,
};

use crate::benchmark_manifest::{
    BenchmarkCase, BenchmarkEntryKind, BenchmarkExpectation, BenchmarkFingerprintMode,
    BenchmarkManifest, BenchmarkRunner, BenchmarkWorkload, FrontendBenchmarkProfile,
};
use crate::frontend_bench::{report_to_observations, run_one_frontend_case};

#[test]
fn report_to_observations_converts_stages_and_counters() {
    let report = FrontendBenchmarkReport {
        total_ms: 42.0,
        warning_count: 0,
        warning_codes: Vec::new(),
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

    let observations =
        report_to_observations(&report).expect("valid frontend observations should convert");

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
fn report_to_observations_rejects_empty_stages() {
    let report = FrontendBenchmarkReport {
        total_ms: 1.0,
        warning_count: 0,
        warning_codes: Vec::new(),
        stages: Vec::new(),
        counters: Vec::new(),
    };

    let error = report_to_observations(&report).expect_err("empty frontend stages must fail");

    assert!(error.to_string().contains("at least one stage"));
}

#[test]
fn report_to_observations_sums_repeated_stages_and_validates_values() {
    let report = FrontendBenchmarkReport {
        total_ms: 1.0,
        warning_count: 0,
        warning_codes: Vec::new(),
        stages: vec![
            FrontendBenchmarkStage {
                name: "frontend.ast".to_owned(),
                duration_ms: 2.0,
            },
            FrontendBenchmarkStage {
                name: "frontend.ast".to_owned(),
                duration_ms: 3.0,
            },
        ],
        counters: Vec::new(),
    };

    let observations =
        report_to_observations(&report).expect("repeated frontend stages should sum");
    assert_eq!(observations.stage_timings.len(), 1);
    assert_eq!(observations.stage_timings[0].value, 5.0);

    let invalid_report = FrontendBenchmarkReport {
        stages: vec![FrontendBenchmarkStage {
            name: "frontend.ast".to_owned(),
            duration_ms: f64::NAN,
        }],
        ..report
    };
    let error =
        report_to_observations(&invalid_report).expect_err("non-finite frontend stages must fail");
    assert!(error.to_string().contains("finite and non-negative"));
}

#[test]
fn frontend_case_uses_typed_dev_profile_and_workload_entry() {
    let manifest = BenchmarkManifest {
        workloads: vec![BenchmarkWorkload {
            id: "fixture".to_string(),
            entry: "fixture".into(),
            entry_kind: BenchmarkEntryKind::File,
            fingerprint_mode: BenchmarkFingerprintMode::FullTree,
            fingerprint_roots: vec!["fixture".into()],
            fingerprint_excludes: vec![],
            generated_output_roots: Vec::new(),
        }],
        cases: vec![],
        manifest_path: "manifest.toml".into(),
        repository_root: "repository-root".into(),
    };
    let case = frontend_case();

    let invocation = manifest
        .frontend_invocation(&case)
        .expect("frontend invocation should resolve");
    assert_eq!(
        invocation.entry,
        std::path::PathBuf::from("repository-root").join("fixture")
    );
    assert_eq!(invocation.profile, FrontendBenchmarkProfile::Dev);

    let cli_case = BenchmarkCase {
        runner: BenchmarkRunner::Cli {
            command: crate::benchmark_manifest::CliBenchmarkCommand::Check,
            args: vec![],
        },
        ..case
    };
    let error = run_one_frontend_case(&manifest, &cli_case)
        .expect_err("CLI runner should not enter frontend execution");
    assert!(error.contains("does not declare a frontend runner"));
}

fn frontend_case() -> BenchmarkCase {
    BenchmarkCase {
        id: "frontend_fixture".to_string(),
        case_index: 0,
        workload_index: 0,
        group_name: BenchmarkGroup::Core,
        quick: false,
        expectation: BenchmarkExpectation::Clean,
        runner: BenchmarkRunner::Frontend {
            profile: FrontendBenchmarkProfile::Dev,
        },
    }
}
