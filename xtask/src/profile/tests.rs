use super::select_profile_cases;
use crate::benchmark_manifest::{
    BenchmarkCase, BenchmarkExpectation, BenchmarkManifest, BenchmarkRunner, BenchmarkWorkload,
    CliBenchmarkCommand, FrontendBenchmarkProfile,
};

#[test]
fn profile_selection_rejects_frontend_cases_clearly() {
    let manifest = manifest();

    let error = select_profile_cases(&manifest, Some("frontend_case"))
        .expect_err("frontend profiling must be rejected");

    assert!(error.contains("Frontend benchmark case 'frontend_case'"));
    assert!(error.contains("cannot be profiled with Samply"));
    assert!(error.contains("CLI benchmark case"));
}

#[test]
fn unfiltered_profile_selection_keeps_only_cli_cases() {
    let manifest = manifest();

    let cases = select_profile_cases(&manifest, None).expect("CLI cases should be selected");

    assert_eq!(cases.len(), 1);
    assert_eq!(cases[0].id, "cli_case");
}

fn manifest() -> BenchmarkManifest {
    BenchmarkManifest {
        workloads: vec![BenchmarkWorkload {
            id: "fixture".to_owned(),
            entry: "fixture.moth".into(),
            fingerprint_roots: vec!["fixture.moth".into()],
            fingerprint_excludes: Vec::new(),
        }],
        cases: vec![
            BenchmarkCase {
                id: "cli_case".to_owned(),
                workload_index: 0,
                group_name: "core".to_owned(),
                quick: false,
                expectation: BenchmarkExpectation::Clean,
                runner: BenchmarkRunner::Cli {
                    command: CliBenchmarkCommand::Check,
                    args: Vec::new(),
                },
            },
            BenchmarkCase {
                id: "frontend_case".to_owned(),
                workload_index: 0,
                group_name: "core".to_owned(),
                quick: false,
                expectation: BenchmarkExpectation::Clean,
                runner: BenchmarkRunner::Frontend {
                    profile: FrontendBenchmarkProfile::Dev,
                },
            },
        ],
        manifest_path: "manifest.toml".into(),
        repository_root: ".".into(),
    }
}
