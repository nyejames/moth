//! WHAT: verifies the Moth CLI's subprocess exit-status and benchmark-status contracts.
//! WHY: benchmark and tooling callers must observe deterministic process outcomes without
//!      inheriting benchmark-only environment state from the test process.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use tempfile::tempdir;

const WARNING_SOURCE: &str = "value ~= \"hello\"\n\
result ~= \"unset\"\n\
if value is:\n\
    captured => result = captured\n\
    \"one\" => result = \"one\"\n\
    \"two\" => result = \"two\"\n\
    else => result = \"other\"\n\
;\n";

const INVALID_SOURCE: &str = "value = (\n";

fn run_moth(arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_moth"))
        .args(arguments)
        .env_remove("MOTH_BENCH_STATUS")
        .output()
        .expect("moth subprocess should start")
}

fn run_moth_with_bench_status(arguments: &[&str], value: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_moth"))
        .args(arguments)
        .env("MOTH_BENCH_STATUS", value)
        .output()
        .expect("moth subprocess should start")
}

fn run_moth_from_dir(current_dir: &Path, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_moth"))
        .current_dir(current_dir)
        .args(arguments)
        .env_remove("MOTH_BENCH_STATUS")
        .output()
        .expect("moth subprocess should start")
}

fn write_source(root: &std::path::Path, name: &str, source: &str) -> std::path::PathBuf {
    let path = root.join(name);
    fs::write(&path, source).expect("test source should be written");
    path
}

#[test]
fn valid_single_file_check_exits_successfully() {
    let root = tempdir().expect("temporary directory should be created");
    let source = write_source(root.path(), "main.moth", "value = 1\n");

    let output = run_moth(&[
        "check",
        source.to_str().expect("source path should be UTF-8"),
    ]);

    assert!(output.status.success());
}

#[test]
fn invalid_syntax_check_exits_with_failure() {
    let root = tempdir().expect("temporary directory should be created");
    let source = write_source(root.path(), "main.moth", "value = (\n");

    let output = run_moth(&[
        "check",
        source.to_str().expect("source path should be UTF-8"),
    ]);

    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn warning_only_check_exits_successfully() {
    let root = tempdir().expect("temporary directory should be created");
    let source = write_source(root.path(), "main.moth", WARNING_SOURCE);

    let output = run_moth(&[
        "check",
        source.to_str().expect("source path should be UTF-8"),
    ]);

    assert!(output.status.success());
}

#[test]
fn valid_project_build_exits_successfully() {
    let root = tempdir().expect("temporary directory should be created");
    write_source(
        root.path(),
        "@page.moth",
        "#[:<h1>Hello</h1>]\nentry = \".\"\n",
    );

    let output = run_moth(&[
        "build",
        root.path().to_str().expect("project path should be UTF-8"),
    ]);

    assert!(output.status.success());
}

#[test]
fn single_file_build_writes_outputs_using_containing_directory_context() {
    let root = tempdir().expect("temporary directory should be created");
    let project_dir = root.path().join("project");
    fs::create_dir(&project_dir).expect("single-file project directory should be created");
    write_source(&project_dir, "main.moth", "#[:<h1>Hello</h1>]\n");

    let output = run_moth_from_dir(root.path(), &["build", "project/main.moth"]);

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        root.path().join("main.html").is_file(),
        "single-file build should reach output writing"
    );
}

#[test]
fn bare_single_file_build_writes_outputs_in_current_directory() {
    let root = tempdir().expect("temporary directory should be created");
    write_source(root.path(), "main.moth", "#[:<h1>Hello</h1>]\n");

    let output = run_moth_from_dir(root.path(), &["build", "main.moth"]);

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        root.path().join("main.html").is_file(),
        "bare single-file build should write output in its current directory"
    );
}

