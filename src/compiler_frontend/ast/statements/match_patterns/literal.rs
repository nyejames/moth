//! Literal pattern parsing.
//!
//! WHAT: parses numeric, bool, char, string and negative-numeric literals
//! and dispatches to relational pattern parsing when the lead token is a comparator.
//! WHY: separating literal parsing from relational and choice parsing keeps each
//! submodule focused on one pattern category.

use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression_rpn::ExpressionRpnItem;
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExprId, ExpressionView,
};
use crate::compiler_frontend::ast::numeric_construction::{
    DeferredArmPattern, DeferredExpressionSuffix, DeferredNumericExpression, RetainedPattern,
    deferred_numeric_expression,
};
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::diagnostics::families::{
    choices_control_flow, token_lexical, types_borrows_targets,
};
use crate::compiler_frontend::compiler_messages::{InvalidMatchPatternReason, TypeMismatchContext};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::numeric_text::parse::{materialize_fixed_scalar, materialize_number};
use crate::compiler_frontend::numeric_text::token::NumericLiteralToken;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::TokenTag;
use crate::compiler_frontend::type_coercion::compatibility::is_type_compatible;
use crate::compiler_frontend::type_coercion::parse_context::ExpectedType;
use crate::compiler_frontend::value_mode::ValueMode;
use moth_lexical::numeric::fixed_scalar::FixedScalar;
use moth_lexical::numeric::grammar::{NumericLiteralKind, NumericLiteralSign};
use moth_lexical::numeric::parse::literal_kind_initialises;

use super::diagnostics::reject_deferred_pattern_lead_token;
use super::relational::{parse_pending_relational_pattern, parse_relational_pattern};
use super::types::{MatchPattern, MatchSubject, PatternValue, RelationalPatternOp};
/// Result for the literal pattern family. Authored mismatches remain diagnostics while malformed
/// retained payloads stay on the infrastructure lane.
type LiteralPatternResult<T> = Result<T, ExpressionParseError>;

/// Materialize a `NumericLiteralToken` into an `Expression` with its resolved sign and location.
///
/// WHAT: naturally materialises whole-number patterns as I32 and decimal/exponent patterns
///       as F64.
/// WHY: receiver-specific materialisation runs first, so both pattern arms share this natural
///      literal fallback at the exact language defaults; a fixed-width or `Dec` subject has
///      already materialised at its own exact precision before this fallback is reached.
fn materialize_numeric_literal(
    token: &NumericLiteralToken,
    span: Option<SourceSpan>,
    string_table: &StringTable,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
) -> LiteralPatternResult<ExprId> {
    let scalar = if token.kind == NumericLiteralKind::WholeNumber {
        FixedScalar::I32
    } else {
        FixedScalar::F64
    };
    let value =
        materialize_fixed_scalar(token, token.sign, scalar, string_table).map_err(|reason| {
            ExpressionParseError::from(token_lexical::invalid_number_literal(
                token.source_text,
                reason,
                span,
            ))
        })?;

    Ok(expression_builder
        .fixed_scalar(const_values, value, span, ValueMode::ImmutableOwned)?
        .id())
}

