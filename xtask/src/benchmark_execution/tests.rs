use std::cell::Cell;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use tempfile::TempDir;

use super::{
    BenchmarkExecutionContext, BenchmarkFailureKind, average_case_observations, execute_case,
    preflight_cases, run_preflighted_suite, validate_total_duration,
};
use crate::bench_types::{BenchmarkCaseObservations, BenchmarkMetric};
use crate::benchmark_manifest::{
    BenchmarkCase, BenchmarkExpectation, BenchmarkManifest, BenchmarkRunner, BenchmarkWorkload,
    CliBenchmarkCommand, FrontendBenchmarkProfile,
};
use crate::benchmark_status::BenchmarkDiagnosticStatus;

static FRONTEND_EXECUTION_TEST_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn cli_execution_uses_declared_build_command_entry_and_ordered_args() {
    let fixture = CliFixture::new();
    let compiler = fixture.mock_path("declared_build");
    create_expected_invocation_executable(
        &compiler,
        &["build", "benchmarks/fixture.moth", "--release", "--terse"],
    );
    let manifest = fixture.manifest(
        vec![fixture.workload("benchmarks/fixture.moth")],
        vec![cli_case(
            "declared_build",
            0,
            CliBenchmarkCommand::Build,
            &["--release", "--terse"],
        )],
    );
    let context = BenchmarkExecutionContext::new(&manifest, &compiler);

    let execution =
        execute_case(&context, &manifest.cases[0]).expect("declared build case should pass");

    assert_eq!(execution.case_id, "declared_build");
    assert_eq!(execution.workload_id, "fixture");
    assert!(matches!(
        execution.runner,
        BenchmarkRunner::Cli {
            command: CliBenchmarkCommand::Build,
            ..
        }
    ));
    assert!(execution.total_duration_ms.is_finite());
    assert!(execution.total_duration_ms > 0.0);
    assert_eq!(execution.benchmark_status.error_count, 0);
    assert_eq!(execution.benchmark_status.warning_count, 0);
}

#[test]
fn cli_execution_uses_declared_check_command() {
    let fixture = CliFixture::new();
    let compiler = fixture.mock_path("declared_check");
    create_expected_invocation_executable(
        &compiler,
        &["check", "benchmarks/fixture.moth", "--terse"],
    );
    let manifest = fixture.manifest(
        vec![fixture.workload("benchmarks/fixture.moth")],
        vec![cli_case(
            "declared_check",
            0,
            CliBenchmarkCommand::Check,
            &["--terse"],
        )],
    );
    let context = BenchmarkExecutionContext::new(&manifest, &compiler);

    execute_case(&context, &manifest.cases[0]).expect("declared check case should pass");
}

#[test]
fn clean_cli_execution_rejects_warning_status() {
    let fixture = CliFixture::new();
    let compiler = fixture.mock_path("warning");
    create_output_executable(&compiler, "MOTH_BENCH status errors=0 warnings=2", "", 0);
    let manifest = fixture.single_cli_manifest();
    let context = BenchmarkExecutionContext::new(&manifest, &compiler);

    let failure =
        execute_case(&context, &manifest.cases[0]).expect_err("warnings must fail clean preflight");

    assert!(matches!(
        failure.kind,
        BenchmarkFailureKind::CleanExpectationWarnings {
            warning_count: 2,
            ..
        }
    ));
    assert_eq!(
        failure
            .benchmark_status
            .expect("status should be retained")
            .warning_count,
        2
    );
}

#[test]
fn successful_cli_process_rejects_reported_errors() {
    let fixture = CliFixture::new();
    let compiler = fixture.mock_path("errors");
    create_output_executable(&compiler, "MOTH_BENCH status errors=3 warnings=0", "", 0);
    let manifest = fixture.single_cli_manifest();
    let context = BenchmarkExecutionContext::new(&manifest, &compiler);

    let failure =
        execute_case(&context, &manifest.cases[0]).expect_err("reported errors must fail closed");

    assert!(matches!(
        failure.kind,
        BenchmarkFailureKind::CleanExpectationErrors { error_count: 3 }
    ));
}

