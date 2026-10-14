//! Receiver-call dispatch orchestration.
//!
//! WHAT: dispatches a receiver method call through the ordered lookup sequence:
//!       source method → generic-bound method.
//! WHY: keeping the dispatch order explicit in one file makes the precedence
//!      between overlapping method sources obvious and easy to audit.

use super::{MemberStepContext, ReceiverAccessMode};
use crate::compiler_frontend::ast::ast_nodes::AstNode;
use crate::compiler_frontend::ast::const_values::ConstValueStore;
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression_rpn::ExpressionRpnItem;
use crate::compiler_frontend::ast::expressions::expression_store::AstExpressionBuilder;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;

mod generic_bound_methods;
mod shared;
mod source_methods;

pub(super) use shared::{finish_pending_receiver_call_expression, recover_pending_producer};
pub(in crate::compiler_frontend::ast::field_access) use source_methods::lookup_receiver_method;
pub(crate) use source_methods::{
    DeferredCallReceiver, DeferredReceiverCall, DeferredReceiverCallee, ReservedGenericRequest,
    complete_deferred_receiver_call,
};

/// True when a receiver type is still an unresolved generic parameter.
///
/// WHAT: the same generic-parameter classification bound dispatch uses, exposed so retained
///       postfix steps keep their generic-method deferral instead of searching numeric types.
/// WHY: a bound method on an unresolved generic receiver belongs to the later generic-consumer
///      work, and no numeric type may be searched for one with a matching method.
pub(super) fn generic_parameter_receiver_type(
    receiver_type_id: TypeId,
    type_environment: &TypeEnvironment,
) -> bool {
    generic_bound_methods::generic_parameter_id_for_receiver_type(
        receiver_type_id,
        type_environment,
    )
    .is_some()
}

/// One dispatched receiver method call: built, or retained for body-root numeric completion.
///
/// WHAT: separates the completed call node from the pending operand item a retained call occupies.
/// WHY: the chain driver needs the exact operand shape; a retained call has no value or type yet
///      and must never be represented by a fabricated node.
pub(super) enum ParsedReceiverMethodCall {
    Value(AstNode),
    RetainedOperand(ExpressionRpnItem),
}

/// Dispatches one receiver method call through the ordered lookup sequence.
///
/// `allow_boundary_catch` is the chain entry's boundary permission, carried to a retained call so
/// its authored `catch` is decided at the site that still owns the token stream.
#[allow(
    clippy::too_many_arguments,
    reason = "receiver method call parsing keeps the token stream, member step, catch policy, type interner, expression builder, const store, string table, and path fork as separate borrows"
)]
pub(super) fn parse_receiver_method_call_typed(
    token_stream: &mut AstCursor<'_>,
    member_step_context: MemberStepContext<'_>,
    allow_boundary_catch: bool,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<Option<ParsedReceiverMethodCall>, ExpressionParseError> {
    let MemberStepContext {
        receiver_node,
        receiver_type_id,
        member_name,
        member_span,
        receiver_access_mode,
        authored_marker_span,
        scope_context,
    } = member_step_context;

    // 1. Visible declared source receiver method.
    if let Some(method_entry) = source_methods::lookup_receiver_method(
        scope_context,
        receiver_type_id,
        member_name,
        type_interner.environment(),
    ) {
        let node = source_methods::parse_source_receiver_method_target_call_typed(
            source_methods::SourceReceiverMethodCallInput {
                token_stream,
                receiver_node,
                member_name,
                member_span,
                receiver_access_mode,
                authored_marker_span,
                allow_boundary_catch,
                scope_context,
                source_method: source_methods::SourceReceiverMethodTarget::Declared(method_entry),
                type_interner,
                expression_builder,
                const_values,
                string_table,
                path_fork,
            },
        )?;
        return Ok(Some(node));
    }

    // 2. Static generic-bound receiver method.
    if let Some(generic_bound_method) = generic_bound_methods::lookup_generic_bound_receiver_method(
        scope_context,
        receiver_node,
        expression_builder,
        receiver_type_id,
        member_name,
        member_span,
        type_interner.environment(),
        string_table,
        path_fork,
    )? {
        let node = source_methods::parse_source_receiver_method_target_call_typed(
            source_methods::SourceReceiverMethodCallInput {
                token_stream,
                receiver_node,
                member_name,
                member_span,
                receiver_access_mode,
                authored_marker_span,
                allow_boundary_catch,
                scope_context,
                source_method: source_methods::SourceReceiverMethodTarget::TraitSurface(
                    generic_bound_method,
                ),
                type_interner,
                expression_builder,
                const_values,
                string_table,
                path_fork,
            },
        )?;
        return Ok(Some(node));
    }

    Ok(None)
}
