use super::*;
use std::fs;
use std::path::PathBuf;

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
        r"@echo off
echo|set /p="{}"
echo|set /p="{}" 1>&2
exit {}
"#,
        stdout, stderr, exit_code
    );

    fs::write(path, script).unwrap();
}

#[test]
fn test_run_moth_command_success() {
    let temp_dir = std::env::temp_dir();
    let mock_moth = temp_dir.join("mock_moth_success");

    #[cfg(unix)]
    let mock_moth = mock_moth.with_extension("");
    #[cfg(windows)]
    let mock_moth = mock_moth.with_extension("bat");

    create_mock_executable(&mock_moth, 0, "success output", "");

    let result = run_moth_command(&mock_moth, "check", &["test.moth".to_string()]);

    assert!(result.is_ok());
    let run = result.unwrap();
    assert!(run.success);
    assert!(run.duration_ms >= 0.0);
    assert!(run.stdout.contains("success output"));

    let _ = fs::remove_file(&mock_moth);
}

#[test]
fn test_run_moth_command_failure() {
    let temp_dir = std::env::temp_dir();
    let mock_moth = temp_dir.join("mock_moth_failure");

    #[cfg(unix)]
    let mock_moth = mock_moth.with_extension("");
    #[cfg(windows)]
    let mock_moth = mock_moth.with_extension("bat");

    create_mock_executable(&mock_moth, 1, "", "error output");

    let result = run_moth_command(&mock_moth, "check", &["test.moth".to_string()]);

    assert!(result.is_ok());
    let run = result.unwrap();
    assert!(!run.success);
    assert!(!run.stderr.is_empty());
    assert!(run.stderr.contains("error output"));

    let _ = fs::remove_file(&mock_moth);
}

#[test]
fn test_run_moth_command_nonexistent() {
    let nonexistent = PathBuf::from("/nonexistent/moth");
    let result = run_moth_command(&nonexistent, "check", &["test.moth".to_string()]);

    assert!(result.is_err());
}