/// Parse one literal or relational pattern against the match subject.
///
/// WHAT: dispatches on whether the subject's exact type is already selected; a pending numeric
///       subject keeps the authored numeric spelling as a retained pattern instead of a typed one.
/// WHY: pattern values materialise at the subject type, so only a known subject can build a typed
///      `MatchPattern` now; a pending subject must hand its authored fragment to body completion.
pub fn parse_non_choice_pattern(
    token_stream: &mut AstCursor,
    subject: MatchSubject,
    string_table: &mut StringTable,
    type_environment: &TypeEnvironment,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
) -> LiteralPatternResult<DeferredArmPattern> {
    match subject {
        MatchSubject::Known(subject_type_id) => {
            let pattern = match token_stream.current_tag() {
                TokenTag::LESS_THAN
                | TokenTag::LESS_THAN_OR_EQUAL
                | TokenTag::GREATER_THAN
                | TokenTag::GREATER_THAN_OR_EQUAL => parse_relational_pattern(
                    token_stream,
                    subject_type_id,
                    string_table,
                    type_environment,
                    expression_builder,
                    const_values,
                )?,

                _ => {
                    let literal = parse_literal_pattern(
                        token_stream,
                        subject_type_id,
                        string_table,
                        type_environment,
                        expression_builder,
                        const_values,
                    )?;
                    MatchPattern::Literal(PatternValue::from_expression(
                        expression_builder.view(literal)?,
                    )?)
                }
            };
            Ok(DeferredArmPattern::Ready(pattern))
        }
        MatchSubject::Pending => parse_pending_non_choice_pattern(
            token_stream,
            string_table,
            expression_builder,
            const_values,
        ),
        MatchSubject::PendingChoice(_) | MatchSubject::PendingOption { .. } => {
            Err(InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                "a pending choice or option subject reached the non-choice pattern parser",
            )
            .into())
        }
    }
}

/// Parse one pattern whose numeric subject type is only selected at the body root.
///
/// WHAT: retains the authored numeric/signed/relational spelling as a `RetainedPattern`, and
///       captures subject-independent literals (bool/char/string) as real typed literals.
/// WHY: a pending subject is a numeric origin, so numeric patterns must materialise exactly once
///      at the type the root selects; literals that need no receiver keep their exact type, and
///      their authored compatibility is checked when the subject exists (never refused early).
fn parse_pending_non_choice_pattern(
    token_stream: &mut AstCursor,
    string_table: &mut StringTable,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
) -> LiteralPatternResult<DeferredArmPattern> {
    if let Some(diagnostic) = reject_deferred_pattern_lead_token(token_stream) {
        return Err(diagnostic.into());
    }

    let pattern = match token_stream.current_tag() {
        TokenTag::LESS_THAN
        | TokenTag::LESS_THAN_OR_EQUAL
        | TokenTag::GREATER_THAN
        | TokenTag::GREATER_THAN_OR_EQUAL => {
            return Ok(DeferredArmPattern::Retained(
                parse_pending_relational_pattern(token_stream, string_table)?,
            ));
        }

        TokenTag::NUMERIC_LITERAL | TokenTag::NEGATIVE => {
            let value = parse_pending_numeric_literal_pattern(token_stream, string_table)?;
            return Ok(DeferredArmPattern::Retained(RetainedPattern {
                relational: None,
                value,
            }));
        }

        TokenTag::BOOL_LITERAL | TokenTag::CHAR_LITERAL | TokenTag::STRING_SLICE_LITERAL => {
            typed_literal_expression(token_stream, string_table, expression_builder, const_values)?
        }

        TokenTag::NONE_LITERAL => {
            return Err(choices_control_flow::invalid_match_pattern(
                InvalidMatchPatternReason::NonePatternRequiresOptionalScrutinee,
                None,
                None,
                Some(token_stream.current_span()),
            )
            .into());
        }

        _ => {
            return Err(choices_control_flow::invalid_match_pattern(
                InvalidMatchPatternReason::LiteralTypeUnsupported,
                None,
                None,
                Some(token_stream.current_span()),
            )
            .into());
        }
    };

    Ok(DeferredArmPattern::Ready(MatchPattern::Literal(
        PatternValue::from_expression(expression_builder.view(pattern)?)?,
    )))
}