#[test]
fn successful_cli_process_rejects_missing_malformed_and_duplicate_status() {
    let fixture = CliFixture::new();
    let manifest = fixture.single_cli_manifest();

    for (name, stdout) in [
        ("missing", "ordinary output"),
        (
            "malformed",
            "MOTH_BENCH status errors=0 warnings=0 trailing",
        ),
        (
            "duplicate",
            "MOTH_BENCH status errors=0 warnings=0\nMOTH_BENCH status errors=0 warnings=0",
        ),
    ] {
        let compiler = fixture.mock_path(name);
        create_output_executable(&compiler, stdout, "", 0);
        let context = BenchmarkExecutionContext::new(&manifest, &compiler);

        let failure = execute_case(&context, &manifest.cases[0])
            .expect_err("invalid machine status must fail closed");

        assert!(
            matches!(
                failure.kind,
                BenchmarkFailureKind::InvalidMachineStatus { .. }
            ),
            "{name} should be classified as invalid machine status"
        );
    }
}

#[test]
fn successful_cli_process_rejects_invalid_live_observations_with_bounded_evidence() {
    let fixture = CliFixture::new();
    let manifest = fixture.single_cli_manifest();

    for (name, observation_output) in [
        ("missing_total", "AST created in: 1ms"),
        (
            "malformed_timing",
            "MOTH_BENCH timing command.check.total=1",
        ),
        ("legacy_only", "AST created in: 1ms\nHIR generated in: 2ms"),
    ] {
        let compiler = fixture.mock_path(name);
        let stdout = format!("MOTH_BENCH status errors=0 warnings=0\n{observation_output}");
        create_output_executable(&compiler, &stdout, "", 0);
        let context = BenchmarkExecutionContext::new(&manifest, &compiler);

        let failure = execute_case(&context, &manifest.cases[0])
            .expect_err("invalid live observations must fail closed");

        assert!(
            matches!(
                failure.kind,
                BenchmarkFailureKind::ObservationInfrastructureFailure { .. }
            ),
            "{name} should use the observation failure lane"
        );
        assert!(
            failure
                .stdout_evidence
                .as_deref()
                .is_some_and(|evidence| evidence.contains(observation_output)),
            "{name} should retain bounded stdout evidence"
        );
    }
}

#[test]
fn nonzero_process_status_fails_even_when_machine_status_is_clean() {
    let fixture = CliFixture::new();
    let compiler = fixture.mock_path("nonzero");
    create_output_executable(
        &compiler,
        "MOTH_BENCH status errors=0 warnings=0",
        "compiler infrastructure failed",
        7,
    );
    let manifest = fixture.single_cli_manifest();
    let context = BenchmarkExecutionContext::new(&manifest, &compiler);

    let failure =
        execute_case(&context, &manifest.cases[0]).expect_err("nonzero process must fail");

    assert!(matches!(
        failure.kind,
        BenchmarkFailureKind::NonZeroProcessStatus
    ));
    assert_eq!(failure.exit_code, Some(7));
    assert_eq!(
        failure
            .benchmark_status
            .expect("clean status evidence should be retained")
            .error_count,
        0
    );
    assert_eq!(
        failure.stderr_evidence.as_deref(),
        Some("compiler infrastructure failed")
    );
}

#[test]
fn missing_compiler_binary_is_a_process_spawn_failure() {
    let fixture = CliFixture::new();
    let compiler = fixture.root().join("missing-compiler");
    let manifest = fixture.single_cli_manifest();
    let context = BenchmarkExecutionContext::new(&manifest, &compiler);

    let failure =
        execute_case(&context, &manifest.cases[0]).expect_err("spawn failure must be retained");

    assert!(matches!(
        failure.kind,
        BenchmarkFailureKind::ProcessSpawnFailure { .. }
    ));
}

