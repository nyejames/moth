//! Test-only forced-exhaustion gate for the compact path-identity domain.
//!
//! WHAT: one thread-local switch that makes every path-interning call reject a NEW node while
//!       lookups of already-interned nodes keep succeeding.
//! WHY:  a project that exhausts the four-byte path domain cannot be reproduced by allocating
//!       billions of nodes. Tests raise the gate, drive one authored allocation, and observe the
//!       typed capacity lane; dropping the guard restores the live table. The gate never affects
//!       release builds.
//!
//! The switch is thread-local so concurrent test threads stay isolated: the Rust test harness
//! runs every `#[test]` on its own thread, so a guard raised in one test can never reject
//! another test's allocation.

use std::cell::Cell;

thread_local! {
    static FORCED_PATH_EXHAUSTION: Cell<bool> = const { Cell::new(false) };
}

/// Return whether test code on the current thread has forced the path domain into exhaustion.
pub(crate) fn forced_exhaustion() -> bool {
    FORCED_PATH_EXHAUSTION.with(Cell::get)
}

/// RAII switch that rejects every new path node on this thread until it drops.
///
/// Lookup reuse stays intact while the gate is raised: already-interned `(parent, component)`
/// children keep returning their existing identity, so tests exercise the allocation boundary
/// exactly once rather than breaking every read.
pub(crate) struct ForcedExhaustionGuard {
    active: bool,
}

impl ForcedExhaustionGuard {
    pub(crate) fn new() -> Self {
        FORCED_PATH_EXHAUSTION.with(|gate| gate.set(true));
        Self { active: true }
    }
}

impl Drop for ForcedExhaustionGuard {
    fn drop(&mut self) {
        if self.active {
            FORCED_PATH_EXHAUSTION.with(|gate| gate.set(false));
        }
    }
}
