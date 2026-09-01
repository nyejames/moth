//! Semantic fact queries over `TypeId + TypeEnvironment`.
//!
//! WHAT: answers questions about types without embedding policy.
//! WHY: keeps `TypeEnvironment` fact-oriented; compatibility policy stays
//!      in `type_coercion`.
//!
//! NOTE: fact queries live as methods on `TypeEnvironment`. This module
//!       only exports the `TypeKind` classification enum used by match
//!       exhaustiveness and coercion logic.

/// High-level classification of a type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeKind {
    Builtin,
    Struct,
    Choice,
    Constructed,
    Function,
    External,
    GenericParameter,
    GenericInstance,
    /// Compile-time-only anonymous const-record marker; never a runtime type.
    AnonymousConstRecordMarker,
}
