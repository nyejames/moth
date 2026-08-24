//! Typed value-production parsing helpers.
//!
//! WHAT: parses one or more expressions after `then`, validating arity and applying
//! contextual coercion against the active value-production target.
//! WHY: this logic was previously catch-specific; generalising it lets value `if`,
//! match, and catch share one arity/coercion path.

use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::expressions::parse_expression::{
    create_expression_with_trailing_newline_policy, create_multiple_expressions,
};
use crate::compiler_frontend::ast::expressions::parse_expression_input::{
    ExpressionParseInput, ExpressionParseResources, ExpressionTrailingPolicy,
};
use crate::compiler_frontend::ast::statements::value_production::types::{
    ActiveValueProductionTarget, ValueReceiverKind,
};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticPayload, InvalidFallibleHandlingReason, InvalidReturnShapeReason,
    TypeMismatchContext,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, TokenKind};
use crate::compiler_frontend::type_coercion::contextual::coerce_expression_to_explicit_type_boundary;
use crate::compiler_frontend::type_coercion::parse_context::{CastTargetContext, ExpectedType};
use crate::compiler_frontend::value_mode::ValueMode;

/// Input bundle for `parse_produced_values_typed`.
///
/// WHAT: avoids a long parameter list by grouping everything the parser needs.
/// WHY: the caller already has all of these values on hand; a struct keeps call sites
/// readable.
pub struct ProducedValuesParseInput<'a, 'b> {
    pub token_stream: &'a mut FileTokens,
    pub context: &'a ScopeContext,
    pub type_interner: &'a mut AstTypeInterner<'b>,
    pub target: &'a ActiveValueProductionTarget,
    pub label: &'a str,
    pub string_table: &'a mut StringTable,
}

/// Returns whether the current token proves that no produced value was authored.
pub(crate) fn is_missing_produced_value_boundary(kind: &TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::Else
            | TokenKind::Eof
            | TokenKind::End
            | TokenKind::Comma
            | TokenKind::CloseParenthesis
            | TokenKind::CloseCurly
    )
}

/// Parses a list of expressions that must match the target's expected type/arity.
///
/// WHAT: reads one or more expressions after `then`, validates that the count matches
/// `target.result_type_ids`, and applies contextual coercion per position.
/// WHY: every value-producing site (value `if`, match and catch) needs identical
/// arity and coercion validation.
pub fn parse_produced_values_typed<'a, 'b>(
    input: ProducedValuesParseInput<'a, 'b>,
) -> Result<Vec<Expression>, ExpressionParseError> {
    let ProducedValuesParseInput {
        token_stream,
        context,
        type_interner,
        target,
        label,
        string_table,
    } = input;

    if target.result_type_ids.is_empty() {
        if target.receiver_kind == ValueReceiverKind::Declaration {
            return parse_single_inferred_declaration_value(
                token_stream,
                context,
                type_interner,
                string_table,
            );
        }

        if let Some(arity) = target.expected_arity {
            return parse_fixed_arity_inferred_values(
                token_stream,
                context,
                type_interner,
                arity,
                string_table,
                &target.known_slot_types,
                target.receiver_kind,
            );
        }

        return Err(CompilerDiagnostic::invalid_fallible_handling(
            InvalidFallibleHandlingReason::FallbackValuesForErrorOnlyResult,
            token_stream.current_location(),
        )
        .into());
    }

    let expression_context = context.new_child_expression(target.result_type_ids.clone());
    let produced_values = match create_multiple_expressions(
        token_stream,
        &expression_context,
        type_interner,
        label,
        false,
        string_table,
    ) {
        Ok(values) => values,

        // If create_multiple_expressions reports TooFewReturnValues but the current token
        // is actually the start of another expression, the value list has more tokens
        // than the expected shape can consume. Report the clearer arity error at
        // the first extra value.
        Err(ExpressionParseError::Diagnostic(diagnostic)) => {
            // Detect the "too many values disguised as too few" case:
            // the parser stopped early because the extra value overflowed
            // the expected shape, but the next token is clearly an expression.
            if let DiagnosticPayload::InvalidReturnShape {
                reason: InvalidReturnShapeReason::TooFewReturnValues { expected_count, .. },
            } = &diagnostic.payload
                && is_expression_start_token(token_stream.current_token_kind())
            {
                return Err(CompilerDiagnostic::invalid_return_shape(
                    InvalidReturnShapeReason::TooManyReturnValues {
                        expected_count: *expected_count,
                    },
                    token_stream.current_location(),
                )
                .into());
            }
            return Err(ExpressionParseError::Diagnostic(diagnostic));
        }

        Err(err) => return Err(err),
    };

    // Explicit too-many check: comma after the expected count means an extra value follows.
    if token_stream.current_token_kind() == &TokenKind::Comma {
        return Err(CompilerDiagnostic::invalid_return_shape(
            InvalidReturnShapeReason::TooManyReturnValues {
                expected_count: target.result_type_ids.len(),
            },
            token_stream.current_location(),
        )
        .into());
    }

    // Defensive invariant: create_multiple_expressions should always return the expected count
    // when it succeeds, but we verify locally so callers cannot silently miss arity errors.
    if produced_values.len() != target.result_type_ids.len() {
        return Err(CompilerDiagnostic::invalid_return_shape(
            InvalidReturnShapeReason::TooFewReturnValues {
                expected_count: target.result_type_ids.len(),
                provided_count: produced_values.len(),
            },
            token_stream.current_location(),
        )
        .into());
    }

    validate_and_coerce_produced_values(
        produced_values,
        &target.result_type_ids,
        type_interner.environment(),
        context,
        target.receiver_kind,
    )
}