/// Parse one authored numeric literal spelling (with an optional leading `-`) into a retained
/// fragment.
///
/// WHAT: consumes the literal token (parser-owned negation folded into its sign) and records
///       exactly one `PendingNumericLiteral` item for body-root materialisation.
/// WHY: a pattern over a pending subject has no selected type yet, so the authored spelling must
///      survive verbatim and materialise exactly once at the subject the root selects.
pub(super) fn parse_pending_numeric_literal_pattern(
    token_stream: &mut AstCursor,
    string_table: &mut StringTable,
) -> LiteralPatternResult<DeferredNumericExpression> {
    let span = Some(token_stream.current_span());
    let sign_override = match token_stream.current_tag() {
        TokenTag::NEGATIVE => {
            token_stream.advance();
            if token_stream.current_tag() != TokenTag::NUMERIC_LITERAL {
                return Err(choices_control_flow::invalid_match_pattern(
                    InvalidMatchPatternReason::NegativeLiteralNotNumeric,
                    None,
                    None,
                    Some(token_stream.current_span()),
                )
                .into());
            }
            Some(NumericLiteralSign::Negative)
        }
        TokenTag::NUMERIC_LITERAL => None,
        // Only a relational operand can reach this arm (the plain literal dispatch checks the lead
        // token first), and a non-numeric operand is an authored mistake, not an invariant break.
        _ => {
            return Err(choices_control_flow::invalid_match_pattern(
                InvalidMatchPatternReason::LiteralTypeUnsupported,
                None,
                None,
                Some(token_stream.current_span()),
            )
            .into());
        }
    };

    let mut token = token_stream
        .current_numeric_literal_in(string_table)?
        .ok_or_else(|| malformed_literal_payload(token_stream))?;
    token.sign = sign_override.unwrap_or(token.sign);
    token_stream.advance();

    Ok(deferred_numeric_expression(
        vec![ExpressionRpnItem::PendingNumericLiteral {
            token,
            span,
            value_mode: ValueMode::ImmutableOwned,
        }],
        ExpectedType::Infer,
        ValueMode::ImmutableOwned,
        span,
        DeferredExpressionSuffix::None,
    ))
}

/// Check one already-built pattern against the subject type selected at the body root.
///
/// WHAT: applies the authored pattern/subject compatibility rule of `parse_literal_pattern` to a
///       pattern value that was built before (or independently of) the subject's exact type.
/// WHY: bool/char/string literal patterns need no receiver, and a retained numeric spelling can
///      fall back to the language default when the selected scalar cannot initialise its kind;
///      both mismatches can only be reported once the subject type exists. Option-presence and
///      choice-variant patterns were validated against the known subject during parse.
pub(crate) fn ensure_pattern_compatible_with_subject(
    pattern: &MatchPattern,
    subject_type_id: TypeId,
    type_environment: &TypeEnvironment,
) -> LiteralPatternResult<()> {
    // A pattern value is typed by the subject's value type: an optional subject compares against
    // its inner payload type, exactly like the parse-time literal and relational checks.
    let subject_value_type_id = type_environment
        .option_inner_type(subject_type_id)
        .unwrap_or(subject_type_id);

    let value = match pattern {
        MatchPattern::Literal(value) => value,
        MatchPattern::Relational { value, .. } => value,
        MatchPattern::OptionNone { .. }
        | MatchPattern::OptionValue { .. }
        | MatchPattern::OptionPresentCapture { .. }
        | MatchPattern::ChoiceVariant { .. } => return Ok(()),
    };
    let value_type = value.type_id;

    if !is_type_compatible(subject_value_type_id, value_type, type_environment) {
        return Err(types_borrows_targets::type_mismatch(
            subject_value_type_id,
            value_type,
            TypeMismatchContext::MatchPattern,
            value.span,
        )
        .into());
    }

    Ok(())
}

fn malformed_literal_payload(token_stream: &AstCursor) -> InfrastructureFailure {
    InfrastructureFailure::internal(
        CompilerStage::AstConstruction,
        format!(
            "literal pattern token {:?} has a malformed payload",
            token_stream.current_tag()
        ),
    )
}

