use super::*;
use std::fs;
use tempfile::tempdir;

// Helper to create a mock executable for testing
#[cfg(unix)]
fn create_mock_executable(path: &std::path::Path, exit_code: i32, stdout: &str, stderr: &str) {
    use std::os::unix::fs::PermissionsExt;

    let script = format!(
        r#"#!/bin/sh
echo -n "{}"
echo -n "{}" >&2
exit {}
"#,
        stdout, stderr, exit_code
    );

    fs::write(path, script).unwrap();
    let mut perms = fs::metadata(path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(path, perms).unwrap();
}

#[cfg(windows)]
fn create_mock_executable(path: &std::path::Path, exit_code: i32, stdout: &str, stderr: &str) {
    let script = format!(
        r#"@echo off
echo|set /p="{}"
echo|set /p="{}" 1>&2
exit {}
"#,
        stdout, stderr, exit_code
    );

    fs::write(path, script).unwrap();
}

#[cfg(unix)]
fn create_working_directory_executable(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;

    fs::write(path, "#!/bin/sh\npwd\n").unwrap();
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}

#[cfg(windows)]
fn create_working_directory_executable(path: &std::path::Path) {
    fs::write(path, "@echo off\ncd\n").unwrap();
}

#[cfg(unix)]
fn create_environment_executable(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;

    fs::write(
        path,
        "#!/bin/sh\nprintf '%s|%s|%s' \"$MOTH_TIMERS\" \"$MOTH_COUNTERS\" \"$MOTH_BENCH_STATUS\"\n",
    )
    .unwrap();
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}

#[cfg(windows)]
fn create_environment_executable(path: &std::path::Path) {
    fs::write(
        path,
        "@echo off\necho|set /p=\"%MOTH_TIMERS%|%MOTH_COUNTERS%|%MOTH_BENCH_STATUS%\"\n",
    )
    .unwrap();
}

#[test]
fn test_run_moth_command_success() {
    let temp_dir = tempdir().expect("temporary directory should exist");
    let mock_moth = temp_dir.path().join("mock_moth_success");

    #[cfg(unix)]
    let mock_moth = mock_moth.with_extension("");
    #[cfg(windows)]
    let mock_moth = mock_moth.with_extension("bat");

    create_mock_executable(&mock_moth, 0, "success output", "");

    let result = run_moth_command(
        &mock_moth,
        temp_dir.path(),
        "check",
        &["test.moth".to_string()],
    );

    assert!(result.is_ok());
    let run = result.unwrap();
    assert!(run.status.success);
    assert_eq!(run.status.code, Some(0));
    assert!(run.duration_ms >= 0.0);
    assert!(run.stdout.contains("success output"));
}

#[test]
fn test_run_moth_command_failure() {
    let temp_dir = tempdir().expect("temporary directory should exist");
    let mock_moth = temp_dir.path().join("mock_moth_failure");

    #[cfg(unix)]
    let mock_moth = mock_moth.with_extension("");
    #[cfg(windows)]
    let mock_moth = mock_moth.with_extension("bat");

    create_mock_executable(&mock_moth, 1, "", "error output");

    let result = run_moth_command(
        &mock_moth,
        temp_dir.path(),
        "check",
        &["test.moth".to_string()],
    );

    assert!(result.is_ok());
    let run = result.unwrap();
    assert!(!run.status.success);
    assert_eq!(run.status.code, Some(1));
    assert!(!run.stderr.is_empty());
    assert!(run.stderr.contains("error output"));
}

#[test]
fn run_moth_command_sets_complete_benchmark_environment() {
    let temp_dir = tempdir().expect("temporary directory should exist");
    let mock_moth = temp_dir.path().join("mock_moth_environment");

    #[cfg(unix)]
    let mock_moth = mock_moth.with_extension("");
    #[cfg(windows)]
    let mock_moth = mock_moth.with_extension("bat");

    create_environment_executable(&mock_moth);

    let run = run_moth_command(&mock_moth, temp_dir.path(), "check", &[])
        .expect("mock command should run");

    assert_eq!(run.stdout, "bench|off|1");
}

#[test]
fn run_moth_command_uses_declared_current_directory() {
    let temp_dir = tempdir().expect("temporary directory should exist");
    let working_directory = temp_dir.path().join("repository");
    fs::create_dir(&working_directory).expect("working directory should be creatable");
    let mock_moth = temp_dir.path().join("mock_moth_working_directory");

    #[cfg(unix)]
    let mock_moth = mock_moth.with_extension("");
    #[cfg(windows)]
    let mock_moth = mock_moth.with_extension("bat");

    create_working_directory_executable(&mock_moth);

    let run = run_moth_command(&mock_moth, &working_directory, "check", &[])
        .expect("mock command should run");
    let reported_directory =
        fs::canonicalize(run.stdout.trim()).expect("reported directory should canonicalise");
    let expected_directory =
        fs::canonicalize(working_directory).expect("working directory should canonicalise");

    assert_eq!(reported_directory, expected_directory);
}

#[test]
fn test_run_moth_command_nonexistent() {
    let temp_dir = tempdir().expect("temporary directory should exist");
    let nonexistent = temp_dir.path().join("missing-moth");
    let result = run_moth_command(
        &nonexistent,
        temp_dir.path(),
        "check",
        &["test.moth".to_string()],
    );

    assert!(result.is_err());
}