// WHAT: parses a single inferred produced value when the receiver has no known type.
// WHY: inferred declaration initializers such as `value = if condition: then 1 ...`
//      only learn their type from the authored `then` values. Known and mixed
//      multi-slot receivers keep per-slot expected types instead.
fn parse_single_inferred_declaration_value(
    token_stream: &mut FileTokens,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
) -> Result<Vec<Expression>, ExpressionParseError> {
    let expression_context = context.new_child_expression(vec![]);
    let mut expected_type = ExpectedType::Infer;
    let mut none_cast_target = CastTargetContext::None;
    let input = ExpressionParseInput::new(
        ExpressionParseResources {
            token_stream,
            scope_context: &expression_context,
            type_interner,
            expected_type: &mut expected_type,
            cast_target_context: &mut none_cast_target,
            value_mode: &ValueMode::ImmutableOwned,
            string_table,
        },
        ExpressionTrailingPolicy {
            consume_closing_parenthesis: false,
            skip_trailing_newlines: false,
            allow_boundary_catch: true,
            allow_expected_result_evidence: false,
        },
    );
    let expression = create_expression_with_trailing_newline_policy(input)?;

    if token_stream.current_token_kind() == &TokenKind::Comma {
        return Err(CompilerDiagnostic::invalid_return_shape(
            InvalidReturnShapeReason::TooManyReturnValues { expected_count: 1 },
            token_stream.current_location(),
        )
        .into());
    }

    Ok(vec![expression])
}

