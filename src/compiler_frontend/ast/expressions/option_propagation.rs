//! Postfix option propagation parsing.
//!
//! WHAT: validates `expr?` after the ordinary expression receiver has been
//! parsed and typed.
//! WHY: option propagation returns from the enclosing function on `none`, so
//! parsing must check both the operand's optional shape and the current
//! function return contract before HIR lowers the control-flow edge.

use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidFallibleHandlingReason,
};
use crate::compiler_frontend::datatypes::diagnostic_type_spelling;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, TokenKind};
use crate::compiler_frontend::type_coercion::compatibility::is_type_compatible;

pub(crate) fn parse_option_propagation_suffix_for_expression(
    token_stream: &mut FileTokens,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression: Expression,
) -> Result<Expression, ExpressionParseError> {
    let propagation_location = token_stream.current_postfix_operator_location();
    token_stream.advance();

    let type_environment = type_interner.environment();
    let Some(inner_type_id) = type_environment.option_inner_type(expression.type_id) else {
        return Err(CompilerDiagnostic::invalid_fallible_handling(
            InvalidFallibleHandlingReason::NotOptionExpression,
            propagation_location,
        )
        .into());
    };

    let [function_return_type_id] = context.current_function_return_type_ids.as_slice() else {
        return Err(CompilerDiagnostic::invalid_fallible_handling(
            InvalidFallibleHandlingReason::FunctionHasNoOptionalReturn,
            propagation_location,
        )
        .into());
    };

    if !type_environment.is_option(*function_return_type_id) {
        return Err(CompilerDiagnostic::invalid_fallible_handling(
            InvalidFallibleHandlingReason::FunctionHasNoOptionalReturn,
            propagation_location,
        )
        .into());
    }

    if !is_type_compatible(
        *function_return_type_id,
        expression.type_id,
        type_environment,
    ) {
        return Err(CompilerDiagnostic::invalid_fallible_handling(
            InvalidFallibleHandlingReason::OptionPropagationReturnTypeMismatch,
            propagation_location,
        )
        .into());
    }

    if token_stream.current_token_kind() == &TokenKind::Catch {
        return Err(CompilerDiagnostic::invalid_fallible_handling(
            InvalidFallibleHandlingReason::OptionPropagationCatchConflict,
            token_stream.current_location(),
        )
        .into());
    }

    let diagnostic_type = diagnostic_type_spelling(inner_type_id, type_environment);
    Ok(Expression::option_propagation_with_type_id(
        expression,
        inner_type_id,
        diagnostic_type,
        propagation_location,
    ))
}
