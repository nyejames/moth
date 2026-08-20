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

#[test]
fn current_dir_guard_finish_returns_error_when_restore_fails() {
    let temp = tempfile::tempdir().expect("should create temp dir");
    let root = temp.path().to_path_buf();

    // The injected restore override must actually restore to the supplied
    // previous directory before returning the synthetic failure. This keeps the
    // process CWD safe while still proving that `finish()` propagates the exact
    // restore error. Because the override performs the real restoration, calling
    // `finish()` directly is safe and lets us assert the returned error.
    let guard = CurrentDirGuard::set_to(&root).with_restore_override(Box::new(|path| {
        // Actually restore to the previous directory and prove it succeeded.
        std::env::set_current_dir(path).expect("override should restore CWD to previous");
        // Now return an error to simulate a restore failure for testing.
        Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "injected restore failure for testing",
        ))
    }));

    // `finish()` consumes the guard and calls the restore override, which
    // performed the real restoration and then returned the injected error.
    // Assert that exact returned error.
    let finish_result = guard.finish();
    assert!(
        finish_result.is_err(),
        "finish() should return an error when restore fails"
    );
    let error = finish_result.expect_err("finish() should return the injected restore error");
    assert!(
        error.to_string().contains("injected restore failure"),
        "error should contain the injected message: {error}"
    );
}

#[test]
fn current_dir_guard_drop_during_unwinding_reports_restore_failure_without_double_panic() {
    let temp = tempfile::tempdir().expect("should create temp dir");
    let root = temp.path().to_path_buf();

    // The override receives the `previous` path (captured by `set_to` inside the
    // mutex). It explicitly restores CWD to that path, verifies the restoration
    // succeeded, then returns an error to simulate a restore failure. This
    // tests the error-reporting path during unwinding without leaving CWD at a
    // wrong location or discarding the real restoration result.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _guard = CurrentDirGuard::set_to(&root).with_restore_override(Box::new(|path| {
            // Explicitly restore to the previous directory and prove it succeeded.
            std::env::set_current_dir(path).expect("override should restore CWD to previous");
            // Now return an error to simulate a restore failure for testing.
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "injected unwind restore failure",
            ))
        }));

        panic!("deliberate test panic to trigger unwinding");
    }));

    // The original panic should be preserved, not masked by a restore-failure
    // panic. Assert the exact panic payload so an unrelated panic cannot pass.
    assert!(result.is_err(), "the original panic should be preserved");
    let panic_payload = result.unwrap_err();
    let panic_message = panic_payload
        .downcast_ref::<String>()
        .map(|s| s.as_str())
        .or_else(|| panic_payload.downcast_ref::<&str>().copied());
    assert_eq!(
        panic_message,
        Some("deliberate test panic to trigger unwinding"),
        "must capture the exact intentional panic payload, not any panic"
    );
}
