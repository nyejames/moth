//! Shared test utilities for the Moth crate.
//!
//! WHAT: common helpers used across unit and integration tests.
//! WHY: avoids duplicating small utility functions in every test module.

use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::projects::html_project::style_directives::html_project_style_directives;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

/// Includes the HTML project builder directives in the test directives
pub fn frontend_test_style_directives() -> StyleDirectiveRegistry {
    StyleDirectiveRegistry::merged(&html_project_style_directives())
        .expect("HTML style directives should merge with core.")
}

/// Returns a unique temporary directory path for test isolation.
///
/// WHAT: joins `std::env::temp_dir()` with a prefix, process ID, nanosecond timestamp, and
///       sequence counter to produce an unused path.
/// WHY: prevents test collisions when multiple tests run concurrently or in sequence.
///
/// NOTE: this returns an unmanaged path. It does not create the directory and does not clean it
///       up. Callers that need an actual created-and-removed directory should use
///       `tempfile::tempdir()` instead.
pub fn temp_dir(prefix: &str) -> PathBuf {
    static TEMP_DIR_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("time should be after unix epoch")
        .as_nanos();
    let sequence = TEMP_DIR_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "moth_{prefix}_{}_{}_{}",
        std::process::id(),
        unique,
        sequence
    ))
}
