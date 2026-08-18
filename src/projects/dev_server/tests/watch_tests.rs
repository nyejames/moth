//! Tests for dev-server watch scope derivation and change detection helpers.

use super::{
    FileFingerprint, WatchScope, WatchSession, WatchTarget, collect_fingerprints, detect_changes,
    fingerprint_from_modified, should_ignore_path,
};
use crate::compiler_tests::test_support::unused_temp_path;
use crate::projects::settings::{CONFIG_FILE_NAME, Config};
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

#[test]
fn detects_added_and_removed_files() {
    let mut previous = HashMap::new();
    let mut current = HashMap::new();
    let path_a = PathBuf::from("a.moth");
    let path_b = PathBuf::from("b.moth");

    previous.insert(
        path_a.clone(),
        FileFingerprint {
            modified: SystemTime::UNIX_EPOCH,
            len: 10,
        },
    );
    current.insert(
        path_a,
        FileFingerprint {
            modified: SystemTime::UNIX_EPOCH,
            len: 10,
        },
    );
    current.insert(
        path_b,
        FileFingerprint {
            modified: SystemTime::UNIX_EPOCH,
            len: 1,
        },
    );

    assert!(detect_changes(&previous, &current));
}

#[test]
fn detects_modified_file_fingerprints() {
    let path = PathBuf::from("test.moth");
    let mut previous = HashMap::new();
    let mut current = HashMap::new();

    previous.insert(
        path.clone(),
        FileFingerprint {
            modified: SystemTime::UNIX_EPOCH,
            len: 5,
        },
    );
    current.insert(
        path,
        FileFingerprint {
            modified: SystemTime::UNIX_EPOCH + Duration::from_secs(1),
            len: 5,
        },
    );

    assert!(detect_changes(&previous, &current));
}

#[test]
fn directory_scope_ignores_legacy_package_folders() {
    let root = unused_temp_path("directory_scope");
    let output_dir = root.join("dev");
    fs::create_dir_all(root.join("src")).expect("should create src dir");
    fs::create_dir_all(root.join("assets/vendor")).expect("should create assets dir");
    fs::write(root.join("src/helper.js"), "export function draw() {}")
        .expect("should write entry-root js file");
    fs::write(
        root.join("assets/vendor/lib.js"),
        "export function lib() {}",
    )
    .expect("should write nested package-folder js file");
    let canonical_root = root.canonicalize().expect("root should canonicalize");

    let mut config = Config::new(root.clone());
    config.entry_root = PathBuf::from("src");
    config.package_folders = vec![PathBuf::from("assets")];

    let scope = WatchScope::derive(&root, Some(&config), &output_dir);

    assert!(scope.watches_path(&canonical_root.join(CONFIG_FILE_NAME)));
    assert!(scope.watches_path(&canonical_root.join("src/main.moth")));
    assert!(scope.watches_path(&canonical_root.join("src/helper.js")));
    assert!(!scope.watches_path(&canonical_root.join("assets/logo.png")));
    assert!(!scope.watches_path(&canonical_root.join("assets/vendor/lib.js")));
    assert!(!scope.watches_path(&canonical_root.join("target/debug/app")));
    assert!(!scope.watches_path(&canonical_root.join("dev/bundle.js")));

    fs::remove_dir_all(&root).expect("should remove temp test dir");
}

#[test]
fn directory_scope_without_config_watches_entry_directory() {
    let root = unused_temp_path("directory_scope_without_config");
    let output_dir = root.join("dev");
    fs::create_dir_all(root.join("src")).expect("should create src dir");
    fs::create_dir_all(&output_dir).expect("should create output dir");
    let canonical_root = root.canonicalize().expect("root should canonicalize");

    let scope = WatchScope::derive(&root, None, &output_dir);

    assert!(scope.watches_path(&canonical_root.join("src/main.moth")));
    assert!(!scope.watches_path(&canonical_root.join("dev/main.html")));

    fs::remove_dir_all(&root).expect("should remove temp test dir");
}

#[test]
fn scanner_only_scans_declared_watch_targets() {
    let root = unused_temp_path("watch_scan");
    let output_dir = root.join("dev");
    let src_dir = root.join("src");
    let unrelated_dir = root.join("target");
    fs::create_dir_all(&output_dir).expect("should create output dir");
    fs::create_dir_all(&src_dir).expect("should create source dir");
    fs::create_dir_all(&unrelated_dir).expect("should create unrelated dir");

    fs::write(src_dir.join("main.moth"), "main").expect("should write source file");
    fs::write(output_dir.join("bundle.js"), "js").expect("should write output file");
    fs::write(unrelated_dir.join("debug.txt"), "ignore me").expect("should write unrelated file");

    let scope = WatchScope {
        output_dir: output_dir.clone(),
        targets: vec![WatchTarget {
            watch_path: src_dir.clone(),
            interest_path: None,
            recursive: true,
        }],
    };

    let fingerprints = collect_fingerprints(&scope).expect("scanner should complete");
    assert!(fingerprints.keys().all(|path| path.starts_with(&src_dir)));
    assert!(fingerprints.keys().any(|path| path.ends_with("main.moth")));
    assert!(
        fingerprints
            .keys()
            .all(|path| !path.starts_with(&output_dir)),
        "scanner should ignore output directory files"
    );

    fs::remove_dir_all(&root).expect("should remove temp test dir");
}

