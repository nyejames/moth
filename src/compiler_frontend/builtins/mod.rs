//! Frontend-owned builtin language surfaces.
//!
//! WHAT: groups canonical builtin type manifests used by AST/HIR construction.
//! WHY: keeps language-owned builtin declarations out of parser orchestration modules.

/// Compiler-owned collection builtin operation kinds.
///
/// WHAT: identifies collection operations that are language builtins, not user receiver methods.
/// WHY: parser and lowering stages need one explicit operation surface for collection semantics.
///
/// Growable and fixed push are distinct identities sharing one source member (`push`): the
/// receiver's canonical collection shape picks exactly one of them, and their fallibility
/// differs (fixed push can exceed capacity; growable push cannot fail).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollectionBuiltinOp {
    Get,
    Set,
    PushGrowable,
    PushFixed,
    Remove,
    Length,
}

/// Each collection operation is characterized by static attributes:
///
/// - `requires_mutable_receiver`: borrow-checker mutability classification
/// - `is_fallible`: whether the operation may produce a runtime error
impl CollectionBuiltinOp {
    /// Whether the receiver must be accessed mutably.
    pub fn requires_mutable_receiver(self) -> bool {
        // Operations that modify collection contents.
        matches!(
            self,
            CollectionBuiltinOp::Set
                | CollectionBuiltinOp::PushGrowable
                | CollectionBuiltinOp::PushFixed
                | CollectionBuiltinOp::Remove
        )
    }

    /// Whether the operation is fallible and must be handled.
    pub fn is_fallible(self) -> bool {
        matches!(
            self,
            CollectionBuiltinOp::Get
                | CollectionBuiltinOp::Set
                | CollectionBuiltinOp::PushFixed
                | CollectionBuiltinOp::Remove
        )
    }
}

pub(crate) mod casts;
pub(crate) mod error_codes;
pub(crate) mod error_type;
pub(crate) mod expression_parsing;
pub mod maps;
