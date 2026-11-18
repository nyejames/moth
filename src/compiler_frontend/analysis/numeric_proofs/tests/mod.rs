//! Focused invariant tests for the numeric proof side table.
//!
//! WHAT: proves soundness of the conservative block analysis over real HIR relationships:
//!       checked operations, fallible narrowings, invalidation and the sole-success-edge seed that
//!       carries one proven result interval into a block whose only incoming edge is that Invoke.
//! WHY: these facts let backends drop runtime checks, so every exclusion and invalidation rule
//!       here protects consumer-visible behavior rather than internal wiring.

mod invariants;
