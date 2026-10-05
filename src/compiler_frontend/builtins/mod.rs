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
/// receiver's canonical collection shape picks exactly one of them. Fixed push has a recoverable
/// `Error!` path at capacity; growable push has no recoverable `Error!` path, though allocation
/// exhaustion may still trap or abort.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollectionBuiltinOp {
    Get,
    Set,
    PushGrowable,
    PushFixed,
    Remove,
    Length,
}

/// Collection access classification is independent of the declared result slots.
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
}

pub(crate) mod casts;
pub(crate) mod error_codes;
pub(crate) mod error_type;
pub(crate) mod expression_parsing;
pub mod maps;
