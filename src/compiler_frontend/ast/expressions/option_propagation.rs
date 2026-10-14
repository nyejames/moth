//! Postfix option propagation parsing.
//!
//! WHAT: validates `expr?` after the ordinary expression receiver has been
//! parsed and typed.
//! WHY: option propagation returns from the enclosing function on `none`, so
//! parsing must check both the operand's optional shape and the current
//! function return contract before HIR lowers the control-flow edge.

use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, OwnedExpr,
};
use crate::compiler_frontend::ast::module_ast::scope_context::EnclosingCallableOwner;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidFallibleHandlingReason,
};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::tokenizer::tokens::TokenTag;
use crate::compiler_frontend::type_coercion::compatibility::is_type_compatible;

/// Consumes one authored postfix `?` and applies it to the value already parsed.
///
/// WHAT: consumes the operator, rejects an authored `catch` attached to it, and then applies the
///       shared option-propagation finisher.
/// WHY: the eager path owns the operator tokens, while the semantic decision belongs to the shared
///      finisher so a value that only completes at the body root applies the same rule.
pub(crate) fn parse_option_propagation_suffix_for_expression(
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    expression: OwnedExpr,
) -> Result<OwnedExpr, ExpressionParseError> {
    let propagation_span = Some(token_stream.current_postfix_operator_span());
    token_stream.advance();

    let propagated = finish_option_propagation(
        expression,
        propagation_span,
        context,
        type_interner,
        expression_builder,
    )?;

    if token_stream.current_tag() == TokenTag::CATCH {
        return Err(CompilerDiagnostic::invalid_fallible_handling(
            InvalidFallibleHandlingReason::OptionPropagationCatchConflict,
            Some(token_stream.current_postfix_operator_span()),
        )
        .into());
    }

    Ok(propagated)
}

/// Applies one authored option propagation to a completed operand.
///
/// WHAT: checks the operand's optional shape and the enclosing function's optional return
///       contract, then builds the option propagation node at the operand's own span.
/// WHY: the eager parse path and a value that only completes at the body root must apply the
///      authored `?` by one rule — same reasons in the same order — so resumption shares this
///      finisher instead of restating the option checks.
pub(crate) fn finish_option_propagation(
    value: OwnedExpr,
    propagation_span: Option<SourceSpan>,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
) -> Result<OwnedExpr, ExpressionParseError> {
    let view = expression_builder.view_owned(value)?;
    let propagation_span = propagation_span.or(view.span());
    let value_span = view.span();
    let value_type_id = match view.success_types() {
        [type_id] => Some(*type_id),
        _ => None,
    };

    let type_environment = type_interner.environment();
    let Some(inner_type_id) =
        value_type_id.and_then(|type_id| type_environment.option_inner_type(type_id))
    else {
        return Err(CompilerDiagnostic::invalid_fallible_handling(
            InvalidFallibleHandlingReason::NotOptionExpression,
            propagation_span,
        )
        .into());
    };

    let Some(contract) = context
        .enclosing_callable
        .as_deref()
        .filter(|contract| contract.owner == EnclosingCallableOwner::Function)
    else {
        return Err(CompilerDiagnostic::invalid_fallible_handling(
            InvalidFallibleHandlingReason::FunctionHasNoOptionalReturn,
            propagation_span,
        )
        .into());
    };
    if contract.authored_success_slot_count > 1 {
        return Err(CompilerDiagnostic::invalid_fallible_handling(
            InvalidFallibleHandlingReason::FunctionHasMultipleSuccessReturns {
                count: contract.authored_success_slot_count,
            },
            propagation_span,
        )
        .into());
    }
    let [function_return_type_id] = contract.success_types.as_slice() else {
        return Err(CompilerDiagnostic::invalid_fallible_handling(
            InvalidFallibleHandlingReason::FunctionHasNoOptionalReturn,
            propagation_span,
        )
        .into());
    };
    let function_return_type_id = *function_return_type_id;

    if !type_environment.is_option(function_return_type_id) {
        return Err(CompilerDiagnostic::invalid_fallible_handling(
            InvalidFallibleHandlingReason::FunctionHasNoOptionalReturn,
            propagation_span,
        )
        .into());
    }

    if !is_type_compatible(
        function_return_type_id,
        value_type_id.expect("optional operand was checked above"),
        type_environment,
    ) {
        return Err(CompilerDiagnostic::invalid_fallible_handling(
            InvalidFallibleHandlingReason::OptionPropagationReturnTypeMismatch,
            propagation_span,
        )
        .into());
    }

    Ok(expression_builder.option_propagation_with_type_id(value, inner_type_id, value_span)?)
}
