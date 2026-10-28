//! Tests for dev-server orchestration and entry-path validation.

use super::{DevServerOptions, resolve_dev_runtime_paths, validate_dev_entry_path};

use std::fs;

#[test]
fn defaults_match_dev_server_contract() {
    let defaults = DevServerOptions::default();
    assert_eq!(defaults.host, "127.0.0.1");
    assert_eq!(defaults.port, 6342);
    assert_eq!(defaults.poll_interval_ms, 300);
    assert!(
        defaults.inputs.is_empty(),
        "default dev options start with no explicit build inputs"
    );
}

#[test]
fn entry_path_validation_accepts_moth_files() {
    let temp = tempfile::tempdir().expect("create temp directory");
    let file = temp.path().join("main.moth");
    fs::write(&file, "x = 1").expect("write Moth entry");

    let validated = validate_dev_entry_path(file.to_str().expect("UTF-8 test path"))
        .expect("valid Moth path should pass validation");

    assert!(validated.is_absolute());
    assert!(validated.ends_with("main.moth"));
}

#[test]
fn empty_entry_path_uses_current_directory() {
    let expected = std::env::current_dir()
        .expect("current directory should resolve")
        .canonicalize()
        .expect("current directory should canonicalize");
    let validated = validate_dev_entry_path("").expect("empty path should use current directory");
    assert_eq!(validated, expected);
}

#[test]
fn entry_path_validation_preserves_missing_file_cause() {
    let temp = tempfile::tempdir().expect("create temp directory");
    let missing_path = temp.path().join("missing.moth");
    let error = validate_dev_entry_path(missing_path.to_str().expect("UTF-8 test path"))
        .expect_err("missing dev entry must fail path validation");

    assert_eq!(
        error.kind,
        crate::compiler_frontend::compiler_errors::InfrastructureFailureKind::Filesystem
    );
    assert_eq!(error.host_path.as_deref(), Some(missing_path.as_path()));
    assert_eq!(&*error.message, "Path does not exist");
}

#[test]
fn entry_path_validation_accepts_directories() {
    let temp = tempfile::tempdir().expect("create temp directory");
    let validated = validate_dev_entry_path(temp.path().to_str().expect("UTF-8 test path"))
        .expect("directories should be accepted");

    assert_eq!(
        validated,
        temp.path().canonicalize().expect("canonical directory")
    );
}

#[test]
fn initial_runtime_paths_watch_the_project_without_bootstrapping() {
    let temp = tempfile::tempdir().expect("create temp directory");
    let root = temp
        .path()
        .canonicalize()
        .expect("project root should canonicalize");
    fs::write(root.join("config.moth"), "not valid Moth config")
        .expect("write invalid config to exercise bootstrap-free startup paths");

    let resolved = resolve_dev_runtime_paths(&root);

    assert_eq!(resolved.output_dir, root.join("dev"));
    assert_eq!(resolved.watch_scope.output_dir, root.join("dev"));
    assert!(
        resolved.watch_scope.watches_path(&root.join("config.moth")),
        "initial watch scope must include config changes after a failed first build"
    );
}

#[test]
fn single_file_runtime_paths_use_the_parent_dev_directory() {
    let temp = tempfile::tempdir().expect("create temp directory");
    let entry = temp.path().join("main.moth");
    fs::write(&entry, "x = 1").expect("write Moth entry");
    let entry = entry.canonicalize().expect("entry should canonicalize");

    let resolved = resolve_dev_runtime_paths(&entry);

    assert_eq!(
        resolved.output_dir,
        entry.parent().expect("entry has a parent").join("dev")
    );
    assert!(
        resolved.watch_scope.watches_path(&entry),
        "single-file entry must be watched from the initial startup boundary"
    );
}
