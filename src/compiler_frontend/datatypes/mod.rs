//! Frontend type identity, environment, and resolution support.
//!
//! WHAT: owns resolved semantic type identity for the compiler frontend.
//! WHY: AST retains authored `ParsedTypeRef` syntax and stores resolved semantics as compact
//!      `TypeId`s.
//!
//! Module boundaries:
//! - `ids.rs` — compact type identifiers and canonical keys.
//! - `environment.rs` — `TypeEnvironment` owns all type definitions and interning.
//! - `definitions.rs` — type definition shapes stored in the environment.
//! - `numeric_scalar.rs` — the canonical numeric scalar vocabulary (fixed scalars and exact `Dec`).
//! - `number.rs` — compiler-owned `Dec` coefficients, arithmetic and conversions; shared scale
//!   identity and normalized-text facts live in `moth_lexical`.
//! - `parsed.rs` — retained authored type syntax before resolution (no semantic identity).
//! - `display.rs` — type name rendering through `StringTable`.
//! - `queries.rs` — semantic fact queries over `TypeId + TypeEnvironment`.
//! - `generic_parameters.rs` — parsed generic parameter declarations and scopes.
//! - `generic_bindings.rs` — TypeId-native generic parameter bindings.
//! - `generic_identity_bridge.rs` — HIR/diagnostic bridge keys only.
//! - `numeric_power.rs` — portable binary64 power for compiler-owned constant folding; JS and
//!   Wasm emit matching target-local implementations of the same algorithm.
//!
//! Backend layout, ABI, drop strategy, and runtime representation do NOT belong here.
//! Type compatibility POLICY does NOT belong here (see `type_coercion`).

pub mod definitions;
pub mod display;
pub mod environment;
pub mod generic_bindings;
pub mod generic_identity_bridge;
pub mod generic_parameters;
pub mod ids;
pub(crate) mod number;
pub(crate) mod numeric_operators;
pub(crate) mod numeric_power;
pub(crate) mod numeric_scalar;
pub mod parsed;
pub mod queries;

// Re-exports for convenience.
pub use ids::*;

use crate::compiler_frontend::external_packages::ExternalTypeId;
use crate::compiler_frontend::symbols::path_interner::PathId;
// -----------------------------------------------------------
//  Method Receivers
// -----------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BuiltinScalarReceiver {
    Bool,
    String,
    Char,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ReceiverKey {
    Struct(PathId),
    Choice(PathId),
    External(ExternalTypeId),
    BuiltinScalar(BuiltinScalarReceiver),
}

#[cfg(test)]
mod tests;