#[test]
fn cli_and_frontend_total_durations_must_be_positive_and_finite() {
    let fixture = CliFixture::new();
    let compiler = fixture.root().join("unused-compiler");
    let manifest = fixture.manifest(
        vec![fixture.workload("benchmarks/fixture.moth")],
        vec![
            cli_case("fixture_check", 0, CliBenchmarkCommand::Check, &[]),
            frontend_case("fixture_frontend", 0),
        ],
    );
    let context = BenchmarkExecutionContext::new(&manifest, &compiler);
    let status = BenchmarkDiagnosticStatus {
        error_count: 0,
        warning_count: 0,
    };

    for case in &manifest.cases {
        for duration_ms in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            let failure = validate_total_duration(&context, case, duration_ms, None, status)
                .expect_err("invalid duration must fail");

            assert!(matches!(
                failure.kind,
                BenchmarkFailureKind::InvalidTotalDuration { .. }
            ));
        }
    }
}

#[test]
fn inconsistent_measured_metric_sets_use_typed_observation_failure() {
    let fixture = CliFixture::new();
    let manifest = fixture.single_cli_manifest();
    let compiler = fixture.root().join("unused-compiler");
    let context = BenchmarkExecutionContext::new(&manifest, &compiler);
    let observations = vec![
        BenchmarkCaseObservations {
            stage_timings: vec![BenchmarkMetric {
                name: "command.check.total".to_owned(),
                value: 1.0,
            }],
            counters: Vec::new(),
        },
        BenchmarkCaseObservations {
            stage_timings: vec![BenchmarkMetric {
                name: "frontend.ast".to_owned(),
                value: 1.0,
            }],
            counters: Vec::new(),
        },
    ];

    let failure = average_case_observations(&context, &manifest.cases[0], &observations)
        .expect_err("timing metric drift must fail through the typed lane");

    assert!(matches!(
        failure.kind,
        BenchmarkFailureKind::ObservationInfrastructureFailure { .. }
    ));
    assert!(failure.to_string().contains("missing: command.check.total"));
    assert!(failure.to_string().contains("additional: frontend.ast"));
}

#[test]
fn preflight_executes_every_case_once_and_aggregates_failures_in_manifest_order() {
    let fixture = CliFixture::new();
    let count_path = fixture.root().join("invocations.txt");
    let compiler = fixture.mock_path("aggregate");
    create_counting_failure_executable(&compiler, &count_path);
    let manifest = fixture.manifest(
        vec![
            fixture.workload("benchmarks/first.moth"),
            fixture.workload("benchmarks/second.moth"),
        ],
        vec![
            cli_case("first_case", 0, CliBenchmarkCommand::Check, &[]),
            cli_case("second_case", 1, CliBenchmarkCommand::Build, &[]),
        ],
    );
    let context = BenchmarkExecutionContext::new(&manifest, &compiler);

    let failures =
        preflight_cases(&context, &manifest.cases).expect_err("both cases should fail preflight");

    assert_eq!(failures.len(), 2);
    assert_eq!(failures[0].case_id, "first_case");
    assert_eq!(failures[1].case_id, "second_case");
    assert_eq!(
        fs::read_to_string(count_path).expect("invocation log should exist"),
        "check\nbuild\n"
    );
}

#[test]
fn failed_preflight_prevents_measurement_callback_invocation() {
    let fixture = CliFixture::new();
    let compiler = fixture.mock_path("failed_preflight");
    create_output_executable(&compiler, "", "preflight failed", 1);
    let manifest = fixture.single_cli_manifest();
    let context = BenchmarkExecutionContext::new(&manifest, &compiler);
    let measurement_called = Cell::new(false);

    let result = run_preflighted_suite(
        &context,
        &manifest.cases,
        || {
            measurement_called.set(true);
            Ok(())
        },
        |_| Ok(()),
    );

    assert!(result.is_err());
    assert!(!measurement_called.get());
}

