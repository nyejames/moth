//! Central declaration binding-mode representation.
//!
//! WHAT: distinguishes the user-facing binding mode for declarations, independent of the later
//! semantic `ValueMode` used for expression and access classification.
//!
//! WHY: binding mode is a syntactic/semantic declaration property (`=`, `~=`, `#=`) that determines
//! both runtime mutability and compile-time foldability. Keeping it separate from `ValueMode`
//! avoids conflating parse-time binding syntax with AST-level access classification.
//!
//! MUST NOT: leak into borrow-checker or backend lowering directly. Lowering stages consume
//! `ValueMode`, not `BindingMode`.

use crate::compiler_frontend::value_mode::ValueMode;

/// The binding mode of a declaration or binding target.
///
/// WHAT: captures which of the mutually exclusive binding markers the user wrote, or the default
/// when no marker is present.
///
/// WHY: a single enum replaces the old `mutable_marker: bool` while keeping the supported runtime
/// and compile-time declaration modes explicit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BindingMode {
    /// No marker: runtime immutable binding (`name = value`, `name Type = value`).
    #[default]
    ImmutableRuntime,

    /// `~` marker: runtime mutable binding (`name ~= value`, `name ~Type = value`).
    MutableRuntime,

    /// `#` marker: compile-time constant binding (`name #= value`, `name #Type = value`).
    CompileTimeConstant,
}

impl BindingMode {
    /// Returns `true` for runtime bindings that own mutation-capable storage.
    pub fn is_mutable(&self) -> bool {
        matches!(self, Self::MutableRuntime)
    }

    /// Returns `true` for compile-time constant bindings.
    pub fn is_compile_time(&self) -> bool {
        matches!(self, Self::CompileTimeConstant)
    }

    /// Maps this binding mode to the AST-level value classification.
    ///
    /// WHY: `ValueMode` tracks access/ownership semantics during expression lowering, while
    /// `BindingMode` tracks the original user syntax. This function is the boundary between them.
    pub fn value_mode(&self) -> ValueMode {
        match self {
            Self::ImmutableRuntime | Self::CompileTimeConstant => ValueMode::ImmutableOwned,
            Self::MutableRuntime => ValueMode::MutableOwned,
        }
    }
}
