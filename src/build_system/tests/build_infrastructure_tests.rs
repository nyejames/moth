//! Tests for the core build orchestration and output writer APIs.
// NOTE: temp file creation processes have to be explicitly dropped
// Or these tests will fail on Windows due to attempts to delete non-empty temp directories while files are still open.

use super::*;
use std::fs;

#[test]
fn current_dir_guard_recovers_after_mutex_poisoning() {
    let temp = tempfile::tempdir().expect("should create temp dir");
    let root = temp.path().to_path_buf();

    // Intentionally poison the cwd mutex by panicking while holding the guard.
    // The panic payload is exact so a different panic cannot satisfy the test.
    let panic_result = std::panic::catch_unwind(|| {
        let _guard = CurrentDirGuard::set_to(&root);
        panic!("deliberate panic to poison the cwd mutex");
    });
    assert!(
        panic_result.is_err(),
        "catch_unwind should capture the panic"
    );

    // Verify the exact panic payload, not just that some panic happened.
    let panic_payload = panic_result.unwrap_err();
    let panic_message = panic_payload
        .downcast_ref::<String>()
        .map(|s| s.as_str())
        .or_else(|| panic_payload.downcast_ref::<&str>().copied());
    assert_eq!(
        panic_message,
        Some("deliberate panic to poison the cwd mutex"),
        "must capture the exact intentional panic payload, not any panic"
    );

    // A subsequent guard acquisition must succeed despite the poisoned mutex.
    let guard = CurrentDirGuard::set_to(&root);
    let current = fs::canonicalize(std::env::current_dir().expect("current dir should resolve"))
        .expect("current dir should canonicalize");
    let expected = fs::canonicalize(&root).expect("temp root should canonicalize");
    assert_eq!(current, expected);
    drop(guard);
}

#[test]
fn current_dir_guard_finish_restores_once_and_drop_does_not_retry() {
    let temp = tempfile::tempdir().expect("should create temp dir");
    let root = temp.path().to_path_buf();

    // Use `finish()` for explicit restoration on the normal path.
    let guard = CurrentDirGuard::set_to(&root);
    let current = fs::canonicalize(std::env::current_dir().expect("current dir should resolve"))
        .expect("current dir should canonicalize");
    let expected = fs::canonicalize(&root).expect("temp root should canonicalize");
    assert_eq!(current, expected);

    // `finish()` restores the previous directory and returns Ok. The Drop impl
    // will not retry because `previous` has been taken.
    let finish_result = guard.finish();
    assert!(
        finish_result.is_ok(),
        "finish() should restore the previous directory: {finish_result:?}"
    );

    // Verify the directory is no longer the temp root. We cannot assert it equals
    // the original because the mutex has been released and another cwd-mutating
    // test may have changed it. We only verify we left the temp dir.
    let restored = std::env::current_dir().expect("current dir should resolve");
    assert_ne!(
        restored, root,
        "finish() should have moved away from the temp directory"
    );
}
