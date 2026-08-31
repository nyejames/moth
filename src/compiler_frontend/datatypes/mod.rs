//! Frontend type identity, environment, and resolution support.
//!
//! WHAT: owns resolved semantic type identity for the compiler frontend.
//! WHY: AST should carry compact `TypeId`s instead of cloned `DataType` payloads.
//!
//! Module boundaries:
//! - `datatype.rs` — the `DataType` enum and its intrinsic methods.
//! - `ids.rs` — compact type identifiers and canonical keys.
//! - `environment.rs` — `TypeEnvironment` owns all type definitions and interning.
//! - `definitions.rs` — type definition shapes stored in the environment.
//! - `parsed.rs` — parsed type syntax before resolution (no semantic identity).
//! - `display.rs` — type name rendering through `StringTable`.
//! - `queries.rs` — semantic fact queries over `TypeId + TypeEnvironment`.
//! - `generic_parameters.rs` — parsed generic parameter declarations and scopes.
//! - `generic_bindings.rs` — TypeId-native generic parameter bindings.
//! - `generic_identity_bridge.rs` — HIR/diagnostic bridge keys only.
//!
//! Backend layout, ABI, drop strategy, and runtime representation do NOT belong here.
//! Type compatibility POLICY does NOT belong here (see `type_coercion`).

pub mod datatype;
pub mod definitions;
pub mod display;
pub mod environment;
pub mod fallible_carrier;
pub mod generic_bindings;
pub mod generic_identity_bridge;
pub mod generic_parameters;
pub mod ids;
pub mod parsed;
pub mod queries;

// Re-exports for convenience.
pub use datatype::DataType;
pub(crate) use datatype::diagnostic_type_spelling;

pub use environment::TypeEnvironment;
pub use ids::*;

use crate::compiler_frontend::external_packages::ExternalTypeId;
use crate::compiler_frontend::symbols::interned_path::InternedPath;

// -----------------------------------------------------------
//  Method Receivers
// -----------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BuiltinScalarReceiver {
    Int,
    Float,
    Bool,
    String,
    Char,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ReceiverKey {
    Struct(InternedPath),
    Choice(InternedPath),
    External(ExternalTypeId),
    BuiltinScalar(BuiltinScalarReceiver),
}

#[cfg(test)]
mod tests;
