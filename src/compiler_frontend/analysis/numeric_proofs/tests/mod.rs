//! Focused invariant tests for the numeric proof side table.
//!
//! WHAT: proves soundness of the conservative block-local analysis over real HIR relationships:
//!       checked operations, fallible narrowings, invalidation and profile pairing.
//! WHY: these facts let backends drop runtime checks, so every exclusion and invalidation rule
//!       here protects consumer-visible behavior rather than internal wiring.

mod invariants;