#[test]
fn failed_measurement_prevents_history_callback_invocation() {
    let fixture = CliFixture::new();
    let compiler = fixture.mock_path("failed_measurement");
    create_output_executable(
        &compiler,
        "MOTH_BENCH status errors=0 warnings=0\nMOTH_BENCH timing command.check.total=1ms",
        "",
        0,
    );
    let manifest = fixture.single_cli_manifest();
    let context = BenchmarkExecutionContext::new(&manifest, &compiler);
    let history_called = Cell::new(false);

    let result = run_preflighted_suite(
        &context,
        &manifest.cases,
        || Err::<(), _>("measured iteration failed".to_owned()),
        |_| {
            history_called.set(true);
            Ok(())
        },
    );

    assert_eq!(result, Err("measured iteration failed".to_owned()));
    assert!(!history_called.get());
}

#[test]
fn failure_evidence_keeps_stdout_and_stderr_separate_and_bounded() {
    let fixture = CliFixture::new();
    let compiler = fixture.mock_path("bounded");
    let stdout = "o".repeat(2_500);
    let stderr = "e".repeat(2_500);
    create_output_executable(&compiler, &stdout, &stderr, 1);
    let manifest = fixture.single_cli_manifest();
    let context = BenchmarkExecutionContext::new(&manifest, &compiler);

    let failure =
        execute_case(&context, &manifest.cases[0]).expect_err("nonzero process should fail");
    let stdout_evidence = failure
        .stdout_evidence
        .as_deref()
        .expect("stdout evidence should exist");
    let stderr_evidence = failure
        .stderr_evidence
        .as_deref()
        .expect("stderr evidence should exist");
    let rendered = failure.to_string();

    assert!(stdout_evidence.len() < stdout.len());
    assert!(stderr_evidence.len() < stderr.len());
    assert!(stdout_evidence.ends_with("[output truncated]"));
    assert!(stderr_evidence.ends_with("[output truncated]"));
    assert!(rendered.contains("\n  stdout:\n"));
    assert!(rendered.contains("\n  stderr:\n"));
}

#[test]
fn frontend_execution_returns_the_common_success_shape() {
    let _guard = FRONTEND_EXECUTION_TEST_LOCK
        .lock()
        .expect("frontend execution test lock should not be poisoned");
    let fixture = CliFixture::new();
    fs::write(fixture.root().join("clean.moth"), "value = 42\n")
        .expect("clean fixture should be written");
    let manifest = fixture.manifest(
        vec![fixture.workload("clean.moth")],
        vec![frontend_case("clean_frontend", 0)],
    );
    let unused_compiler = fixture.root().join("unused-compiler");
    let context = BenchmarkExecutionContext::new(&manifest, &unused_compiler);

    let execution =
        execute_case(&context, &manifest.cases[0]).expect("clean frontend case should pass");

    assert_eq!(execution.case_id, "clean_frontend");
    assert_eq!(execution.workload_id, "clean");
    assert!(matches!(
        execution.runner,
        BenchmarkRunner::Frontend {
            profile: FrontendBenchmarkProfile::Dev
        }
    ));
    assert!(execution.total_duration_ms > 0.0);
    assert_eq!(execution.benchmark_status.error_count, 0);
    assert_eq!(execution.benchmark_status.warning_count, 0);
    assert!(
        execution
            .observations
            .stage_timings
            .iter()
            .all(|metric| metric.value.is_finite())
    );
}

#[test]
fn frontend_execution_uses_public_api_and_rejects_clean_warnings() {
    let _guard = FRONTEND_EXECUTION_TEST_LOCK
        .lock()
        .expect("frontend execution test lock should not be poisoned");
    let fixture = CliFixture::new();
    let warning_path = fixture.root().join("warning.moth");
    fs::write(
        &warning_path,
        "\
value ~= \"hello\"
result ~= \"unset\"

if value is:
    captured => result = captured
    \"one\" => result = \"one\"
    \"two\" => result = \"two\"
    else => result = \"other\"
;
",
    )
    .expect("warning fixture should be written");
    let manifest = fixture.manifest(
        vec![fixture.workload("warning.moth")],
        vec![frontend_case("warning_frontend", 0)],
    );
    let unused_compiler = fixture.root().join("unused-compiler");
    let context = BenchmarkExecutionContext::new(&manifest, &unused_compiler);

    let failure = execute_case(&context, &manifest.cases[0])
        .expect_err("frontend warnings must fail clean preflight");

    assert!(matches!(
        failure.kind,
        BenchmarkFailureKind::CleanExpectationWarnings {
            warning_count: 3,
            ..
        }
    ));
    assert_eq!(
        failure
            .benchmark_status
            .expect("frontend warning status should be retained")
            .warning_count,
        3
    );
}

