//! Receiver-call dispatch orchestration.
//!
//! WHAT: dispatches a receiver method call through the ordered lookup sequence:
//!       source method → generic-bound method.
//! WHY: keeping the dispatch order explicit in one file makes the precedence
//!      between overlapping method sources obvious and easy to audit.

use super::{MemberStepContext, ReceiverAccessMode};
use crate::compiler_frontend::ast::ast_nodes::AstNode;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::FileTokens;

mod generic_bound_methods;
mod shared;
mod source_methods;

pub(super) fn parse_receiver_method_call_typed(
    token_stream: &mut FileTokens,
    member_step_context: MemberStepContext<'_>,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<Option<AstNode>, ExpressionParseError> {
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
                scope_context,
                source_method: source_methods::SourceReceiverMethodTarget::Declared(method_entry),
                type_interner,
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
                scope_context,
                source_method: source_methods::SourceReceiverMethodTarget::TraitSurface(
                    generic_bound_method,
                ),
                type_interner,
                string_table,
                path_fork,
            },
        )?;
        return Ok(Some(node));
    }

    Ok(None)
}
