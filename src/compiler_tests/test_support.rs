//! Shared test utilities for the Moth crate.
//!
//! WHAT: common helpers used across unit and integration tests.
//! WHY: avoids duplicating small utility functions in every test module.

use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::projects::html_project::style_directives::html_project_style_directives;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Receiver;
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime};

/// Includes the HTML project builder directives in the test directives
pub fn frontend_test_style_directives() -> StyleDirectiveRegistry {
    StyleDirectiveRegistry::merged(&html_project_style_directives())
        .expect("HTML style directives should merge with core.")
}

/// Returns a unique uncreated temporary path for test isolation.
///
/// WHAT: joins `std::env::temp_dir()` with a prefix, process ID, nanosecond timestamp, and
///       sequence counter to produce a path that does not exist on disk.
/// WHY: some tests need a path that is guaranteed not to exist (e.g. asserting absence,
///      testing missing-file behavior). The name `unused_temp_path` makes the non-existence
///      contract explicit.
///
/// NOTE: this returns an unmanaged, uncreated path. It does not create the directory and does
///       not clean it up. Callers that need an actual created-and-removed directory should use
///       `tempfile::tempdir()` instead.
///
/// The non-existence contract is proved before returning: PID reuse after an interrupted run
/// can leave a stale node behind, and a caller that assumes absence would then test the wrong
/// condition. Absence is decided by `symlink_metadata`, so a dangling symlink is a node, not
/// absence.
#[track_caller]
pub fn unused_temp_path(prefix: &str) -> PathBuf {
    static TEMP_PATH_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("time should be after unix epoch")
        .as_nanos();
    let sequence = TEMP_PATH_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "moth_{prefix}_{}_{}_{}",
        std::process::id(),
        unique,
        sequence
    ));

    crate::compiler_tests::test_fs::assert_path_missing(&path);

    path
}

/// Run `body`, require that it panics, and assert the panic message contains `expected_fragment`.
///
/// WHAT: captures the unwind payload as text and matches it against the expected fragment.
/// WHY: `catch_unwind(...).is_err()` proves only that *some* panic happened. A helper that
///      panics for an unrelated reason — a fixture IO failure inside the closure, an
///      unwrap on unrelated state — would satisfy a bare `is_err()`. Self-tests for
///      assertion helpers must prove the helper rejected the intended condition.
///
/// The panic hook is deliberately left alone: replacing it is a process-global mutation that
/// would race with panics on other test threads, and the expected-panic backtrace noise is a
/// smaller cost than an unserialized global owner.
#[track_caller]
pub fn assert_panics_with(expected_fragment: &str, body: impl FnOnce() + std::panic::UnwindSafe) {
    let payload = match std::panic::catch_unwind(body) {
        Ok(()) => panic!("expected a panic containing {expected_fragment:?}, but none happened"),
        Err(payload) => payload,
    };

    let message = if let Some(text) = payload.downcast_ref::<&str>() {
        (*text).to_string()
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text.clone()
    } else {
        panic!("panic payload was not a string, so its reason cannot be verified");
    };

    assert!(
        message.contains(expected_fragment),
        "panic message {message:?} does not contain expected fragment {expected_fragment:?}"
    );
}

// ---------------------------------------------------------------------------
//  Worker-thread failure surfacing
// ---------------------------------------------------------------------------
//
// A test that spawns a worker must never lose that worker's panic. Dropping the
// join handle, or joining with `let _ =`, turns a real worker failure into a
// timeout or a missing signal, which reports the wrong cause.

/// Bounded deadlock protection for cross-thread waits with no contractual bound of their own.
///
/// A test that is waiting only because another thread has not got there yet uses this: the
/// deadline stops a stalled worker from hanging the suite and is never the thing being proven.
/// A test whose contract *is* a time bound passes that bound explicitly instead.
pub const WORKER_COMPLETION_DEADLINE: Duration = Duration::from_secs(30);

/// Render a captured panic payload as text.
fn panic_payload_text(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| {
            payload
                .downcast_ref::<&str>()
                .map(|text| (*text).to_owned())
        })
        .unwrap_or_else(|| String::from("<non-string panic payload>"))
}

/// Join a worker thread and print its panic instead of discarding it.
///
/// WHAT: joins the handle and reports any panic payload to stderr.
/// WHY: `let _ = handle.join()` silently discards a worker panic. On failure
///      paths where the test is about to panic for its own reason, the worker's
///      panic must still be visible so the root cause is not hidden.
pub fn surface_thread_panic<T>(name: &str, handle: JoinHandle<T>) {
    if let Err(payload) = handle.join() {
        eprintln!(
            "worker thread '{name}' panicked: {}",
            panic_payload_text(payload.as_ref())
        );
    }
}

/// Wait for a worker's completion signal within `deadline`, then join it.
///
/// WHAT: receives one completion signal, then joins the worker and re-raises its panic as this
///       test's failure.
/// WHY: a worker that panics before signalling would otherwise surface only as a receive
///      timeout, hiding the assertion that actually failed. Joining after a timeout reports the
///      worker's own reason when it has one, and names the timeout when it does not. The caller
///      chooses `deadline` because for some tests the bound is the contract, not just deadlock
///      protection.
#[track_caller]
pub fn await_worker_completion<T>(
    name: &str,
    signal: &Receiver<()>,
    worker: JoinHandle<T>,
    deadline: Duration,
) -> T {
    let signalled = signal.recv_timeout(deadline);

    match worker.join() {
        Ok(value) => {
            signalled.unwrap_or_else(|error| {
                panic!("worker thread '{name}' finished without signalling completion: {error}")
            });
            value
        }
        Err(payload) => panic!(
            "worker thread '{name}' panicked: {}",
            panic_payload_text(payload.as_ref())
        ),
    }
}