#[test]
fn frontend_compilation_failure_is_typed_and_bounded() {
    let _guard = FRONTEND_EXECUTION_TEST_LOCK
        .lock()
        .expect("frontend execution test lock should not be poisoned");
    let fixture = CliFixture::new();
    fs::write(fixture.root().join("invalid.moth"), "value =\n")
        .expect("invalid fixture should be written");
    let manifest = fixture.manifest(
        vec![fixture.workload("invalid.moth")],
        vec![frontend_case("invalid_frontend", 0)],
    );
    let unused_compiler = fixture.root().join("unused-compiler");
    let context = BenchmarkExecutionContext::new(&manifest, &unused_compiler);

    let failure = execute_case(&context, &manifest.cases[0])
        .expect_err("frontend compiler failure must fail preflight");

    assert!(matches!(
        failure.kind,
        BenchmarkFailureKind::FrontendCompilationFailure
    ));
    assert!(
        failure
            .stderr_evidence
            .as_deref()
            .is_some_and(|evidence| evidence.contains("MOTH-"))
    );
}

struct CliFixture {
    temp_dir: TempDir,
}

impl CliFixture {
    fn new() -> Self {
        Self {
            temp_dir: tempfile::tempdir().expect("temporary directory should exist"),
        }
    }

    fn root(&self) -> &Path {
        self.temp_dir.path()
    }

    fn mock_path(&self, name: &str) -> PathBuf {
        let path = self.root().join(name);

        #[cfg(windows)]
        let path = path.with_extension("bat");

        path
    }

    fn workload(&self, entry: &str) -> BenchmarkWorkload {
        BenchmarkWorkload {
            id: entry
                .rsplit('/')
                .next()
                .unwrap_or(entry)
                .trim_end_matches(".moth")
                .to_owned(),
            entry: PathBuf::from(entry),
            fingerprint_roots: vec![PathBuf::from(entry)],
            fingerprint_excludes: Vec::new(),
        }
    }

    fn manifest(
        &self,
        workloads: Vec<BenchmarkWorkload>,
        cases: Vec<BenchmarkCase>,
    ) -> BenchmarkManifest {
        BenchmarkManifest {
            workloads,
            cases,
            manifest_path: self.root().join("manifest.toml"),
            repository_root: self.root().to_path_buf(),
        }
    }

    fn single_cli_manifest(&self) -> BenchmarkManifest {
        self.manifest(
            vec![self.workload("benchmarks/fixture.moth")],
            vec![cli_case(
                "fixture_check",
                0,
                CliBenchmarkCommand::Check,
                &[],
            )],
        )
    }
}

fn cli_case(
    id: &str,
    workload_index: usize,
    command: CliBenchmarkCommand,
    args: &[&str],
) -> BenchmarkCase {
    BenchmarkCase {
        id: id.to_owned(),
        workload_index,
        group_name: "core".to_owned(),
        quick: false,
        expectation: BenchmarkExpectation::Clean,
        runner: BenchmarkRunner::Cli {
            command,
            args: args.iter().map(|arg| (*arg).to_owned()).collect(),
        },
    }
}

fn frontend_case(id: &str, workload_index: usize) -> BenchmarkCase {
    BenchmarkCase {
        id: id.to_owned(),
        workload_index,
        group_name: "core".to_owned(),
        quick: false,
        expectation: BenchmarkExpectation::Clean,
        runner: BenchmarkRunner::Frontend {
            profile: FrontendBenchmarkProfile::Dev,
        },
    }
}