#[test]
fn invalid_syntax_project_build_exits_with_failure() {
    let root = tempdir().expect("temporary directory should be created");
    write_source(root.path(), "@page.moth", INVALID_SOURCE);

    let output = run_moth(&[
        "build",
        root.path().to_str().expect("project path should be UTF-8"),
    ]);

    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn output_root_write_failure_exits_with_failure_without_benchmark_status() {
    let root = tempdir().expect("temporary directory should be created");
    write_source(
        root.path(),
        "@page.moth",
        "#[:<h1>Hello</h1>]\nentry = \".\"\n",
    );
    fs::write(root.path().join("dev"), b"occupied output root")
        .expect("output-root collision file should be written");

    let output = run_moth_with_bench_status(
        &[
            "build",
            root.path().to_str().expect("project path should be UTF-8"),
        ],
        "1",
    );

    assert_eq!(output.status.code(), Some(1));
    assert_no_status_record(&output);
}

#[test]
fn unknown_command_exits_with_failure() {
    let output = run_moth(&["unknown-command"]);

    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn unknown_check_and_build_flags_exit_with_failure() {
    for arguments in [["check", "--unknown"], ["build", "--unknown"]] {
        let output = run_moth(&arguments);
        assert_eq!(output.status.code(), Some(1), "arguments: {arguments:?}");
    }
}

#[test]
fn version_exits_successfully() {
    let output = run_moth(&["--version"]);

    assert!(output.status.success());
}

#[test]
fn benchmark_status_record_is_emitted_once_for_check_and_build() {
    let root = tempdir().expect("temporary directory should be created");
    let source = write_source(root.path(), "main.moth", "value = 1\n");
    let source_path = source.to_str().expect("source path should be UTF-8");

    let check_output = run_moth_with_bench_status(&["check", source_path], "1");
    assert!(check_output.status.success());
    assert_status_record(&check_output, "MOTH_BENCH status errors=0 warnings=0");

    let build_root = tempdir().expect("temporary directory should be created");
    write_source(
        build_root.path(),
        "@page.moth",
        "#[:<h1>Hello</h1>]\nentry = \".\"\n",
    );
    let build_path = build_root
        .path()
        .to_str()
        .expect("project path should be UTF-8");
    let build_output = run_moth_with_bench_status(&["build", build_path], "1");
    assert!(build_output.status.success());
    assert_status_record(&build_output, "MOTH_BENCH status errors=0 warnings=0");
}

#[test]
fn benchmark_status_record_covers_warning_only_check_and_build() {
    let check_root = tempdir().expect("temporary directory should be created");
    let check_source = write_source(check_root.path(), "main.moth", WARNING_SOURCE);
    let check_path = check_source
        .to_str()
        .expect("check source path should be UTF-8");
    let check_output = run_moth_with_bench_status(&["check", check_path], "1");
    assert!(check_output.status.success());
    assert_status_record(&check_output, "MOTH_BENCH status errors=0 warnings=3");

    let build_root = tempdir().expect("temporary directory should be created");
    write_source(build_root.path(), "@page.moth", WARNING_SOURCE);
    let build_path = build_root
        .path()
        .to_str()
        .expect("build project path should be UTF-8");
    let build_output = run_moth_with_bench_status(&["build", build_path], "1");
    assert!(build_output.status.success());
    assert_status_record(&build_output, "MOTH_BENCH status errors=0 warnings=3");
}

#[test]
fn benchmark_status_record_covers_diagnosed_check_and_build() {
    let check_root = tempdir().expect("temporary directory should be created");
    let check_source = write_source(check_root.path(), "main.moth", INVALID_SOURCE);
    let check_path = check_source
        .to_str()
        .expect("check source path should be UTF-8");
    let check_output = run_moth_with_bench_status(&["check", check_path], "1");
    assert_eq!(check_output.status.code(), Some(1));
    assert_status_record(&check_output, "MOTH_BENCH status errors=1 warnings=0");

    let build_root = tempdir().expect("temporary directory should be created");
    write_source(build_root.path(), "@page.moth", INVALID_SOURCE);
    let build_path = build_root
        .path()
        .to_str()
        .expect("build project path should be UTF-8");
    let build_output = run_moth_with_bench_status(&["build", build_path], "1");
    assert_eq!(build_output.status.code(), Some(1));
    assert_status_record(&build_output, "MOTH_BENCH status errors=1 warnings=0");
}

#[test]
fn benchmark_status_record_is_absent_without_opt_in() {
    let root = tempdir().expect("temporary directory should be created");
    let source = write_source(root.path(), "main.moth", "value = 1\n");
    let source_path = source.to_str().expect("source path should be UTF-8");
    let output = run_moth(&["check", source_path]);

    assert!(output.status.success());
    assert_no_status_record(&output);
}

#[test]
fn benchmark_status_record_requires_exact_opt_in_value() {
    let root = tempdir().expect("temporary directory should be created");
    let source = write_source(root.path(), "main.moth", "value = 1\n");
    let source_path = source.to_str().expect("source path should be UTF-8");

    for value in ["0", "true", "01"] {
        let output = run_moth_with_bench_status(&["check", source_path], value);
        assert!(output.status.success(), "opt-in value: {value}");
        assert_no_status_record(&output);
    }
}

#[test]
fn infrastructure_failure_emits_no_benchmark_status_record() {
    let root = tempdir().expect("temporary directory should be created");
    let missing_source = root.path().join("missing.moth");
    let source_path = missing_source
        .to_str()
        .expect("missing source path should be UTF-8");
    let output = run_moth_with_bench_status(&["check", source_path], "1");

    assert_eq!(output.status.code(), Some(1));
    assert_no_status_record(&output);
}

fn assert_status_record(output: &Output, expected: &str) {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines = stdout.lines().collect::<Vec<_>>();
    let records = stdout
        .lines()
        .filter(|line| line.starts_with("MOTH_BENCH status"))
        .collect::<Vec<_>>();
    assert_eq!(
        records,
        vec![expected],
        "unexpected benchmark status output: {stdout}"
    );
    assert_eq!(
        lines.last().copied(),
        Some(expected),
        "benchmark status record must be the final stdout line: {stdout}"
    );
}

fn assert_no_status_record(output: &Output) {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let records = stdout
        .lines()
        .filter(|line| line.starts_with("MOTH_BENCH status"))
        .collect::<Vec<_>>();
    assert!(
        records.is_empty(),
        "unexpected benchmark status output: {stdout}"
    );
}
