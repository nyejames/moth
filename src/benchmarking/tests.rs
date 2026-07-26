use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

use crate::benchmarking::frontend::{
    FrontendBenchmarkBuildProfile, FrontendBenchmarkOptions, run_frontend_benchmark,
};

// The in-memory benchmark collector uses a global static. Parallel tests that
// each start/stop a collection scope would race. Serialize benchmarking tests
// to keep the harness simple without adding a test-dependency crate.
static BENCHMARK_TEST_MUTEX: Mutex<()> = Mutex::new(());

#[test]
fn frontend_benchmark_runs_for_simple_file() {
    let _guard = BENCHMARK_TEST_MUTEX.lock().expect("test mutex should lock");
    #[cfg(all(feature = "timers", feature = "benchmark_counters"))]
    let _counter_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
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

    assert!(report.total_ms > 0.0, "total time should be positive");
    assert_eq!(report.warning_count, 0);
    assert!(report.warning_codes.is_empty());

    // Stage timings are collected when `timers` is enabled.
    #[cfg(feature = "timers")]
    assert!(
        !report.stages.is_empty(),
        "stage timings should be collected when timers is enabled"
    );

    // Counters additionally require `benchmark_counters`.
    #[cfg(all(feature = "timers", feature = "benchmark_counters"))]
    assert!(
        !report.counters.is_empty(),
        "counters should be collected when timers and benchmark_counters are enabled"
    );
}

#[test]
fn frontend_benchmark_retains_warning_count_and_codes() {
    let _guard = BENCHMARK_TEST_MUTEX.lock().expect("test mutex should lock");
    let temp_dir = tempfile::tempdir().expect("should create temp dir");
    let file_path = temp_dir.path().join("warning.moth");
    let warning_source = "\
value ~= \"hello\"
result ~= \"unset\"

if value is:
    captured => result = captured
    \"one\" => result = \"one\"
    \"two\" => result = \"two\"
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
fn frontend_benchmark_fails_for_missing_file() {
    let _guard = BENCHMARK_TEST_MUTEX.lock().expect("test mutex should lock");

    let options = FrontendBenchmarkOptions {
        entry_path: PathBuf::from("/definitely/does/not/exist.moth"),
        build_profile: FrontendBenchmarkBuildProfile::Dev,
    };

    let result = run_frontend_benchmark(options);
    assert!(result.is_err(), "benchmark should fail for missing file");
}

#[test]
fn frontend_benchmark_fails_for_invalid_syntax() {
    let _guard = BENCHMARK_TEST_MUTEX.lock().expect("test mutex should lock");

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
    assert!(result.is_err(), "benchmark should fail for invalid syntax");
}
