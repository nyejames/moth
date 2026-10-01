//! Builtin cast surface: target classification, evidence table, and policy helpers.
//!
//! WHAT: groups the cast-related modules that together own the compiler-supported
//!      builtin cast surface. The module map is small and re-export based:
//!      - `targets`  — classification enums, policy ids, and target/resolution helpers
//!      - `evidence` — single static table of initial builtin evidence rows
//!      - `policies` — pure policy functions that operate on `BuiltinCastLiteral`
//!      - `traits`   — central names and registration helpers for core cast traits
//!      - `resolution` — AST cast resolver wiring
//! WHY: the cast owner needs one clearly mapped location that parser, AST, and
//!      folding stages can all reach for cast answers without depending on
//!      parser orchestration modules or backend code.
//! NOTE: `Int` range policy lives on `IntWidth::{min_value, max_value, contains}`;
//!      there is no separate cast-owned numeric-limit module.

pub(crate) mod evidence;
pub(crate) mod policies;
pub(crate) mod resolution;
pub(crate) mod targets;
pub(crate) mod traits;

#[cfg(test)]
#[path = "tests/mod.rs"]
mod tests;

pub(crate) use policies::{BuiltinCastLiteral, apply_builtin_cast_policy};
