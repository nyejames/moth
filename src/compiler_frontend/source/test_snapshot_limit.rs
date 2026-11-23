//! Test-only source-snapshot size gate.
//!
//! WHAT: one thread-local byte limit consulted before a snapshot is retained.
//! WHY: the real limit is `u32::MAX` bytes, so an oversized donor cannot be constructed in a
//!      test without allocating multiple gigabytes. Tests raise a small limit on their own thread
//!      and drive the same capacity branch; dropping the guard restores the authored limit.
//!
//! The switch is thread-local because the Rust test harness runs every `#[test]` on its own
//! thread, so a guard raised in one test never rejects another test's sources.

use std::cell::Cell;

thread_local! {
    static FORCED_SNAPSHOT_LIMIT: Cell<Option<usize>> = const { Cell::new(None) };
}

/// Return the test thread's snapshot limit override, if any.
pub(super) fn forced_snapshot_limit() -> Option<usize> {
    FORCED_SNAPSHOT_LIMIT.with(Cell::get)
}

/// RAII switch that applies a smaller snapshot limit on the current test thread.
pub(crate) struct ForcedSourceSnapshotLimitGuard;

impl ForcedSourceSnapshotLimitGuard {
    pub(crate) fn new(limit: usize) -> Self {
        FORCED_SNAPSHOT_LIMIT.with(|gate| gate.set(Some(limit)));
        Self
    }
}

impl Drop for ForcedSourceSnapshotLimitGuard {
    fn drop(&mut self) {
        FORCED_SNAPSHOT_LIMIT.with(|gate| gate.set(None));
    }
}
