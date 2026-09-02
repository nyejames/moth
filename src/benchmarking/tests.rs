use crate::benchmarking::frontend::{
    FrontendBenchmarkBuildProfile, FrontendBenchmarkFailureKind, FrontendBenchmarkOptions,
    run_frontend_benchmark,
};
use std::io::Write;

// Frontend benchmarks use the process-global timing and counter stores. Share
// the facade-owned test lock with timing, instrumentation and build tests so
// parallel workspace execution cannot interleave collection sessions.
fn benchmark_test_guard() -> std::sync::MutexGuard<'static, ()> {
    crate::timing::lock_instrumentation_tests()
}

#[test]
fn frontend_benchmark_runs_for_simple_file() {
    let _guard = benchmark_test_guard();
    #[cfg(all(feature = "timers", feature = "benchmark_counters"))]
    let _counter_capture =
        crate::compiler_frontend::instrumentation::capture_frontend_counters_for_test();

    let temp_dir = tempfile::tempdir().expect("should create temp dir");
    let file_path = temp_dir.path().join("test.moth");

    {
        let mut file = std::fs::File::create(&file_path).expect("should create file");
        file.write_all(b"x = 1\n").expect("should write to file");
    }

    let options = FrontendBenchmarkOptions {
        entry_path: file_path,
        build_profile: FrontendBenchmarkBuildProfile::Dev,
    };

    let report = run_frontend_benchmark(options).expect("benchmark should succeed");

    // `total_ms` is a wall-clock measurement, so its value proves nothing about the work done:
    // on a coarse clock a real compile can still measure zero. The report's contract is that the
    // number is a usable measurement, and the stage rows below are the evidence of work.
    assert!(
        report.total_ms.is_finite() && report.total_ms >= 0.0,
        "total time must be a usable measurement: {}",
        report.total_ms
    );
    assert_eq!(report.warning_count, 0);
    assert!(report.warning_codes.is_empty());

    // Stage timings are collected when `timers` is enabled.
    #[cfg(feature = "timers")]
    assert_frontend_stage_rows(&report);

    // Counters additionally require `benchmark_counters`.
    #[cfg(all(feature = "timers", feature = "benchmark_counters"))]
    assert!(
        !report.counters.is_empty(),
        "counters should be collected when timers and benchmark_counters are enabled"
    );
}

/// Assert the stage rows a successful single-file frontend benchmark must report.
///
/// WHAT: every row names a schema metric exactly once with a usable duration, and the frontend
///       spine a single-file dev build always runs is present.
/// WHY: a non-empty stage list also passes when the collector reports one unrelated row or the
///      same row twice. Naming the required stages makes the report evidence that the frontend
///      actually ran, without depending on how long any stage took.
#[cfg(feature = "timers")]
fn assert_frontend_stage_rows(report: &crate::benchmarking::FrontendBenchmarkReport) {
    use crate::benchmarking::{
        TIMING_FRONTEND_AST_TOTAL_NAME, TIMING_FRONTEND_HIR_NAME, TIMING_FRONTEND_PREPARE_NAME,
        TIMING_SCHEMA_METRIC_NAMES,
    };

    let mut reported: Vec<&str> = report
        .stages
        .iter()
        .map(|stage| stage.name.as_str())
        .collect();
    reported.sort_unstable();
    let mut unique = reported.clone();
    unique.dedup();
    assert_eq!(
        unique, reported,
        "each stage must be aggregated into one row: {reported:?}"
    );

    for stage in &report.stages {
        assert!(
            TIMING_SCHEMA_METRIC_NAMES.contains(&stage.name.as_str()),
            "stage '{}' is not a name in the timing schema",
            stage.name
        );
        assert!(
            stage.duration_ms.is_finite() && stage.duration_ms >= 0.0,
            "stage '{}' must report a usable duration: {}",
            stage.name,
            stage.duration_ms
        );
    }

    for required in [
        TIMING_FRONTEND_PREPARE_NAME,
        TIMING_FRONTEND_AST_TOTAL_NAME,
        TIMING_FRONTEND_HIR_NAME,
    ] {
        assert!(
            reported.contains(&required),
            "a single-file frontend benchmark must report '{required}': {reported:?}"
        );
    }
}