#[allow(
    clippy::too_many_arguments,
    reason = "literal materialisation keeps the cursor, sign, span, type and mutable state separate"
)]
fn materialize_current_numeric_literal(
    token_stream: &AstCursor,
    sign_override: Option<NumericLiteralSign>,
    span: Option<SourceSpan>,
    string_table: &mut StringTable,
    subject_type_id: TypeId,
    type_environment: &TypeEnvironment,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
) -> LiteralPatternResult<ExprId> {
    if let Some(mut token) = token_stream.current_numeric_literal_in(string_table)? {
        let sign = sign_override.unwrap_or(token.sign);
        token.sign = sign;

        if let Some(scale) = type_environment.number_scale(subject_type_id) {
            let value =
                materialize_number(&token, sign, scale, string_table).map_err(|reason| {
                    ExpressionParseError::from(token_lexical::invalid_number_literal(
                        token.source_text,
                        reason,
                        span,
                    ))
                })?;
            return Ok(expression_builder
                .number(
                    const_values,
                    value,
                    subject_type_id,
                    span,
                    ValueMode::ImmutableOwned,
                )?
                .id());
        }

        if let Some(scalar) = type_environment.fixed_scalar(subject_type_id)
            && literal_kind_initialises(token.kind, scalar)
        {
            let value =
                materialize_fixed_scalar(&token, sign, scalar, string_table).map_err(|reason| {
                    ExpressionParseError::from(token_lexical::invalid_number_literal(
                        token.source_text,
                        reason,
                        span,
                    ))
                })?;

            return Ok(expression_builder
                .fixed_scalar(const_values, value, span, ValueMode::ImmutableOwned)?
                .id());
        }

        return materialize_numeric_literal(
            &token,
            span,
            string_table,
            expression_builder,
            const_values,
        );
    }

    Err(malformed_literal_payload(token_stream).into())
}

fn current_bool_literal(token_stream: &AstCursor) -> Option<bool> {
    token_stream.current()?.bool_value()
}

fn current_char_literal(token_stream: &AstCursor) -> Option<char> {
    token_stream.current()?.char_value()
}

fn current_string_literal(
    token_stream: &AstCursor,
    string_table: &mut StringTable,
) -> Result<
    Option<crate::compiler_frontend::symbols::string_interning::StringId>,
    crate::compiler_frontend::compiler_errors::InfrastructureFailure,
> {
    if token_stream.current().is_none() {
        return Ok(None);
    }

    token_stream.current_string_id_in(string_table)
}
/// Materialize one bool/char/string literal token as its exact typed literal.
///
/// WHAT: consumes the current token and builds the typed expression, which needs no subject
///       dependency.
/// WHY: these literals have their exact type from spelling, so the known-subject and
///      pending-subject pattern paths capture the same real typed literal instead of duplicating
///      the token handling.
fn typed_literal_expression(
    token_stream: &mut AstCursor,
    string_table: &mut StringTable,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
) -> LiteralPatternResult<ExprId> {
    let span = Some(token_stream.current_span());
    let expression = match token_stream.current_tag() {
        TokenTag::BOOL_LITERAL => {
            let value = current_bool_literal(token_stream)
                .ok_or_else(|| malformed_literal_payload(token_stream))?;
            expression_builder.bool(const_values, value, span, ValueMode::ImmutableOwned)?
        }
        TokenTag::CHAR_LITERAL => {
            let value = current_char_literal(token_stream)
                .ok_or_else(|| malformed_literal_payload(token_stream))?;
            expression_builder.char(const_values, value, span, ValueMode::ImmutableOwned)?
        }
        TokenTag::STRING_SLICE_LITERAL => {
            let value = current_string_literal(token_stream, string_table)?
                .ok_or_else(|| malformed_literal_payload(token_stream))?;
            expression_builder.string_slice(const_values, value, span, ValueMode::ImmutableOwned)?
        }
        _ => crate::compiler_bug!(
            CompilerStage::AstConstruction,
            "caller checked the bool/char/string literal lead token"
        ),
    };
    token_stream.advance();
    Ok(expression.id())
}

