//! WHAT: verifies the Moth CLI's subprocess exit-status and benchmark-status contracts.
//! WHY: benchmark and tooling callers must observe deterministic process outcomes without
//!      inheriting benchmark-only environment state from the test process.

#[cfg(target_os = "linux")]
use std::ffi::OsStr;
#[cfg(unix)]
use std::ffi::OsString;
use std::fs;
use std::io::Write;
#[cfg(target_os = "linux")]
use std::os::unix::ffi::OsStrExt;
#[cfg(unix)]
use std::os::unix::ffi::OsStringExt;
use std::path::Path;
use std::process::{Command, Output, Stdio};

use tempfile::tempdir;

const WARNING_SOURCE: &str = "value ~= \"hello\"\n\
result ~= \"unset\"\n\
if value is:\n\
    \"one\" => ~result = \"one\"\n\
    \"one\" => ~result = \"one\"\n\
    \"one\" => ~result = \"one\"\n\
    \"one\" => ~result = \"one\"\n\
    \"two\" => ~result = \"two\"\n\
    else => ~result = \"other\"\n\
;\n";

const INVALID_SOURCE: &str = "value = (\n";

/// Two independent ordinary bodies that fail on unknown value names.
const TWO_FAILING_BODIES_SOURCE: &str = "\
first || -> I32:
    return missing_one
;

second || -> I32:
    return missing_two
;
";

/// Directory-project config retaining one module error before discovery stops.
const MODULE_LIMIT_CONFIG_SOURCE: &str =
    "project #= (\n    name = \"docs\",\n    max_errors_per_module = 1,\n)\nhtml #= ()\n";

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
    fs::write(
        root.join("config.moth"),
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
    )
    .expect("config should be written");
    fs::write(root.join("@entry.moth"), page_source).expect("page source should be written");
}