#[test]
fn frontend_benchmark_retains_warning_count_and_codes() {
    let _guard = benchmark_test_guard();
    let temp_dir = tempfile::tempdir().expect("should create temp dir");
    let file_path = temp_dir.path().join("warning.moth");
    let warning_source = "\
value ~= \"hello\"
result ~= \"unset\"

if value is:
    \"one\" => result = \"one\"
    \"one\" => result = \"one\"
    \"one\" => result = \"one\"
    \"one\" => result = \"one\"
    else => result = \"other\"
;
";

    {
        let mut file = std::fs::File::create(&file_path).expect("should create file");
        file.write_all(warning_source.as_bytes())
            .expect("should write to file");
    }

    let options = FrontendBenchmarkOptions {
        entry_path: file_path,
        build_profile: FrontendBenchmarkBuildProfile::Dev,
    };

    let report = run_frontend_benchmark(options).expect("warnings should remain successful");

    assert_eq!(report.warning_count, 3);
    assert_eq!(
        report.warning_codes,
        vec!["MOTH-RULE-0022", "MOTH-RULE-0022", "MOTH-RULE-0022"]
    );
}

#[test]
fn frontend_benchmark_retains_source_package_warning() {
    let _guard = benchmark_test_guard();
    let temp_dir = tempfile::tempdir().expect("should create temp dir");
    let root = temp_dir.path();
    let package = root.join("src/warnpkg");
    let src = root.join("src");
    std::fs::create_dir_all(&package).expect("should create package root");
    std::fs::create_dir_all(&src).expect("should create entry root");
    std::fs::write(
        root.join("config.moth"),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    std::fs::write(src.join("@page.moth"), "value = 1\n").expect("should write project root");
    std::fs::write(
        package.join("+package.moth"),
        "export:\n    run || -> Int:\n        value ~= \"hello\"\n        result ~= \"unset\"\n\n        if value is:\n            \"one\" => result = \"one\"\n            \"one\" => result = \"one\"\n            else => result = \"other\"\n        ;\n        return 1\n    ;\n;\n",
    )
    .expect("should write warning package root");

    let options = FrontendBenchmarkOptions {
        entry_path: root.to_path_buf(),
        build_profile: FrontendBenchmarkBuildProfile::Dev,
    };

    let report = run_frontend_benchmark(options)
        .expect("source-package warning should remain a successful benchmark");
    // The fixture authors exactly one duplicated match arm, so exactly one warning is
    // contractual. `>= 1` plus `any` would also pass if the benchmark leaked warnings from the
    // project root, or emitted the duplicate-arm warning twice.
    assert_eq!(
        report.warning_codes,
        vec!["MOTH-RULE-0022".to_string()],
        "the source-package warning multiset must be exactly the duplicated match arm"
    );
    assert_eq!(
        report.warning_count,
        report.warning_codes.len(),
        "the reported warning count must match the retained codes"
    );
}

#[test]
fn frontend_benchmark_fails_for_missing_file() {
    let _guard = benchmark_test_guard();

    let temp_dir = tempfile::tempdir().expect("should create temp dir");
    let missing_file = temp_dir.path().join("does_not_exist.moth");

    let options = FrontendBenchmarkOptions {
        entry_path: missing_file,
        build_profile: FrontendBenchmarkBuildProfile::Dev,
    };

    let result = run_frontend_benchmark(options);
    let error = result.expect_err("benchmark should fail for missing file");
    assert_eq!(
        error.kind,
        FrontendBenchmarkFailureKind::PathValidation,
        "missing-file benchmark should fail at path validation"
    );
    assert_eq!(
        error.diagnostic_codes,
        vec!["MOTH-INFRA-0001".to_owned()],
        "missing-file benchmark should report an infrastructure diagnostic code"
    );
}

/// A frontend benchmark must acquire its raw session before it validates the entry path,
/// otherwise an outer caller-owned snapshot could receive its observations.
///
/// The evidence is the typed failure boundary, not the outer snapshot's sample counts: the
/// collector is process-global, so compiler work in any other concurrently running test also
/// records into the active session and a "no samples" assertion would depend on what else the
/// suite happens to be running. A missing entry path discriminates the two orderings directly —
/// path validation first would report `PathValidation` with an infrastructure diagnostic.
#[cfg(feature = "timers")]
#[test]
fn frontend_benchmark_rejects_a_busy_raw_session_before_path_validation() {
    let _guard = benchmark_test_guard();
    let temp_dir = tempfile::tempdir().expect("should create temp dir");
    let missing_entry = temp_dir.path().join("does_not_exist.moth");

    let outer =
        crate::timing::start_benchmark_collection(true).expect("outer timing session should start");

    let error = run_frontend_benchmark(FrontendBenchmarkOptions {
        entry_path: missing_entry,
        build_profile: FrontendBenchmarkBuildProfile::Dev,
    })
    .expect_err("busy raw benchmark should fail before the entry path is validated");
    drop(outer);

    assert_eq!(
        error.kind,
        FrontendBenchmarkFailureKind::TimingSession,
        "the busy collector must be reported before path validation: {error}"
    );
    assert!(
        error.diagnostic_codes.is_empty(),
        "timing-session rejection precedes the missing-file diagnostic: {:?}",
        error.diagnostic_codes
    );
}

/// A frontend benchmark must acquire its raw session before any compiler work, otherwise the
/// compile it runs would be recorded into an outer caller-owned snapshot.
///
/// Invalid source discriminates the ordering: compiling first would report `Compilation` with a
/// syntax diagnostic, which is what `frontend_benchmark_fails_for_invalid_syntax` proves happens
/// when the collector is free.
#[cfg(feature = "timers")]
#[test]
fn frontend_benchmark_rejects_a_busy_raw_session_before_compilation() {
    let _guard = benchmark_test_guard();
    let temp_dir = tempfile::tempdir().expect("should create temp dir");
    let entry_path = temp_dir.path().join("bad.moth");
    std::fs::write(&entry_path, "!!! invalid syntax !!!\n").expect("should write invalid entry");

    let outer =
        crate::timing::start_benchmark_collection(true).expect("outer timing session should start");

    let error = run_frontend_benchmark(FrontendBenchmarkOptions {
        entry_path,
        build_profile: FrontendBenchmarkBuildProfile::Dev,
    })
    .expect_err("busy raw benchmark should fail before compiler work");
    drop(outer);

    assert_eq!(
        error.kind,
        FrontendBenchmarkFailureKind::TimingSession,
        "the busy collector must be reported before compilation: {error}"
    );
    assert!(
        error.diagnostic_codes.is_empty(),
        "timing-session rejection precedes any syntax diagnostic: {:?}",
        error.diagnostic_codes
    );
}

#[test]
fn frontend_benchmark_fails_for_invalid_syntax() {
    let _guard = benchmark_test_guard();

    let temp_dir = tempfile::tempdir().expect("should create temp dir");
    let file_path = temp_dir.path().join("bad.moth");

    {
        let mut file = std::fs::File::create(&file_path).expect("should create file");
        file.write_all(b"!!! invalid syntax !!!\n")
            .expect("should write to file");
    }

    let options = FrontendBenchmarkOptions {
        entry_path: file_path,
        build_profile: FrontendBenchmarkBuildProfile::Dev,
    };

    let result = run_frontend_benchmark(options);
    let error = result.expect_err("benchmark should fail for invalid syntax");
    assert_eq!(
        error.kind,
        FrontendBenchmarkFailureKind::Compilation,
        "invalid-syntax benchmark should fail at compilation"
    );
    assert!(
        error
            .diagnostic_codes
            .iter()
            .any(|code| code.starts_with("MOTH-SYNTAX")),
        "invalid-syntax benchmark should report a syntax diagnostic code: {:?}",
        error.diagnostic_codes
    );
}