#[test]
fn manual_watch_session_coalesces_bursty_changes() {
    let scope = WatchScope {
        output_dir: PathBuf::from("dev"),
        targets: vec![WatchTarget {
            watch_path: PathBuf::from("src"),
            interest_path: None,
            recursive: true,
        }],
    };
    let (session, trigger) = WatchSession::manual(scope);

    trigger.notify_change();
    trigger.notify_change();
    trigger.notify_change();

    let seen_revision = session
        .wait_for_stable_change(0)
        .expect("manual watch session should settle");
    assert_eq!(seen_revision, 3);
}

#[test]
fn ignore_rules_cover_git_output_and_editor_temp_files() {
    let root = PathBuf::from("/tmp/project");
    let output_dir = root.join("dev");
    assert!(should_ignore_path(&root.join(".git/index"), &output_dir));
    assert!(should_ignore_path(&root.join("dev/main.js"), &output_dir));
    assert!(should_ignore_path(
        &root.join("src/main.moth.swp"),
        &output_dir
    ));
    assert!(should_ignore_path(
        &root.join("src/#main.moth#"),
        &output_dir
    ));
    assert!(!should_ignore_path(
        &root.join("src/main.moth"),
        &output_dir
    ));
}

#[test]
fn exact_file_target_collects_single_fingerprint() {
    let root = unused_temp_path("watch_exact_file");
    let output_dir = root.join("dev");
    fs::create_dir_all(&root).expect("should create temp test dir");
    let source_file = root.join("page.moth");
    fs::write(&source_file, "hello").expect("should write source file");

    let scope = WatchScope {
        output_dir: output_dir.clone(),
        targets: vec![WatchTarget {
            watch_path: source_file.clone(),
            interest_path: Some(source_file.clone()),
            recursive: false,
        }],
    };

    let fingerprints = collect_fingerprints(&scope).expect("exact-file scan should complete");
    assert_eq!(
        fingerprints.len(),
        1,
        "exact-file target should collect one file"
    );
    let fingerprint = fingerprints
        .get(&source_file)
        .expect("source file should be fingerprinted");
    assert_eq!(fingerprint.len, 5);

    fs::remove_dir_all(&root).expect("should remove temp test dir");
}

#[test]
fn recursive_directory_target_collects_nested_files() {
    let root = unused_temp_path("watch_recursive_directory");
    let output_dir = root.join("dev");
    let src_dir = root.join("src");
    let nested_dir = src_dir.join("pages");
    fs::create_dir_all(&nested_dir).expect("should create nested dirs");
    fs::write(src_dir.join("main.moth"), "main").expect("should write main");
    fs::write(nested_dir.join("about.moth"), "about").expect("should write nested file");

    let scope = WatchScope {
        output_dir: output_dir.clone(),
        targets: vec![WatchTarget {
            watch_path: src_dir.clone(),
            interest_path: None,
            recursive: true,
        }],
    };

    let fingerprints = collect_fingerprints(&scope).expect("recursive scan should complete");
    assert_eq!(
        fingerprints.len(),
        2,
        "recursive target should collect nested files"
    );
    assert!(fingerprints.keys().any(|path| path.ends_with("main.moth")));
    assert!(fingerprints.keys().any(|path| path.ends_with("about.moth")));

    fs::remove_dir_all(&root).expect("should remove temp test dir");
}

#[test]
fn timestamp_failure_propagates_with_path_context() {
    let path = PathBuf::from("unreachable.moth");
    let result = fingerprint_from_modified(
        Err(io::Error::from(io::ErrorKind::PermissionDenied)),
        12,
        &path,
    );

    let error = result.expect_err("modified-time failure should propagate");
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    assert!(
        error.to_string().contains("unreachable.moth"),
        "error should name the affected path: {error}"
    );
}

#[test]
fn same_length_edit_detected_via_timestamp_change() {
    let root = unused_temp_path("watch_same_length_edit");
    let output_dir = root.join("dev");
    fs::create_dir_all(&root).expect("should create temp test dir");
    let source_file = root.join("page.moth");
    fs::write(&source_file, "first value").expect("should write initial content");

    let scope = WatchScope {
        output_dir: output_dir.clone(),
        targets: vec![WatchTarget {
            watch_path: source_file.clone(),
            interest_path: Some(source_file.clone()),
            recursive: false,
        }],
    };

    let before = collect_fingerprints(&scope).expect("initial scan should complete");
    let before_fingerprint = before
        .get(&source_file)
        .expect("source file should be fingerprinted before edit");

    fs::write(&source_file, "second word").expect("should rewrite same-length content");
    let advanced_modified = before_fingerprint
        .modified
        .checked_add(Duration::from_secs(1))
        .expect("test timestamp should advance");
    let file = fs::OpenOptions::new()
        .write(true)
        .open(&source_file)
        .expect("should reopen source file");
    let set_times_result = file.set_times(fs::FileTimes::new().set_modified(advanced_modified));
    drop(file);
    if let Err(error) = set_times_result {
        if error.kind() == io::ErrorKind::Unsupported {
            fs::remove_dir_all(&root).expect("should remove temp test dir");
            return;
        }
        panic!("supported modified-time update should succeed: {error}");
    }

    let after = collect_fingerprints(&scope).expect("post-edit scan should complete");
    let after_fingerprint = after
        .get(&source_file)
        .expect("source file should be fingerprinted after edit");
    assert_eq!(after_fingerprint.len, before_fingerprint.len);
    assert_ne!(after_fingerprint.modified, before_fingerprint.modified);
    assert!(
        detect_changes(&before, &after),
        "same-length edit with a changed timestamp must be detected"
    );

    fs::remove_dir_all(&root).expect("should remove temp test dir");
}
