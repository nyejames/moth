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
mod retained_postfix;

#[cfg(test)]
#[path = "tests/postfix_owner_tests.rs"]
mod postfix_owner_tests;

pub(crate) use builtin_call_args::concrete_shape_type;
pub(crate) use field_member::project_explicit_const_record_field;
pub use parse_chain::parse_field_access;
pub(crate) use parse_chain::{
    ParsedPostfixExpression, parse_field_access_expression_with_receiver_access,
    parse_postfix_chain_expression, reference_expression_from_declaration,
};
pub(crate) use receiver_calls::{
    DeferredCallReceiver, DeferredReceiverCall, complete_deferred_receiver_call,
};
pub(crate) use retained_postfix::{
    GenericSuccessSlots, RetainedMemberTarget, RetainedOperandContinuation, RetainedOperandInput,
    RetainedOperandResult, RetainedPostfixStep, RetainedTerminalCall,
    complete_retained_operand_continuation, grouped_evaluation_expected,
    retain_operand_call_propagation, retain_operand_handling_suffix,
    retain_pending_operand_member_access, retained_member_target,
    retained_operand_numeric_origin_shape, retained_operand_result, retained_operand_result_shape,
    retained_terminal_call,
};

use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::ast_nodes::AstNode;
use crate::compiler_frontend::ast::expressions::expression_rpn::ExpressionRpnItem;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::string_interning::StringId;

// --------------------------
//  Types
// --------------------------

/// The outcome of one compiler-owned builtin member step.
///
/// WHAT: either the completed builtin node, or the operand item a retained chain keeps when the
///       member's own arguments still hold numeric origins.
/// WHY: a builtin member's operation, receiver shape and result slots are already resolved when its
///      arguments turn out to need completion, so the step resumes through the same constructor
///      instead of a second member grammar; this mirrors the receiver-method outcome shape.
pub(crate) enum ParsedBuiltinMemberCall {
    Completed(AstNode),
    RetainedOperand(ExpressionRpnItem),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReceiverAccessMode {
    Shared,
    Mutable,
}

/// Access context for entering a postfix chain.
///
/// WHAT: pairs the receiver access mode with the optional authored `~` marker span that opened
///       the chain.
/// WHY: explicit mutable receiver access (`~name.method(...)`) carries the marker span so
///      authored-marker diagnostics can point at `~`; shared entries carry no marker. Grouping the
///      two keeps the chain entry signature small and threads the marker through the postfix-chain
///      context in one value.
#[derive(Clone)]
pub(crate) struct PostfixChainAccess {
    pub(crate) mode: ReceiverAccessMode,
    pub(crate) authored_marker_span: Option<SourceSpan>,
}

impl PostfixChainAccess {
    /// Shared access with no authored mutable marker.
    pub(crate) fn shared() -> Self {
        PostfixChainAccess {
            mode: ReceiverAccessMode::Shared,
            authored_marker_span: None,
        }
    }

    /// Explicit mutable receiver access opened by an authored `~` marker.
    pub(crate) fn mutable_marker(marker_span: Option<SourceSpan>) -> Self {
        PostfixChainAccess {
            mode: ReceiverAccessMode::Mutable,
            authored_marker_span: marker_span,
        }
    }
}

/// Shared parse state for one postfix member step.
#[derive(Clone)]
pub(super) struct MemberStepContext<'a> {
    pub receiver_node: &'a AstNode,
    pub receiver_type_id: TypeId,
    pub member_name: StringId,
    pub member_span: Option<SourceSpan>,
    pub receiver_access_mode: ReceiverAccessMode,
    /// The authored `~` marker span when the chain was entered through explicit mutable receiver
    /// access. Authored-marker receiver diagnostics point here instead of the method boundary.
    pub(crate) authored_marker_span: Option<SourceSpan>,
    pub scope_context: &'a ScopeContext,
}
