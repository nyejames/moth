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
    #[cfg(feature = "timers")]
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
    #[cfg(feature = "timers")]
    let _counter_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
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
    let _guard = BENCHMARK_TEST_MUTEX.lock().expect("test mutex should lock");
    #[cfg(feature = "timers")]
    let _counter_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let temp_dir = tempfile::tempdir().expect("should create temp dir");
    let root = temp_dir.path();
    let package = root.join("packages/warnpkg");
    let src = root.join("src");
    std::fs::create_dir_all(&package).expect("should create package root");
    std::fs::create_dir_all(&src).expect("should create entry root");
    std::fs::write(
        root.join("config.moth"),
        "entry_root #= \"src\"\npackage_folders #= { \"packages\" }\n",
    )
    .expect("should write config");
    std::fs::write(src.join("@page.moth"), "value = 1\n").expect("should write project root");
    std::fs::write(
        package.join("@mod.moth"),
        "value ~= \"hello\"\nresult ~= \"unset\"\n\nif value is:\n    \"one\" => result = \"one\"\n    \"one\" => result = \"one\"\n    else => result = \"other\"\n;\n",
    )
    .expect("should write warning package root");

    let options = FrontendBenchmarkOptions {
        entry_path: root.to_path_buf(),
        build_profile: FrontendBenchmarkBuildProfile::Dev,
    };

    let report = run_frontend_benchmark(options)
        .expect("source-package warning should remain a successful benchmark");
    assert!(
        report.warning_count >= 1,
        "source-package warning should be retained by the frontend benchmark"
    );
    assert!(
        report
            .warning_codes
            .iter()
            .any(|code| code == "MOTH-RULE-0022"),
        "source-package warning code should be retained: {:?}",
        report.warning_codes
    );
}

#[test]
fn frontend_benchmark_fails_for_missing_file() {
    let _guard = BENCHMARK_TEST_MUTEX.lock().expect("test mutex should lock");
    #[cfg(feature = "timers")]
    let _counter_guard = crate::compiler_frontend::instrumentation::lock_counter_test();

    let options = FrontendBenchmarkOptions {
        entry_path: PathBuf::from("/definitely/does/not/exist.moth"),
        build_profile: FrontendBenchmarkBuildProfile::Dev,
    };

    let result = run_frontend_benchmark(options);
    assert!(result.is_err(), "benchmark should fail for missing file");
}

/// A frontend benchmark must acquire its raw session before path validation or
/// compiler setup, otherwise an outer caller-owned snapshot could receive its
/// observations.
#[cfg(feature = "timers")]
#[test]
fn frontend_benchmark_rejects_a_busy_raw_session_before_compilation() {
    let _guard = BENCHMARK_TEST_MUTEX.lock().expect("test mutex should lock");
    let _counter_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let outer =
        crate::timing::start_benchmark_collection(true).expect("outer timing session should start");

    let result = run_frontend_benchmark(FrontendBenchmarkOptions {
        entry_path: PathBuf::from("/definitely/does/not/exist.moth"),
        build_profile: FrontendBenchmarkBuildProfile::Dev,
    });

    let error = result.expect_err("busy raw benchmark should fail before compiler work");
    assert!(
        error
            .to_string()
            .contains("Could not start frontend benchmark timing session"),
        "busy raw-session failures must identify the tooling boundary: {error}"
    );

    let outer_snapshot = outer.finish();
    assert!(
        outer_snapshot.timings.is_empty() && outer_snapshot.counters.is_empty(),
        "the rejected benchmark must not record path or compiler work into the outer session"
    );
}

#[test]
fn frontend_benchmark_fails_for_invalid_syntax() {
    let _guard = BENCHMARK_TEST_MUTEX.lock().expect("test mutex should lock");
    #[cfg(feature = "timers")]
    let _counter_guard = crate::compiler_frontend::instrumentation::lock_counter_test();

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
