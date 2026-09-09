//! Postfix/member parsing coordinator.
//!
//! WHAT: drives chained postfix parsing and dispatches each member step to focused handlers.
//! WHY: field access, receiver methods, and compiler-owned builtin members evolve independently,
//! so the chain driver should stay thin while policy lives in dedicated modules.

mod builtin_call_args;
mod collection_builtin;
mod field_member;
mod map_builtin;
mod parse_chain;
mod receiver_access;
mod receiver_calls;

pub use parse_chain::parse_field_access;
pub(crate) use parse_chain::{
    parse_field_access_expression_with_receiver_access, parse_postfix_chain_expression,
    reference_expression_from_declaration,
};

use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::ast_nodes::AstNode;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::string_interning::StringId;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;

// --------------------------
//  Types
// --------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReceiverAccessMode {
    Shared,
    Mutable,
}

/// Access context for entering a postfix chain.
///
/// WHAT: pairs the receiver access mode with the optional authored `~` marker location that
///       opened the chain.
/// WHY: explicit mutable receiver access (`~name.method(...)`) carries the marker location so
///      authored-marker diagnostics can point at `~`; shared entries carry no marker. Grouping
///      the two keeps the chain entry signature small and threads the marker through the
///      postfix-chain context in one value.
#[derive(Clone)]
pub(crate) struct PostfixChainAccess {
    pub(crate) mode: ReceiverAccessMode,
    pub(crate) authored_marker_location: Option<SourceLocation>,
}

impl PostfixChainAccess {
    /// Shared access with no authored mutable marker.
    pub(crate) fn shared() -> Self {
        PostfixChainAccess {
            mode: ReceiverAccessMode::Shared,
            authored_marker_location: None,
        }
    }

    /// Explicit mutable receiver access opened by an authored `~` marker.
    pub(crate) fn mutable_marker(marker_location: SourceLocation) -> Self {
        PostfixChainAccess {
            mode: ReceiverAccessMode::Mutable,
            authored_marker_location: Some(marker_location),
        }
    }
}

/// Shared parse state for one postfix member step.
#[derive(Clone)]
pub(super) struct MemberStepContext<'a> {
    pub receiver_node: &'a AstNode,
    pub receiver_type_id: TypeId,
    pub member_name: StringId,
    pub member_location: SourceLocation,
    pub member_span: Option<SourceSpan>,
    pub receiver_access_mode: ReceiverAccessMode,
    /// The authored `~` marker location when the chain was entered through explicit mutable
    /// receiver access. Authored-marker receiver diagnostics point here instead of the method
    /// boundary.
    pub(crate) authored_marker_location: Option<SourceLocation>,
    pub scope_context: &'a ScopeContext,
}