fn write_directory_html_config_input_project(root: &Path) {
    fs::write(
        root.join("config.moth"),
        "project #= (\n    name = \"docs\",\n)\nhtml #= (\n    dev_output = \"preview\",\n    release_output = \"release\",\n)\n",
    )
    .expect("config should be written");
    fs::write(
        root.join("@entry.moth"),
        "$config\nenabled #Bool\nresult ~= 0\nif enabled:\n    ~result = 1\n;\n",
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

#[cfg(unix)]
#[test]
fn cross_directory_root_alias_is_a_typed_diagnostic_not_a_compiler_bug() {
    use std::os::unix::fs::symlink;

    let root = tempdir().expect("temporary directory should be created");
    let project = root.path().join("project");
    fs::create_dir_all(project.join("child")).expect("child directory should be created");
    write_directory_html_project(&project, "page_title #= \"root\"\n");
    symlink(
        project.join("@entry.moth"),
        project.join("child/@entry.moth"),
    )
    .expect("child root symlink should be created");

    let output = run_moth(&["check", project.to_str().expect("path should be UTF-8")]);
    let report = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    assert_eq!(output.status.code(), Some(1), "report: {report}");
    assert!(
        report.contains("resolve to the same physical source file"),
        "the root alias must be a typed layout diagnostic: {report}"
    );
    assert!(
        !report.contains("COMPILER BUG"),
        "the root alias must not escape as a compiler bug: {report}"
    );
}

fn write_support_scope_project(project: &Path) {
    fs::create_dir_all(project.join("pages/extras")).expect("pages extras should be created");
    write_source(
        project,
        "config.moth",
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
    );
    write_source(project, "@mod.moth", "");
    write_source(project, "pages/@mod.moth", "");
    write_source(
        project,
        "pages/extras/@package.moth",
        "$export\nextra #= 1\n",
    );
}

fn terse_diagnostic_detail(report: &str, code: &str) -> String {
    report
        .lines()
        .find(|line| line.starts_with(&format!("E|{code}|")))
        .and_then(|line| line.splitn(5, '|').nth(4))
        .unwrap_or_else(|| panic!("expected a terse {code} diagnostic in: {report}"))
        .to_owned()
}

#[test]
fn support_scope_diagnostic_matches_canonical_and_check_only_lanes() {
    let root = tempdir().expect("temporary directory should be created");
    let canonical_project = root.path().join("canonical");
    write_support_scope_project(&canonical_project);
    write_source(&canonical_project, "pages/@package.moth", "@extras extra\n");

    let canonical = run_moth(&[
        "check",
        canonical_project
            .to_str()
            .expect("canonical path should be UTF-8"),
        "--terse",
    ]);
    let canonical_report = format!(
        "{}{}",
        String::from_utf8_lossy(&canonical.stdout),
        String::from_utf8_lossy(&canonical.stderr)
    );
    assert_eq!(canonical.status.code(), Some(1), "{canonical_report}");
    let canonical_detail = terse_diagnostic_detail(&canonical_report, "MOTH-IMPORT-0015");

    let check_only_project = root.path().join("check_only");
    write_support_scope_project(&check_only_project);
    write_source(
        &check_only_project,
        "pages/@package.moth",
        "$export\nmarker #= 1\n",
    );
    write_source(&check_only_project, "pages/orphan.moth", "@extras extra\n");

    let check_only = run_moth(&[
        "check",
        check_only_project
            .to_str()
            .expect("check-only path should be UTF-8"),
        "--terse",
    ]);
    let check_only_report = format!(
        "{}{}",
        String::from_utf8_lossy(&check_only.stdout),
        String::from_utf8_lossy(&check_only.stderr)
    );
    assert_eq!(check_only.status.code(), Some(1), "{check_only_report}");
    let check_only_detail = terse_diagnostic_detail(&check_only_report, "MOTH-IMPORT-0015");

    assert_eq!(
        check_only_detail, canonical_detail,
        "canonical and check-only preparation must emit the same support diagnostic"
    );
}

#[test]
fn paired_provider_check_only_uses_the_selected_surface() {
    let root = tempdir().expect("temporary directory should be created");
    let project = root.path().join("project");
    fs::create_dir_all(project.join("consumer")).expect("consumer directory should be created");
    fs::create_dir_all(project.join("pkg")).expect("provider directory should be created");

    write_source(
        &project,
        "config.moth",
        "project #= (name = \"check_only\")\nhtml #= ()\n",
    );
    write_source(&project, "@entry.moth", "@consumer run\n[:run]\n");
    write_source(&project, "consumer/@mod.moth", "$export\nrun #= \"ok\"\n");
    write_source(
        &project,
        "consumer/orphan.moth",
        "@pkg support_only\norphan || -> String:\n    return support_only\n;\n",
    );
    write_source(
        &project,
        "pkg/@package.moth",
        "$export\nsupport_only #= \"support\"\n",
    );
    write_source(
        &project,
        "pkg/@mod.moth",
        "$export\nnormal_only #= \"normal\"\n",
    );

    let check = run_moth_from_dir(&project, &["check", ".", "--terse"]);
    assert!(
        check.status.success(),
        "check-only support import failed: {}",
        String::from_utf8_lossy(&check.stderr)
    );

    // The orphan must really enter check-only compilation, and it must bind the support
    // interface rather than silently falling back to the paired normal interface.
    write_source(
        &project,
        "consumer/orphan.moth",
        "@pkg normal_only\norphan || -> String:\n    return normal_only\n;\n",
    );
    let wrong_surface = run_moth_from_dir(&project, &["check", ".", "--terse"]);
    let report = format!(
        "{}{}",
        String::from_utf8_lossy(&wrong_surface.stdout),
        String::from_utf8_lossy(&wrong_surface.stderr)
    );
    assert!(!wrong_surface.status.success());
    assert!(
        report.contains("MOTH-IMPORT-0011"),
        "expected support-surface rejection, got: {report}"
    );
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
    write_directory_html_project(root.path(), "[:<h1>Hello</h1>]\nentry = \".\"\n");

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
    assert!(root.path().join("_preview/index.html").is_file());
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
    assert!(!root.path().join("_preview").exists());
}

#[test]
fn check_orders_the_module_stop_before_the_terse_summary_and_omits_it_when_complete() {
    let root = tempdir().expect("temporary directory should be created");
    fs::write(root.path().join("config.moth"), MODULE_LIMIT_CONFIG_SOURCE)
        .expect("limited config should be written");
    fs::write(root.path().join("@entry.moth"), TWO_FAILING_BODIES_SOURCE)
        .expect("page source should be written");
    let stop_line =
        "Diagnostic discovery stopped early (module error limit): later code was not checked.";

    let terse = run_moth_from_dir(root.path(), &["check", ".", "--terse"]);
    assert_eq!(terse.status.code(), Some(1));
    let terse_stdout = String::from_utf8_lossy(&terse.stdout);
    let terse_lines = terse_stdout.lines().collect::<Vec<_>>();
    assert_eq!(
        terse_lines.last().copied(),
        Some("errors=1, warnings=0."),
        "the terse summary must stay the final line: {terse_stdout}"
    );
    assert_eq!(
        terse_lines
            .get(terse_lines.len().saturating_sub(2))
            .copied(),
        Some(stop_line),
        "the stop line must precede the terse summary: {terse_stdout}"
    );
    assert_eq!(terse_stdout.matches(stop_line).count(), 1);

    let default = run_moth_from_dir(root.path(), &["check", "."]);
    assert_eq!(default.status.code(), Some(1));
    let default_stdout = String::from_utf8_lossy(&default.stdout);
    assert_eq!(
        default_stdout.matches(stop_line).count(),
        1,
        "the default check output must explain the stop exactly once: {default_stdout}"
    );
    let last_diagnostic = default_stdout
        .rfind("missing_one")
        .expect("default check output must retain the retained diagnostic");
    assert!(
        last_diagnostic
            < default_stdout
                .find(stop_line)
                .expect("default check output must include the stop line"),
        "the stop explanation must follow the diagnostics: {default_stdout}"
    );

    let complete_root = tempdir().expect("temporary directory should be created");
    write_directory_html_project(complete_root.path(), TWO_FAILING_BODIES_SOURCE);
    let complete = run_moth_from_dir(complete_root.path(), &["check", "."]);
    assert_eq!(complete.status.code(), Some(1));
    let complete_stdout = String::from_utf8_lossy(&complete.stdout);
    assert!(
        !complete_stdout.contains("Diagnostic discovery stopped early"),
        "a complete diagnosed result must not print a stop line: {complete_stdout}"
    );
}

#[test]
fn single_file_build_writes_outputs_using_containing_directory_context() {
    let root = tempdir().expect("temporary directory should be created");
    let project_dir = root.path().join("project");
    fs::create_dir(&project_dir).expect("single-file project directory should be created");
    write_source(&project_dir, "main.moth", "[:<h1>Hello</h1>]\n");

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
    write_source(root.path(), "main.moth", "[:<h1>Hello</h1>]\n");

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
    write_directory_html_project(root.path(), "[:<h1>Hello</h1>]\nentry = \".\"\n");
    fs::write(root.path().join("_dev"), b"occupied output root")
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
    write_directory_html_project(build_root.path(), "[:<h1>Hello</h1>]\nentry = \".\"\n");
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

#[test]
fn scaffolded_project_checks_and_builds_with_generated_config() {
    let temp = tempdir().expect("should create temp dir");
    let project = temp.path().join("my-site");
    let project_argument = project.to_string_lossy().into_owned();

    let mut child = Command::new(env!("CARGO_BIN_EXE_moth"))
        .args(["new", "html", &project_argument])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("should start moth new");
    child
        .stdin
        .as_mut()
        .expect("stdin should be piped")
        .write_all(b"y\n\nY\n")
        .expect("should answer scaffold prompts");
    let scaffold = child.wait_with_output().expect("should finish moth new");
    assert!(
        scaffold.status.success(),
        "scaffold failed: {}",
        String::from_utf8_lossy(&scaffold.stderr)
    );

    let config = fs::read_to_string(project.join("config.moth"))
        .expect("generated config should be readable");
    assert!(
        config.contains("name = \"my_site\""),
        "invalid generated config: {config}"
    );
    assert!(!config.contains("entry_root"));
    assert!(!config.contains("version"));

    let check = run_moth(&["check", &project_argument, "--terse"]);
    assert!(
        check.status.success(),
        "scaffolded check failed: {}",
        String::from_utf8_lossy(&check.stdout)
    );

    let build = run_moth(&["build", &project_argument]);
    assert!(
        build.status.success(),
        "scaffolded dev build failed: {}",
        String::from_utf8_lossy(&build.stdout)
    );
    let html = fs::read_to_string(project.join("_dev/index.html"))
        .expect("scaffolded document should be readable");
    assert!(html.contains("<title>Welcome</title>"), "{html}");
    let head = html.split_once("</head>").expect("document has a head").0;
    assert!(head.contains("background-color: light-dark("), "{head}");
    assert!(head.contains("padding: var(--moth-spacing)"), "{head}");

    let release = run_moth(&["build", &project_argument, "--release"]);
    assert!(
        release.status.success(),
        "scaffolded release build failed: {}",
        String::from_utf8_lossy(&release.stdout)
    );
    assert!(project.join("_release/index.html").is_file());
}

#[test]
fn check_only_shared_helper_rejects_lateral_support_import() {
    let root = tempdir().expect("temporary directory should be created");
    let project = root.path().join("project");
    fs::create_dir_all(project.join("pkg")).expect("provider directory should be created");

    write_source(
        &project,
        "config.moth",
        "project #= (name = \"check_only\")\nhtml #= ()\n",
    );
    write_source(&project, "@entry.moth", "[:ok]\n");
    write_source(
        &project,
        "@package.moth",
        "$export\nroot_support #= \"root\"\n",
    );
    write_source(
        &project,
        "shared.moth",
        "@pkg support_only\n\nhelper || -> String:\n    return support_only\n;\n",
    );
    write_source(
        &project,
        "pkg/@package.moth",
        "$export\nsupport_only #= \"support\"\n",
    );

    let check = run_moth_from_dir(&project, &["check", ".", "--terse"]);
    let report = format!(
        "{}{}",
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr)
    );
    assert!(
        !check.status.success(),
        "a check-only shared helper must reject a lateral support import: {report}"
    );
    assert!(
        report.contains("MOTH-IMPORT-0015") && report.contains("shared.moth"),
        "expected shared-helper support rejection, got: {report}"
    );
}