/// Build the completed arm pattern from a retained draft whose value has resolved at the subject.
///
/// WHAT: wraps one materialised pattern value with its authored relational operator, when present.
/// WHY: every completion lane that materialises a retained pattern at the selected subject must
///      produce the same `MatchPattern` shape, so the construction lives with the pattern module.
pub(crate) fn finish_retained_pattern(
    relational: Option<RelationalPatternOp>,
    value: ExpressionView<'_>,
) -> LiteralPatternResult<MatchPattern> {
    let value = PatternValue::from_expression(value)?;
    let span = value.span;
    Ok(match relational {
        Some(op) => MatchPattern::Relational { op, value, span },
        None => MatchPattern::Literal(value),
    })
}

/// Parse one literal pattern and verify it is compatible with the scrutinee type.
///
/// WHAT: materialises the authored literal at the subject's exact type and rejects an authored
///       subject/pattern mismatch at parse time.
/// WHY: catching type mismatches at parse time produces better source-located errors
/// than deferring the check to HIR lowering.
pub(super) fn parse_literal_pattern(
    token_stream: &mut AstCursor,
    subject_type_id: TypeId,
    string_table: &mut StringTable,
    type_environment: &TypeEnvironment,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
) -> LiteralPatternResult<ExprId> {
    if let Some(diagnostic) = reject_deferred_pattern_lead_token(token_stream) {
        return Err(diagnostic.into());
    }

    let pattern = match token_stream.current_tag() {
        // Numeric literal — use the checked typed payload view.
        TokenTag::NUMERIC_LITERAL => {
            let span = Some(token_stream.current_span());
            let sign = None;
            let expression = materialize_current_numeric_literal(
                token_stream,
                sign,
                span,
                string_table,
                subject_type_id,
                type_environment,
                expression_builder,
                const_values,
            )?;
            token_stream.advance();
            expression
        }

        // Bool, char, and string literals share the exact-typed construction with the
        // pending-subject path.
        TokenTag::BOOL_LITERAL | TokenTag::CHAR_LITERAL | TokenTag::STRING_SLICE_LITERAL => {
            typed_literal_expression(token_stream, string_table, expression_builder, const_values)?
        }

        // Negative numeric literal — consume the leading `-` then materialize via the helper.
        TokenTag::NEGATIVE => {
            let minus_sign_span = Some(token_stream.current_span());
            token_stream.advance();
            if token_stream.current_tag() != TokenTag::NUMERIC_LITERAL {
                return Err(choices_control_flow::invalid_match_pattern(
                    InvalidMatchPatternReason::NegativeLiteralNotNumeric,
                    None,
                    None,
                    Some(token_stream.current_span()),
                )
                .into());
            }
            let expression = materialize_current_numeric_literal(
                token_stream,
                Some(NumericLiteralSign::Negative),
                minus_sign_span,
                string_table,
                subject_type_id,
                type_environment,
                expression_builder,
                const_values,
            )?;
            token_stream.advance();
            expression
        }
        // Patterns that are never valid as literal matches.
        TokenTag::NONE_LITERAL => {
            return Err(choices_control_flow::invalid_match_pattern(
                InvalidMatchPatternReason::NonePatternRequiresOptionalScrutinee,
                None,
                None,
                Some(token_stream.current_span()),
            )
            .into());
        }
        _ => {
            return Err(choices_control_flow::invalid_match_pattern(
                InvalidMatchPatternReason::LiteralTypeUnsupported,
                None,
                None,
                Some(token_stream.current_span()),
            )
            .into());
        }
    };

    // -------------------------------
    //  Type-check the literal pattern
    // -------------------------------
    //
    let pattern_view = expression_builder.view(pattern)?;
    if !is_type_compatible(
        subject_type_id,
        pattern_view.value_type_id()?,
        type_environment,
    ) {
        return Err(types_borrows_targets::type_mismatch(
            subject_type_id,
            pattern_view.value_type_id()?,
            TypeMismatchContext::MatchPattern,
            pattern_view.span(),
        )
        .into());
    }

    Ok(pattern)
}

#[cfg(test)]
#[path = "tests/literal_tests.rs"]
mod tests;