/// Parses a fixed number of expressions after `then` for mixed or fully inferred slots.
///
/// WHAT: reads exactly `arity` expressions. Known slots use their receiving type so
/// forms such as `none` still parse; unknown slots stay inferred.
/// WHY: mixed multi-bind must keep known-slot context at parse time even though
/// unknown siblings are inferred later from every producing path.
pub(crate) fn parse_fixed_arity_inferred_values(
    token_stream: &mut FileTokens,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    arity: usize,
    string_table: &mut StringTable,
    slot_expected_types: &[Option<TypeId>],
    receiver_kind: ValueReceiverKind,
) -> Result<Vec<Expression>, ExpressionParseError> {
    let expression_context = context.new_child_expression(vec![]);
    let mut values = Vec::with_capacity(arity);

    for index in 0..arity {
        let known_slot_type = slot_expected_types.get(index).copied().flatten();
        let mut expected_type = known_slot_type
            .map(ExpectedType::Known)
            .unwrap_or(ExpectedType::Infer);
        let mut none_cast_target = CastTargetContext::None;
        let input = ExpressionParseInput::new(
            ExpressionParseResources {
                token_stream,
                scope_context: &expression_context,
                type_interner,
                expected_type: &mut expected_type,
                cast_target_context: &mut none_cast_target,
                value_mode: &ValueMode::ImmutableOwned,
                string_table,
            },
            ExpressionTrailingPolicy {
                consume_closing_parenthesis: false,
                skip_trailing_newlines: false,
                allow_boundary_catch: true,
                allow_expected_result_evidence: false,
            },
        );
        let mut expression = create_expression_with_trailing_newline_policy(input)?;
        if let Some(expected_type_id) = known_slot_type {
            expression = coerce_expression_to_explicit_type_boundary(
                expression,
                expected_type_id,
                type_interner.environment(),
                context,
                mismatch_context_for_receiver(receiver_kind),
            )?;
        }
        values.push(expression);

        if index + 1 < arity {
            if token_stream.current_token_kind() != &TokenKind::Comma {
                return Err(CompilerDiagnostic::invalid_return_shape(
                    InvalidReturnShapeReason::TooFewReturnValues {
                        expected_count: arity,
                        provided_count: values.len(),
                    },
                    token_stream.current_location(),
                )
                .into());
            }
            token_stream.advance();
        }
    }

    if token_stream.current_token_kind() == &TokenKind::Comma {
        return Err(CompilerDiagnostic::invalid_return_shape(
            InvalidReturnShapeReason::TooManyReturnValues {
                expected_count: arity,
            },
            token_stream.current_location(),
        )
        .into());
    }

    Ok(values)
}

fn validate_and_coerce_produced_values(
    produced_values: Vec<Expression>,
    expected_type_ids: &[TypeId],
    type_environment: &TypeEnvironment,
    context: &ScopeContext,
    receiver_kind: ValueReceiverKind,
) -> Result<Vec<Expression>, ExpressionParseError> {
    let mut checked_values = Vec::with_capacity(produced_values.len());
    let mismatch_context = mismatch_context_for_receiver(receiver_kind);

    // Validate each produced value against the corresponding expected type,
    // coercing when compatible or reporting a mismatch when not.
    for (produced_value, expected_type_id) in
        produced_values.into_iter().zip(expected_type_ids.iter())
    {
        checked_values.push(coerce_expression_to_explicit_type_boundary(
            produced_value,
            *expected_type_id,
            type_environment,
            context,
            mismatch_context,
        )?);
    }

    Ok(checked_values)
}

fn mismatch_context_for_receiver(receiver_kind: ValueReceiverKind) -> TypeMismatchContext {
    match receiver_kind {
        ValueReceiverKind::Declaration => TypeMismatchContext::Declaration,
        ValueReceiverKind::Return => TypeMismatchContext::ReturnValue,
        ValueReceiverKind::Assignment
        | ValueReceiverKind::MultiBind
        | ValueReceiverKind::CatchHandler => TypeMismatchContext::General,
    }
}

// WHAT: identifies tokens that can begin a new expression.
// WHY: when create_multiple_expressions reports TooFewReturnValues, the token at which it
// stopped may actually be the start of an extra produced value (user forgot a comma).
// Distinguishing expression starts from statement keywords or terminators lets us report
// TooManyReturnValues instead of the misleading TooFewReturnValues.
fn is_expression_start_token(token: &TokenKind) -> bool {
    matches!(
        token,
        TokenKind::Symbol(_)
            | TokenKind::NumericLiteral(_)
            | TokenKind::StringSliceLiteral(_)
            | TokenKind::RawStringLiteral(_)
            | TokenKind::CharLiteral(_)
            | TokenKind::BoolLiteral(_)
            | TokenKind::NoneLiteral
            | TokenKind::OpenCurly
            | TokenKind::OpenParenthesis
            | TokenKind::TemplateHead
            | TokenKind::DatatypeInt
            | TokenKind::DatatypeFloat
            | TokenKind::DatatypeBool
            | TokenKind::DatatypeString
            | TokenKind::DatatypeChar
            | TokenKind::Subtract
            | TokenKind::Copy
            | TokenKind::Mutable
    )
}
