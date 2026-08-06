//! Unit tests for the timing facade.

mod erasure_tests;

#[cfg(feature = "timers")]
mod enabled_tests;

#[cfg(feature = "timers")]
mod summary_tests;

/// Serialize timing-collector tests against each other.
///
/// The collector is one process-global scope. This lock is owned by the timing
/// test suite so timing tests never borrow the frontend counter-test lock.
#[cfg(feature = "timers")]
pub(crate) fn lock_timing_tests() -> std::sync::MutexGuard<'static, ()> {
    use std::sync::Mutex;
    static TIMING_TEST_LOCK: Mutex<()> = Mutex::new(());
    TIMING_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
