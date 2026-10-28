//! WHAT: verifies the Moth CLI's subprocess exit-status and benchmark-status contracts.
//! WHY: benchmark and tooling callers must observe deterministic process outcomes without
//!      inheriting benchmark-only environment state from the test process.

#[cfg(target_os = "linux")]
use std::ffi::OsStr;
#[cfg(unix)]
use std::ffi::OsString;
use std::fs;
#[cfg(target_os = "linux")]
use std::os::unix::ffi::OsStrExt;
#[cfg(unix)]
use std::os::unix::ffi::OsStringExt;
use std::path::Path;
use std::process::{Command, Output};

use tempfile::tempdir;

const WARNING_SOURCE: &str = "value ~= \"hello\"\n\
result ~= \"unset\"\n\
if value is:\n\
    \"one\" => result = \"one\"\n\
    \"one\" => result = \"one\"\n\
    \"one\" => result = \"one\"\n\
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

fn write_directory_html_project(root: &Path, page_source: &str) {
    let src = root.join("src");
    fs::create_dir_all(&src).expect("src should be created");
    fs::write(
        root.join("config.moth"),
        "project #= (\n    name = \"docs\",\n    entry_root = \"src\",\n)\nhtml #= ()\n",
    )
    .expect("config should be written");
    fs::write(src.join("@page.moth"), page_source).expect("page source should be written");
}

fn write_directory_html_config_input_project(root: &Path) {
    let src = root.join("src");
    fs::create_dir_all(&src).expect("src should be created");
    fs::write(
        root.join("config.moth"),
        "project #= (\n    name = \"docs\",\n    entry_root = \"src\",\n)\nhtml #= (\n    dev_output = \"preview\",\n    release_output = \"release\",\n)\n",
    )
    .expect("config should be written");
    fs::write(
        src.join("@page.moth"),
        "enabled #Config of Bool\nresult ~= 0\nif enabled:\n    result = 1\n;\n",
    )
    .expect("page source should be written");
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
    write_directory_html_project(root.path(), "#[:<h1>Hello</h1>]\nentry = \".\"\n");

    let output = run_moth(&[
        "build",
        root.path().to_str().expect("project path should be UTF-8"),
    ]);

    assert!(output.status.success());
}

#[test]
fn build_command_executes_with_typed_config_input() {
    let root = tempdir().expect("temporary directory should be created");
    write_directory_html_config_input_project(root.path());

    let output = run_moth(&[
        "build",
        root.path().to_str().expect("project path should be UTF-8"),
        "--input",
        "enabled=true",
    ]);

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(root.path().join("preview/index.html").is_file());
}

