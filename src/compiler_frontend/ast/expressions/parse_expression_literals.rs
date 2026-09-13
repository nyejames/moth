//! Literal expression parsing helpers.
//!
//! WHAT: parses literal tokens (numeric, string, boolean, char, none) and
//!       option-none literal rules.
//! WHY: literal semantics are independent from identifier/call logic and easier
//!      to validate in isolation.
//!
//! The main entry point is [`parse_literal_expression`], which dispatches on
//! token kind. The helper [`none_literal_has_option_equality_context`] detects
//! bare `none` in equality-comparison position so type inference can proceed
//! without an explicit `ExpectedType`.

use super::error::ExpressionParseError;
use super::expression::Expression;
use super::expression_rpn::ExpressionRpnItem;
use super::parse_expression_dispatch::push_expression_operand;
use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_messages::{
    CompileTimeEvaluationErrorReason, CompilerDiagnostic,
};
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::diagnostic_type_spelling;
use crate::compiler_frontend::numeric_text::parse::{materialize_f64, materialize_i32_with_sign};
use crate::compiler_frontend::numeric_text::token::{NumericLiteralKind, NumericLiteralSign};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, TokenKind};
use crate::compiler_frontend::type_coercion::parse_context::ExpectedType;
use crate::compiler_frontend::value_mode::ValueMode;

pub(super) struct LiteralParseState<'a> {
    /// The expected type from surrounding context, if known.
    /// Used by `none` literal resolution to determine the option inner type.
    pub(super) expected_type: &'a ExpectedType,

    /// The current value mode (e.g. compile-time, runtime).
    /// Propagated to every literal expression so downstream stages know how to
    /// evaluate the value.
    pub(super) value_mode: &'a ValueMode,

    /// The RPN expression stack being built.
    /// Literal expressions are pushed as operands for later operator resolution.
    pub(super) expression: &'a mut Vec<ExpressionRpnItem>,

    /// Whether the next numeric literal should be negated.
    /// Set by the parser when a unary `-` precedes a numeric token, bridging
    /// the gap between tokenizer-signed literals and parser-owned negation.
    pub(super) next_number_negative: &'a mut bool,

    /// Whether to allow boundary-catch expressions during operand push.
    /// When true, the operand push may wrap the expression in a boundary check
    /// instead of failing on out-of-range values.
    pub(super) allow_boundary_catch: bool,
}

