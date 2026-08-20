//! Explicit filesystem assertion helpers for xtask tests.
//!
//! WHAT: narrow helpers that distinguish `NotFound` from every other IO result.
//! WHY: `Path::exists` returns false for any metadata failure and follows
//!   symlinks, so a permission error or a dangling link reads as absence. The
//!   compiler crate owns the same contract in `src/compiler_tests/test_fs.rs`;
//!   xtask is a separate crate and needs its own copy of the narrow API rather
//!   than a weaker predicate.

use std::path::Path;

/// Assert that no filesystem node exists at `path`.
///
/// Uses `symlink_metadata` so a dangling symlink is detected as an existing
/// node, not absence. Only `NotFound` is accepted as missing.
#[track_caller]
pub fn assert_path_missing(path: &Path) {
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Ok(metadata) => panic!("expected no filesystem node at {path:?}, found {metadata:?}"),
        Err(error) => panic!("failed to inspect {path:?} for absence assertion: {error}"),
    }
}

/// Assert that `path` is a regular file (not a directory or symlink).
#[track_caller]
#[allow(dead_code)]
pub fn assert_regular_file(path: &Path) {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => assert!(
            metadata.is_file(),
            "expected a regular file at {path:?}, found {metadata:?}"
        ),
        Err(error) => panic!("failed to inspect {path:?} for regular-file assertion: {error}"),
    }
}

/// Assert that `path` is a directory (not a file or symlink).
#[track_caller]
#[allow(dead_code)]
pub fn assert_directory(path: &Path) {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => assert!(
            metadata.is_dir(),
            "expected a directory at {path:?}, found {metadata:?}"
        ),
        Err(error) => panic!("failed to inspect {path:?} for directory assertion: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assert_path_missing_accepts_absent_path() {
        let dir = tempfile::tempdir().expect("should create temp dir");
        assert_path_missing(&dir.path().join("absent"));
    }

    #[test]
    #[should_panic(expected = "expected no filesystem node at")]
    fn assert_path_missing_rejects_existing_file() {
        let dir = tempfile::tempdir().expect("should create temp dir");
        let file = dir.path().join("present.txt");
        std::fs::write(&file, b"data").expect("should write file");
        assert_path_missing(&file);
    }

    #[test]
    #[cfg(unix)]
    #[should_panic(expected = "expected no filesystem node at")]
    fn assert_path_missing_rejects_dangling_symlink() {
        let dir = tempfile::tempdir().expect("should create temp dir");
        let link = dir.path().join("dangling");
        std::os::unix::fs::symlink(dir.path().join("no_such_target"), &link)
            .expect("should create symlink");
        assert_path_missing(&link);
    }
}
