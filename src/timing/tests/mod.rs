//! Unit tests for the timing facade.

mod erasure_tests;

#[cfg(feature = "timers")]
mod enabled_tests;

#[cfg(feature = "timers")]
mod summary_tests;