/// Parse a single literal token and push the resulting AST node.
///
/// WHAT: handles numeric, text, boolean, and option-none literals.
/// WHY: literals are self-contained tokens that do not need identifier resolution or
/// postfix chaining, so they can be validated and emitted in one step.
///
/// Signed numeric tokens are materialized directly from their token metadata. The
/// `next_number_negative` fallback keeps parser-owned unary negation and hand-built test streams
/// on the same signed-i32 boundary policy as tokenizer-signed literals.
pub(super) fn parse_literal_expression(
    token_stream: &mut FileTokens,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    state: &mut LiteralParseState<'_>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<(), ExpressionParseError> {
    match token_stream.current_token_kind().to_owned() {
        TokenKind::NumericLiteral(token) => {
            let span = Some(token_stream.current_span());

            let expression = if token.kind == NumericLiteralKind::WholeNumber {
                let effective_sign =
                    if *state.next_number_negative && token.sign == NumericLiteralSign::Positive {
                        NumericLiteralSign::Negative
                    } else {
                        token.sign
                    };
                *state.next_number_negative = false;

                let value_i32 = materialize_i32_with_sign(&token, effective_sign, string_table)
                    .map_err(|reason| {
                        // Use authored source text so diagnostics report the original literal.
                        CompilerDiagnostic::invalid_number_literal(token.source_text, reason, span)
                    })?;

                Expression::int(value_i32, span, state.value_mode.to_owned())
            } else {
                let mut value = materialize_f64(&token, string_table).map_err(|reason| {
                    // Use authored source text so diagnostics report the original literal.
                    CompilerDiagnostic::invalid_number_literal(token.source_text, reason, span)
                })?;

                if *state.next_number_negative {
                    *state.next_number_negative = false;
                    if token.sign == NumericLiteralSign::Positive {
                        value = -value;
                    }
                }

                Expression::float(value, span, state.value_mode.to_owned())
            };

            token_stream.advance();
            push_expression_operand(
                token_stream,
                context,
                type_interner,
                string_table,
                state.expression,
                state.allow_boundary_catch,
                expression,
                path_fork,
            )?;
            Ok(())
        }

        TokenKind::StringSliceLiteral(string) => {
            let span = Some(token_stream.current_span());
            let string_expr = Expression::string_slice(string, span, state.value_mode.to_owned());
            token_stream.advance();
            push_expression_operand(
                token_stream,
                context,
                type_interner,
                string_table,
                state.expression,
                state.allow_boundary_catch,
                string_expr,
                path_fork,
            )?;
            Ok(())
        }

        TokenKind::BoolLiteral(value) => {
            let span = Some(token_stream.current_span());
            let bool_expr = Expression::bool(value, span, state.value_mode.to_owned());
            token_stream.advance();
            push_expression_operand(
                token_stream,
                context,
                type_interner,
                string_table,
                state.expression,
                state.allow_boundary_catch,
                bool_expr,
                path_fork,
            )?;
            Ok(())
        }

        TokenKind::CharLiteral(value) => {
            let span = Some(token_stream.current_span());
            let char_expr = Expression::char(value, span, state.value_mode.to_owned());
            token_stream.advance();
            push_expression_operand(
                token_stream,
                context,
                type_interner,
                string_table,
                state.expression,
                state.allow_boundary_catch,
                char_expr,
                path_fork,
            )?;
            Ok(())
        }

        TokenKind::NoneLiteral => {
            let span = Some(token_stream.current_span());
            let (inner_type_id, inner_diagnostic_type) =
                if let ExpectedType::Known(expected_type_id) = state.expected_type {
                    let type_environment = type_interner.environment();
                    let Some(inner_type_id) = type_environment.option_inner_type(*expected_type_id)
                    else {
                        return Err(CompilerDiagnostic::compile_time_evaluation_error(
                        CompileTimeEvaluationErrorReason::NoneLiteralRequiresOptionalTypeContext,
                        None,
                        span,
                    )
                    .into());
                    };

                    (
                        inner_type_id,
                        diagnostic_type_spelling(inner_type_id, type_environment),
                    )
                } else if none_literal_has_option_equality_context(token_stream) {
                    // Comparisons like `value is none` and `none is value` infer the option
                    // shape from the opposite operand during evaluation.
                    (type_interner.builtins().none, DataType::Inferred)
                } else {
                    return Err(CompilerDiagnostic::compile_time_evaluation_error(
                        CompileTimeEvaluationErrorReason::NoneLiteralRequiresOptionalTypeContext,
                        None,
                        span,
                    )
                    .into());
                };

            let mut none_expr = Expression::option_none_with_type_id(
                inner_type_id,
                inner_diagnostic_type,
                type_interner.environment_mut_for_derived_types(),
                span,
            );
            none_expr.value_mode = state.value_mode.to_owned();
            token_stream.advance();
            push_expression_operand(
                token_stream,
                context,
                type_interner,
                string_table,
                state.expression,
                state.allow_boundary_catch,
                none_expr,
                path_fork,
            )?;
            Ok(())
        }

        _ => Ok(()),
    }
}

/// Detect whether `none` appears next to an `is` or `not` operator.
///
/// WHAT: allows bare `none` in equality comparisons (`value is none`, `none is value`)
/// even when there is no explicit `ExpectedType` context.
/// WHY: the type of `none` can be inferred from the other operand during later
/// type-checking, so rejecting it here would be overly strict.
fn none_literal_has_option_equality_context(token_stream: &FileTokens) -> bool {
    let follows_equality_operator = token_stream.index > 0
        && matches!(
            token_stream.previous_token(),
            TokenKind::Is | TokenKind::Not
        );
    let leads_equality_operator = matches!(token_stream.peek_next_token(), Some(TokenKind::Is));

    follows_equality_operator || leads_equality_operator
}

#[cfg(test)]
#[path = "tests/parse_expression_literals_tests.rs"]
mod tests;
