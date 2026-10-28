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
use super::expression_rpn::ExpressionRpnItem;
use super::expression_store::AstExpressionBuilder;
use super::parse_expression_dispatch::push_expression_operand;
use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompileTimeEvaluationErrorReason;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::diagnostics::families::choices_control_flow;
use crate::compiler_frontend::compiler_messages::diagnostics::families::token_lexical;
use crate::compiler_frontend::numeric_text::parse::{materialize_fixed_scalar, materialize_number};
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::TokenTag;
use crate::compiler_frontend::type_coercion::parse_context::ExpectedType;
use crate::compiler_frontend::value_mode::ValueMode;
use moth_lexical::numeric::fixed_scalar::FixedScalar;
use moth_lexical::numeric::grammar::{NumericLiteralKind, NumericLiteralSign};

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
/// Numeric literals without a typed suffix are retained as pending items for
/// destination-aware materialisation in `evaluate_expression`. The
/// `next_number_negative` fallback keeps parser-owned unary negation and hand-built test streams
/// on the same boundary-width policy as tokenizer-signed literals.
#[allow(
    clippy::too_many_arguments,
    reason = "literal parsing keeps the token stream, scope, type interner, expression builder, const store, literal state, string table, and path fork as separate borrows"
)]
pub(super) fn parse_literal_expression(
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    state: &mut LiteralParseState<'_>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<(), ExpressionParseError> {
    match token_stream.current_tag() {
        TokenTag::NUMERIC_LITERAL => {
            let mut token = token_stream
                .current_numeric_literal_in(string_table)?
                .ok_or_else(|| {
                    InfrastructureFailure::internal(
                        CompilerStage::AstConstruction,
                        "numeric literal token had no payload",
                    )
                })?;
            let span = Some(token_stream.current_span());

            // Fold parser-owned unary negation into the token sign now, so the retained
            // literal carries one effective sign exactly like tokenizer-signed literals.
            if *state.next_number_negative && token.sign == NumericLiteralSign::Positive {
                token.sign = NumericLiteralSign::Negative;
            }
            *state.next_number_negative = false;

            if typed_suffix_follows(token_stream, 1) {
                // Member access, fallible handling and option propagation need a typed
                // operand, so this path materialises using any exact receiving Dec identity.
                let number_destination = state
                    .expected_type
                    .literal_destination_type_id()
                    .map(|type_id| {
                        type_interner
                            .environment()
                            .option_inner_type(type_id)
                            .unwrap_or(type_id)
                    })
                    .and_then(|type_id| {
                        type_interner
                            .environment()
                            .number_scale(type_id)
                            .map(|scale| (scale, type_id))
                    });
                let expression = if let Some((scale, type_id)) = number_destination {
                    let value = materialize_number(&token, token.sign, scale, string_table)
                        .map_err(|reason| {
                            token_lexical::invalid_number_literal(token.source_text, reason, span)
                        })?;
                    expression_builder.number(
                        const_values,
                        value,
                        type_id,
                        span,
                        *state.value_mode,
                    )?
                } else if token.kind == NumericLiteralKind::WholeNumber {
                    let value = materialize_fixed_scalar(
                        &token,
                        token.sign,
                        FixedScalar::I32,
                        string_table,
                    )
                    .map_err(|reason| {
                        // Use authored source text so diagnostics report the original literal.
                        token_lexical::invalid_number_literal(token.source_text, reason, span)
                    })?;

                    expression_builder.fixed_scalar(const_values, value, span, *state.value_mode)?
                } else {
                    // The sign was already folded above, so materialising the signed
                    // token rounds once at the default precision; negation is exact.
                    let value = materialize_fixed_scalar(
                        &token,
                        token.sign,
                        FixedScalar::F64,
                        string_table,
                    )
                    .map_err(|reason| {
                        // Use authored source text so diagnostics report the original literal.
                        token_lexical::invalid_number_literal(token.source_text, reason, span)
                    })?;

                    expression_builder.fixed_scalar(const_values, value, span, *state.value_mode)?
                };

                token_stream.advance();
                push_expression_operand(
                    token_stream,
                    context,
                    type_interner,
                    expression_builder,
                    const_values,
                    string_table,
                    state.expression,
                    state.allow_boundary_catch,
                    expression,
                    path_fork,
                )?;
                return Ok(());
            }

            // No typed suffix follows: retain the literal for destination-aware
            // materialisation once `evaluate_expression` knows the receiving type.
            // Adjacency was already rejected by dispatch, and the suffix checks above
            // ruled out postfix/fallible/option handling, so a direct push is exact.
            token_stream.advance();
            state
                .expression
                .push(ExpressionRpnItem::PendingNumericLiteral {
                    token,
                    span,
                    value_mode: *state.value_mode,
                });
            Ok(())
        }

        TokenTag::STRING_SLICE_LITERAL => {
            let string = token_stream
                .current_string_id_in(string_table)?
                .ok_or_else(|| {
                    InfrastructureFailure::internal(
                        CompilerStage::AstConstruction,
                        "string literal token had no payload",
                    )
                })?;
            let span = Some(token_stream.current_span());
            let string_expr =
                expression_builder.string_slice(const_values, string, span, *state.value_mode)?;
            token_stream.advance();
            push_expression_operand(
                token_stream,
                context,
                type_interner,
                expression_builder,
                const_values,
                string_table,
                state.expression,
                state.allow_boundary_catch,
                string_expr,
                path_fork,
            )?;
            Ok(())
        }

        TokenTag::BOOL_LITERAL => {
            let value = token_stream
                .current()
                .and_then(|token| token.bool_value())
                .ok_or_else(|| {
                    InfrastructureFailure::internal(
                        CompilerStage::AstConstruction,
                        "boolean literal token had no payload",
                    )
                })?;
            let span = Some(token_stream.current_span());
            let bool_expr =
                expression_builder.bool(const_values, value, span, *state.value_mode)?;
            token_stream.advance();
            push_expression_operand(
                token_stream,
                context,
                type_interner,
                expression_builder,
                const_values,
                string_table,
                state.expression,
                state.allow_boundary_catch,
                bool_expr,
                path_fork,
            )?;
            Ok(())
        }

        TokenTag::CHAR_LITERAL => {
            let value = token_stream
                .current()
                .and_then(|token| token.char_value())
                .ok_or_else(|| {
                    InfrastructureFailure::internal(
                        CompilerStage::AstConstruction,
                        "char literal token had no payload",
                    )
                })?;
            let span = Some(token_stream.current_span());
            let char_expr =
                expression_builder.char(const_values, value, span, *state.value_mode)?;
            token_stream.advance();
            push_expression_operand(
                token_stream,
                context,
                type_interner,
                expression_builder,
                const_values,
                string_table,
                state.expression,
                state.allow_boundary_catch,
                char_expr,
                path_fork,
            )?;
            Ok(())
        }

        TokenTag::NONE_LITERAL => {
            let span = Some(token_stream.current_span());
            let none_value_mode = *state.value_mode;
            let inner_type_id = if let ExpectedType::Known(expected_type_id) = state.expected_type {
                let type_environment = type_interner.environment();
                let Some(inner_type_id) = type_environment.option_inner_type(*expected_type_id)
                else {
                    return Err(choices_control_flow::compile_time_evaluation_error(
                        CompileTimeEvaluationErrorReason::NoneLiteralRequiresOptionalTypeContext,
                        None,
                        span,
                    )
                    .into());
                };
                inner_type_id
            } else if none_literal_has_option_equality_context(token_stream) {
                // Comparisons like `value is none` and `none is value` infer the
                // option shape from the opposite operand during evaluation.
                type_interner.builtins().none
            } else {
                return Err(choices_control_flow::compile_time_evaluation_error(
                    CompileTimeEvaluationErrorReason::NoneLiteralRequiresOptionalTypeContext,
                    None,
                    span,
                )
                .into());
            };

            let none_expr = expression_builder.option_none_with_type_id(
                const_values,
                inner_type_id,
                type_interner.environment_mut_for_derived_types(),
                span,
            )?;
            let mut completed = expression_builder
                .view_owned(none_expr)?
                .to_completed_input(expression_builder)?;
            completed.value_mode = none_value_mode;
            token_stream.advance();
            push_expression_operand(
                token_stream,
                context,
                type_interner,
                expression_builder,
                const_values,
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

/// True when the token at `offset` (skipping newlines) needs a typed operand before it.
///
/// WHAT: peeks the unconsumed stream for member access (`.`), typed propagation (`!`,
///       a `name!` suffix) and option propagation (`?`). A literal passes offset 1
///       (the token after itself); a closed group passes offset 0.
/// WHY: those suffixes resolve through `push_expression_operand`, which needs a typed
///      `Expression`, so a literal or group followed by one completes before outer context.
///      Whole-expression `catch` completes after evaluation, so its unresolved arithmetic
///      retains receiving and local-peer materialisation.
pub(super) fn typed_suffix_follows(token_stream: &AstCursor, mut offset: usize) -> bool {
    while token_stream
        .token_ref_at_offset(offset)
        .is_some_and(|token| token.tag() == TokenTag::NEWLINE)
    {
        offset = offset.saturating_add(1);
    }

    let Some(next) = token_stream.token_ref_at_offset(offset) else {
        return false;
    };

    match next.tag() {
        TokenTag::DOT | TokenTag::BANG | TokenTag::QUESTION_MARK => true,

        TokenTag::SYMBOL => token_stream
            .token_ref_at_offset(offset.saturating_add(1))
            .is_some_and(|following| following.tag() == TokenTag::BANG),

        _ => false,
    }
}

/// Detect whether `none` appears next to an `is` or `not` operator.
///
/// WHAT: allows bare `none` in equality comparisons (`value is none`, `none is value`)
/// even when there is no explicit `ExpectedType` context.
/// WHY: the type of `none` can be inferred from the other operand during later
/// type-checking, so rejecting it here would be overly strict.
fn none_literal_has_option_equality_context(token_stream: &AstCursor) -> bool {
    let follows_equality_operator = token_stream
        .previous()
        .is_some_and(|token| matches!(token.tag(), TokenTag::IS | TokenTag::NOT));
    let leads_equality_operator = token_stream.peek_next_tag() == Some(TokenTag::IS);
    follows_equality_operator || leads_equality_operator
}

#[cfg(test)]
#[path = "tests/parse_expression_literals_tests.rs"]
mod tests;
