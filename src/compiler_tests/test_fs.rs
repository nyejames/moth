//! Explicit filesystem assertion helpers for tests.
//!
//! WHAT: narrow helpers that distinguish `NotFound` from every other IO result.
//! WHY: `Path::exists`, `Path::is_file` and `Path::is_dir` return false for any
//!   metadata failure, turning permission errors and dangling symlinks into
//!   false absence. These helpers make the IO outcome explicit at every
//!   assertion boundary.
//!
//! Every helper uses `#[track_caller]` and includes the path and underlying IO
//! error in failure output so the caller location and the filesystem state are
//! both visible in a panic message.

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
///
/// Uses `symlink_metadata` to detect symlinks. A symlink to a regular file is
/// a symlink, not a regular file, and fails this assertion.
#[track_caller]
pub fn assert_regular_file(path: &Path) {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => {
            assert!(
                metadata.is_file(),
                "expected a regular file at {path:?}, found {metadata:?}"
            );
        }
        Err(error) => panic!("failed to inspect {path:?} for regular-file assertion: {error}"),
    }
}

/// Assert that `path` is a directory (not a file or symlink).
///
/// Uses `symlink_metadata` to detect symlinks. A symlink to a directory is
/// a symlink, not a directory, and fails this assertion.
#[track_caller]
pub fn assert_directory(path: &Path) {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => {
            assert!(
                metadata.is_dir(),
                "expected a directory at {path:?}, found {metadata:?}"
            );
        }
        Err(error) => panic!("failed to inspect {path:?} for directory assertion: {error}"),
    }
}

/// Assert that `path` is a symlink.
#[track_caller]
pub fn assert_symlink(path: &Path) {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => {
            assert!(
                metadata.file_type().is_symlink(),
                "expected a symlink at {path:?}, found {metadata:?}"
            );
        }
        Err(error) => panic!("failed to inspect {path:?} for symlink assertion: {error}"),
    }
}

/// Read file bytes, panicking with the path and IO error on failure.
#[track_caller]
pub fn read_bytes(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|error| panic!("failed to read bytes at {path:?}: {error}"))
}

/// Read file contents as UTF-8, panicking with the path and IO or decode error.
#[track_caller]
pub fn read_utf8(path: &Path) -> String {
    let bytes = read_bytes(path);
    String::from_utf8(bytes)
        .unwrap_or_else(|error| panic!("file at {path:?} is not valid UTF-8: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn assert_path_missing_accepts_nonexistent_path() {
        let dir = tempfile::tempdir().expect("should create temp dir");
        let missing = dir.path().join("does_not_exist");
        assert_path_missing(&missing);
    }

    #[test]
    fn assert_path_missing_rejects_existing_file() {
        let dir = tempfile::tempdir().expect("should create temp dir");
        let file = dir.path().join("exists.txt");
        std::fs::write(&file, b"data").expect("should write file");
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            assert_path_missing(&file);
        }));
        assert!(result.is_err(), "should panic for an existing file");
    }

    #[test]
    fn assert_regular_file_accepts_file() {
        let dir = tempfile::tempdir().expect("should create temp dir");
        let file = dir.path().join("file.txt");
        std::fs::write(&file, b"data").expect("should write file");
        assert_regular_file(&file);
    }

    #[test]
    fn assert_regular_file_rejects_directory() {
        let dir = tempfile::tempdir().expect("should create temp dir");
        let subdir = dir.path().join("subdir");
        std::fs::create_dir(&subdir).expect("should create directory");
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            assert_regular_file(&subdir);
        }));
        assert!(result.is_err(), "should panic for a directory");
    }

    #[test]
    fn assert_directory_accepts_directory() {
        let dir = tempfile::tempdir().expect("should create temp dir");
        assert_directory(dir.path());
    }

    #[test]
    fn assert_directory_rejects_file() {
        let dir = tempfile::tempdir().expect("should create temp dir");
        let file = dir.path().join("file.txt");
        std::fs::write(&file, b"data").expect("should write file");
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            assert_directory(&file);
        }));
        assert!(result.is_err(), "should panic for a regular file");
    }

    #[test]
    fn read_utf8_returns_file_contents() {
        let dir = tempfile::tempdir().expect("should create temp dir");
        let file = dir.path().join("text.txt");
        std::fs::write(&file, "hello world").expect("should write file");
        assert_eq!(read_utf8(&file), "hello world");
    }

    #[test]
    fn read_bytes_returns_raw_contents() {
        let dir = tempfile::tempdir().expect("should create temp dir");
        let file = dir.path().join("data.bin");
        std::fs::write(&file, b"\x00\x01\x02").expect("should write file");
        assert_eq!(read_bytes(&file), vec![0x00, 0x01, 0x02]);
    }

    #[test]
    #[cfg(unix)]
    fn assert_symlink_detects_dangling_link_as_existing() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().expect("should create temp dir");
        let target = dir.path().join("nonexistent_target");
        let link = dir.path().join("dangling_link");
        symlink(&target, &link).expect("should create symlink");

        // A dangling symlink is NOT missing — it exists as a symlink node.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            assert_path_missing(&link);
        }));
        assert!(
            result.is_err(),
            "dangling symlink must not be treated as missing"
        );

        // The symlink itself is a symlink.
        assert_symlink(&link);
    }

    #[test]
    #[cfg(unix)]
    fn assert_regular_file_rejects_symlink_to_file() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().expect("should create temp dir");
        let target = dir.path().join("target.txt");
        std::fs::write(&target, b"data").expect("should write target");
        let link = dir.path().join("link.txt");
        symlink(&target, &link).expect("should create symlink");

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            assert_regular_file(&link);
        }));
        assert!(
            result.is_err(),
            "symlink must not satisfy assert_regular_file"
        );
    }

    #[test]
    #[cfg(unix)]
    fn assert_directory_rejects_symlink_to_directory() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().expect("should create temp dir");
        let target = dir.path().join("target_dir");
        std::fs::create_dir(&target).expect("should create target dir");
        let link = dir.path().join("link_dir");
        symlink(&target, &link).expect("should create symlink");

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            assert_directory(&link);
        }));
        assert!(result.is_err(), "symlink must not satisfy assert_directory");
    }

    // Ensure the TempDir is not optimized away.
    fn _keep_tempdir_alive(_dir: TempDir) {}
}