#[cfg(unix)]
fn create_expected_invocation_executable(path: &Path, expected_args: &[&str]) {
    use std::os::unix::fs::PermissionsExt;

    let checks = expected_args
        .iter()
        .enumerate()
        .map(|(index, expected)| {
            format!(
                "if [ \"${}\" != \"{}\" ]; then exit 9; fi",
                index + 1,
                expected
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let required_timing = match expected_args.first().copied() {
        Some("check") => "command.check.total",
        Some("build") => "command.build.total",
        command => panic!("unexpected mock benchmark command: {command:?}"),
    };
    let script = format!(
        "#!/bin/sh\n{checks}\nif [ \"$#\" -ne \"{}\" ]; then exit 9; fi\nprintf '%s\\n' 'MOTH_BENCH timing {required_timing}=1ms' 'MOTH_BENCH status errors=0 warnings=0'\n",
        expected_args.len(),
    );

    fs::write(path, script).expect("mock executable should be written");
    let mut permissions = fs::metadata(path)
        .expect("mock metadata should exist")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).expect("mock executable should be executable");
}

#[cfg(windows)]
fn create_expected_invocation_executable(path: &Path, expected_args: &[&str]) {
    let checks = expected_args
        .iter()
        .enumerate()
        .map(|(index, expected)| format!("if not \"%~{}\"==\"{}\" exit /b 9", index + 1, expected))
        .collect::<Vec<_>>()
        .join("\r\n");
    let required_timing = match expected_args.first().copied() {
        Some("check") => "command.check.total",
        Some("build") => "command.build.total",
        command => panic!("unexpected mock benchmark command: {command:?}"),
    };
    let script = format!(
        "@echo off\r\n{checks}\r\nif not \"%~{}\"==\"\" exit /b 9\r\necho MOTH_BENCH timing {required_timing}=1ms\r\necho MOTH_BENCH status errors=0 warnings=0\r\n",
        expected_args.len() + 1,
    );

    fs::write(path, script).expect("mock executable should be written");
}

#[cfg(unix)]
fn create_output_executable(path: &Path, stdout: &str, stderr: &str, exit_code: i32) {
    use std::os::unix::fs::PermissionsExt;

    let script = format!(
        "#!/bin/sh\nprintf '%s' '{}'\nprintf '%s' '{}' >&2\nexit {exit_code}\n",
        shell_single_quote(stdout),
        shell_single_quote(stderr)
    );
    fs::write(path, script).expect("mock executable should be written");
    let mut permissions = fs::metadata(path)
        .expect("mock metadata should exist")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).expect("mock executable should be executable");
}

#[cfg(windows)]
fn create_output_executable(path: &Path, stdout: &str, stderr: &str, exit_code: i32) {
    let script = format!(
        "@echo off\r\n<nul set /p=\"{stdout}\"\r\n<nul set /p=\"{stderr}\" 1>&2\r\nexit /b {exit_code}\r\n"
    );
    fs::write(path, script).expect("mock executable should be written");
}

#[cfg(unix)]
fn create_counting_failure_executable(path: &Path, count_path: &Path) {
    use std::os::unix::fs::PermissionsExt;

    let script = format!(
        "#!/bin/sh\nprintf '%s\\n' \"$1\" >> '{}'\nexit 1\n",
        shell_single_quote(&count_path.display().to_string())
    );
    fs::write(path, script).expect("mock executable should be written");
    let mut permissions = fs::metadata(path)
        .expect("mock metadata should exist")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).expect("mock executable should be executable");
}

#[cfg(windows)]
fn create_counting_failure_executable(path: &Path, count_path: &Path) {
    let script = format!(
        "@echo off\r\necho %~1>>\"{}\"\r\nexit /b 1\r\n",
        count_path.display()
    );
    fs::write(path, script).expect("mock executable should be written");
}

#[cfg(unix)]
fn shell_single_quote(value: &str) -> String {
    value.replace('\'', "'\"'\"'")
}
