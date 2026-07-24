//! Alpha cast integer range policy shared by Rust-side folding and JS runtime.
//!
//! WHAT: defines the signed 32-bit integer range that explicit builtin casts to `Int` are
//!      allowed to materialize for the current Alpha runtime target.
//! WHY: Moth Alpha `Int` is a signed `i32`. Keeping one source of truth for the cast
//!      range in the compiler lets AST folding and JS helper emission agree instead of
//!      duplicating magic values in two stages.
//! NOTE: this is an explicit cast materialization policy, not a general numeric-limit
//!      utility. Other `i64` uses for indices, byte counts, or external ABI metadata are
//!      intentionally not part of this policy.

/// Maximum integer value that explicit builtin casts to `Int` may produce.
///
/// Equal to `i32::MAX`.
pub(crate) const I32_MAX: i32 = i32::MAX;

/// Minimum integer value that explicit builtin casts to `Int` may produce.
///
/// Equal to `i32::MIN`.
pub(crate) const I32_MIN: i32 = i32::MIN;