#[test]
fn check_command_executes_with_typed_config_input() {
    let root = tempdir().expect("temporary directory should be created");
    write_directory_html_config_input_project(root.path());

    let output = run_moth(&[
        "check",
        root.path().to_str().expect("project path should be UTF-8"),
        "--input",
        "enabled=true",
    ]);

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!root.path().join("preview").exists());
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
    write_directory_html_project(root.path(), INVALID_SOURCE);

    let output = run_moth(&[
        "build",
        root.path().to_str().expect("project path should be UTF-8"),
    ]);

    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn output_root_write_failure_exits_with_failure_without_benchmark_status() {
    let root = tempdir().expect("temporary directory should be created");
    write_directory_html_project(root.path(), "#[:<h1>Hello</h1>]\nentry = \".\"\n");
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

#[cfg(unix)]
#[test]
fn non_utf8_argument_exits_with_actionable_error_instead_of_panicking() {
    let output = Command::new(env!("CARGO_BIN_EXE_moth"))
        .arg(OsString::from_vec(b"bad\xff".to_vec()))
        .env_remove("MOTH_BENCH_STATUS")
        .output()
        .expect("moth subprocess should start");

    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Argument position 0"), "{stdout}");
    assert!(
        stdout.contains(&format!("'bad{}'", char::REPLACEMENT_CHARACTER)),
        "{stdout}"
    );
    assert!(
        stdout.contains("Moth arguments must be valid UTF-8"),
        "{stdout}"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stdout.contains("panicked") && !stderr.contains("panicked"),
        "CLI output must not contain a panic message:\nstdout: {stdout}\nstderr: {stderr}"
    );
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
    write_directory_html_project(build_root.path(), "#[:<h1>Hello</h1>]\nentry = \".\"\n");
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
    write_directory_html_project(build_root.path(), WARNING_SOURCE);
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
    write_directory_html_project(build_root.path(), INVALID_SOURCE);
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
#[cfg(target_os = "linux")]
fn create_non_utf8_working_directory(root: &Path) -> std::path::PathBuf {
    let working_directory = root.join(OsStr::from_bytes(b"working-\xff"));
    fs::create_dir(&working_directory).expect("non-UTF-8 working directory should be created");
    working_directory
}

#[cfg(target_os = "linux")]
#[test]
fn non_utf8_check_path_emits_terse_infrastructure_record_and_exits_one() {
    let root = tempdir().expect("temporary directory should be created");
    let working_directory = create_non_utf8_working_directory(root.path());
    let output = run_moth_from_dir(&working_directory, &["check", ".", "--terse"]);

    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let records = stdout
        .lines()
        .filter(|line| line.starts_with("E|MOTH-INFRA-0001|"))
        .collect::<Vec<_>>();
    assert_eq!(
        records.len(),
        1,
        "expected one infrastructure record: {stdout}"
    );
    let fields = records[0].splitn(5, '|').collect::<Vec<_>>();
    assert_eq!(fields[0], "E");
    assert_eq!(fields[1], "MOTH-INFRA-0001");
    assert!(!fields[2].is_empty(), "record should retain the host path");
    assert_eq!(fields[3], "");
    assert!(
        fields[4].contains("Could not pass non-UTF-8 absolute check path"),
        "record should describe the check entry path: {}",
        fields[4]
    );
    assert!(
        fields[4].contains("Stage: Project build/assembly"),
        "record should retain the project assembly stage: {}",
        fields[4]
    );
}

#[cfg(target_os = "linux")]
#[test]
fn non_utf8_build_path_uses_typed_infrastructure_failure_and_exits_one() {
    let root = tempdir().expect("temporary directory should be created");
    let working_directory = create_non_utf8_working_directory(root.path());
    let output = run_moth_from_dir(&working_directory, &["build", "."]);

    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Filesystem Failure"),
        "build path failure should use the typed filesystem renderer: {stdout}"
    );
    assert!(
        stdout.contains("Could not pass non-UTF-8 absolute build path"),
        "build path failure should retain its operation context: {stdout}"
    );
    assert!(
        stdout.contains("Stage: Project build/assembly"),
        "build path failure should retain the project assembly stage: {stdout}"
    );
}

#[cfg(feature = "test_compiler_bug")]
#[test]
fn compiler_bug_exit_is_distinct_and_reported_once() {
    let root = tempdir().expect("temporary directory should be created");
    let trigger = root.path().join("__moth_test_compiler_bug__");
    let output = run_moth(&[
        "check",
        trigger
            .to_str()
            .expect("compiler-bug test path should be UTF-8"),
    ]);

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        stderr.matches("COMPILER BUG: Moth encountered").count(),
        1,
        "the structured panic must be rendered exactly once: {stderr}"
    );
    assert!(
        !stderr.contains("thread 'main' panicked"),
        "the installed hook must suppress its duplicate rendering: {stderr}"
    );
}

#[cfg(feature = "test_compiler_bug")]
#[test]
fn build_compiler_bug_exit_is_distinct_and_reported_once() {
    let root = tempdir().expect("temporary directory should be created");
    let trigger = root.path().join("__moth_test_compiler_bug__");
    let output = run_moth(&[
        "build",
        trigger
            .to_str()
            .expect("compiler-bug test path should be UTF-8"),
    ]);

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        stderr.matches("COMPILER BUG: Moth encountered").count(),
        1,
        "the structured panic must be rendered exactly once: {stderr}"
    );
    assert!(
        !stderr.contains("thread 'main' panicked"),
        "the installed hook must suppress its duplicate rendering: {stderr}"
    );
}

#[cfg(all(feature = "boracle", feature = "test_compiler_bug"))]
#[test]
fn boracle_compiler_bug_exit_is_distinct_and_reported_once() {
    let root = tempdir().expect("temporary directory should be created");
    let trigger = root.path().join("__moth_test_compiler_bug__");
    let output = run_moth(&[
        "boracle",
        trigger
            .to_str()
            .expect("Boracle compiler-bug test path should be UTF-8"),
    ]);

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        stderr.matches("COMPILER BUG: Moth encountered").count(),
        1,
        "Boracle's structured compiler bug must render exactly once: {stderr}"
    );
    assert!(
        !stderr.contains("thread 'main' panicked"),
        "the installed hook must suppress its duplicate rendering: {stderr}"
    );
}

#[cfg(feature = "test_compiler_bug")]
#[test]
fn unknown_panic_uses_the_previous_hook_without_repeating_details() {
    let root = tempdir().expect("temporary directory should be created");
    let trigger = root.path().join("__moth_test_unknown_panic__");
    let output = run_moth(&[
        "check",
        trigger
            .to_str()
            .expect("unknown-panic test path should be UTF-8"),
    ]);

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        stderr
            .matches("test-only unknown panic hook detail")
            .count(),
        1,
        "the previous panic hook's unknown details must not be repeated: {stderr}"
    );
    assert!(stderr.contains("COMPILER BUG: an unknown panic interrupted this command;"));
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
