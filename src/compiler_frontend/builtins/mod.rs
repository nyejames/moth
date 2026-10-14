//! Frontend-owned builtin language surfaces.
//!
//! WHAT: owns canonical builtin type manifests and language operation identities/signatures.
//! WHY: AST construction and HIR validation consume the same semantic rules, independently of ABI
//! registrations. `member_rules` owns allocation-free collection/map slots and error channels.

use crate::compiler_frontend::external_packages::ExternalFunctionId;

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

    /// External execution targets implement these language-owned collection operations.
    pub(crate) fn external_function_id(self) -> ExternalFunctionId {
        match self {
            Self::Get => ExternalFunctionId::CollectionGet,
            Self::Set => ExternalFunctionId::CollectionSet,
            Self::PushGrowable => ExternalFunctionId::CollectionPushGrowable,
            Self::PushFixed => ExternalFunctionId::CollectionPushFixed,
            Self::Remove => ExternalFunctionId::CollectionRemove,
            Self::Length => ExternalFunctionId::CollectionLength,
        }
    }

    pub(crate) fn from_external_function_id(id: ExternalFunctionId) -> Option<Self> {
        match id {
            ExternalFunctionId::CollectionGet => Some(Self::Get),
            ExternalFunctionId::CollectionSet => Some(Self::Set),
            ExternalFunctionId::CollectionPushGrowable => Some(Self::PushGrowable),
            ExternalFunctionId::CollectionPushFixed => Some(Self::PushFixed),
            ExternalFunctionId::CollectionRemove => Some(Self::Remove),
            ExternalFunctionId::CollectionLength => Some(Self::Length),
            _ => None,
        }
    }
}

pub(crate) mod casts;
pub(crate) mod error_codes;
pub(crate) mod error_type;
pub(crate) mod expression_parsing;
pub mod maps;
pub(crate) mod member_rules;
