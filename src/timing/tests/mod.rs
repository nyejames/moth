//! Unit tests for the timing facade.

mod erasure_tests;

#[cfg(feature = "timers")]
mod enabled_tests;

#[cfg(feature = "timers")]
mod schema_tests;

#[cfg(feature = "timers")]
mod summary_tests;

/// Serialize timing-collector tests against each other and against every
/// frontend counter/build test.
///
/// The collector is one process-global scope shared with the frontend counter
/// stores. This test suite delegates to the single facade-owned lock so timing
/// tests and frontend instrumentation tests can never run concurrently and
/// interleave sessions.
#[cfg(feature = "timers")]
pub(crate) fn lock_timing_tests() -> std::sync::MutexGuard<'static, ()> {
    crate::timing::lock_instrumentation_tests()
}
