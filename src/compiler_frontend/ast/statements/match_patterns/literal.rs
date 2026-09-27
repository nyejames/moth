//! Literal pattern parsing.
//!
//! WHAT: parses int, float, bool, char, string, and negative-numeric literals
//! and dispatches to relational pattern parsing when the lead token is a comparator.
//! WHY: separating literal parsing from relational and choice parsing keeps each
//! submodule focused on one pattern category.

use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidMatchPatternReason, TypeMismatchContext,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::fixed_scalar::FixedScalar;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::datatypes::numeric_profile::NumericProfile;
use crate::compiler_frontend::numeric_text::parse::{
    literal_kind_initialises, materialize_fixed_scalar, materialize_float, materialize_int,
};
use crate::compiler_frontend::numeric_text::token::{
    NumericLiteralKind, NumericLiteralSign, NumericLiteralToken,
};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::TokenTag;
use crate::compiler_frontend::type_coercion::compatibility::is_type_compatible;
use crate::compiler_frontend::value_mode::ValueMode;

use super::diagnostics::reject_deferred_pattern_lead_token;
use super::relational::parse_relational_pattern;
use super::types::MatchPattern;
/// Result for the literal pattern family. Authored mismatches remain diagnostics while malformed
/// retained payloads stay on the infrastructure lane.
type LiteralPatternResult<T> = Result<T, ExpressionParseError>;

/// Materialize a `NumericLiteralToken` into an `Expression` with an explicit sign and location.
///
/// WHAT: naturally materialises whole-number patterns as `Int` and decimal/exponent patterns as
///       profile `Float`; Float subjects route whole-number tokens through profile Float directly.
/// WHY: positive and negative pattern arms share the same natural materialisation logic, while
///      `parse_literal_pattern` applies receiver-specific materialisation before falling back here.
fn materialize_numeric_literal(
    token: &NumericLiteralToken,
    sign: NumericLiteralSign,
    span: Option<SourceSpan>,
    string_table: &StringTable,
    numeric_profile: NumericProfile,
) -> LiteralPatternResult<Expression> {
    if token.kind == NumericLiteralKind::WholeNumber {
        let value = materialize_int(token, sign, numeric_profile.int_width, string_table).map_err(
            |reason| {
                ExpressionParseError::from(CompilerDiagnostic::invalid_number_literal(
                    token.source_text,
                    reason,
                    span,
                ))
            },
        )?;

        Ok(Expression::int(value, span, ValueMode::ImmutableOwned))
    } else {
        let value = materialize_float(token, numeric_profile.float_precision, string_table)
            .map_err(|reason| {
                ExpressionParseError::from(CompilerDiagnostic::invalid_number_literal(
                    token.source_text,
                    reason,
                    span,
                ))
            })?;

        // Negate the float when the sign is negative; the normalised text is unsigned
        // and always carries the positive magnitude. Negation is exact, so the
        // materialized precision is preserved.
        let float_value = match sign {
            NumericLiteralSign::Negative => -value,
            NumericLiteralSign::Positive => value,
        };

        Ok(Expression::float(
            float_value,
            span,
            ValueMode::ImmutableOwned,
        ))
    }
}

pub fn parse_non_choice_pattern(
    token_stream: &mut AstCursor,
    subject_type_id: TypeId,
    numeric_profile: NumericProfile,
    string_table: &mut StringTable,
    type_environment: &TypeEnvironment,
) -> LiteralPatternResult<MatchPattern> {
    match token_stream.current_tag() {
        TokenTag::LESS_THAN
        | TokenTag::LESS_THAN_OR_EQUAL
        | TokenTag::GREATER_THAN
        | TokenTag::GREATER_THAN_OR_EQUAL => parse_relational_pattern(
            token_stream,
            subject_type_id,
            numeric_profile,
            string_table,
            type_environment,
        ),

        _ => {
            let literal = parse_literal_pattern(
                token_stream,
                subject_type_id,
                numeric_profile,
                string_table,
                type_environment,
            )?;
            Ok(MatchPattern::Literal(literal))
        }
    }
}

fn malformed_literal_payload(token_stream: &AstCursor) -> CompilerError {
    CompilerError::compiler_error(format!(
        "literal pattern token {:?} has a malformed payload",
        token_stream.current_tag()
    ))
}

fn materialize_current_numeric_literal(
    token_stream: &AstCursor,
    sign_override: Option<NumericLiteralSign>,
    span: Option<SourceSpan>,
    string_table: &mut StringTable,
    numeric_profile: NumericProfile,
    fixed_scalar: Option<FixedScalar>,
    float_subject: bool,
) -> LiteralPatternResult<Expression> {
    if let Some(token) = token_stream.current_numeric_literal_in(string_table)? {
        let sign = sign_override.unwrap_or(token.sign);

        if float_subject && token.kind == NumericLiteralKind::WholeNumber {
            let mut signed_token = token;
            signed_token.sign = sign;
            let value =
                materialize_float(&signed_token, numeric_profile.float_precision, string_table)
                    .map_err(|reason| {
                        ExpressionParseError::from(CompilerDiagnostic::invalid_number_literal(
                            signed_token.source_text,
                            reason,
                            span,
                        ))
                    })?;

            return Ok(Expression::float(value, span, ValueMode::ImmutableOwned));
        }

        if let Some(scalar) = fixed_scalar
            && literal_kind_initialises(token.kind, scalar)
        {
            let value =
                materialize_fixed_scalar(&token, sign, scalar, string_table).map_err(|reason| {
                    ExpressionParseError::from(CompilerDiagnostic::invalid_number_literal(
                        token.source_text,
                        reason,
                        span,
                    ))
                })?;

            return Ok(Expression::fixed_scalar(
                value,
                span,
                ValueMode::ImmutableOwned,
            ));
        }

        return materialize_numeric_literal(&token, sign, span, string_table, numeric_profile);
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
    crate::compiler_frontend::compiler_errors::CompilerError,
> {
    if token_stream.current().is_none() {
        return Ok(None);
    }

    token_stream.current_string_id_in(string_table)
}
/// verifies the pattern type is compatible with the scrutinee type.
/// WHY: catching type mismatches at parse time produces better source-located errors
/// than deferring the check to HIR lowering.
pub(super) fn parse_literal_pattern(
    token_stream: &mut AstCursor,
    subject_type_id: TypeId,
    numeric_profile: NumericProfile,
    string_table: &mut StringTable,
    type_environment: &TypeEnvironment,
) -> LiteralPatternResult<Expression> {
    let fixed_scalar = type_environment.fixed_scalar(subject_type_id);
    let float_subject = subject_type_id == type_environment.builtins().float;
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
                numeric_profile,
                fixed_scalar,
                float_subject,
            )?;
            token_stream.advance();
            expression
        }

        // Bool, char, and string literals.
        TokenTag::BOOL_LITERAL => {
            let span = Some(token_stream.current_span());
            let value = current_bool_literal(token_stream)
                .ok_or_else(|| malformed_literal_payload(token_stream))?;
            let expression = Expression::bool(value, span, ValueMode::ImmutableOwned);
            token_stream.advance();
            expression
        }
        TokenTag::CHAR_LITERAL => {
            let span = Some(token_stream.current_span());
            let value = current_char_literal(token_stream)
                .ok_or_else(|| malformed_literal_payload(token_stream))?;
            let expression = Expression::char(value, span, ValueMode::ImmutableOwned);
            token_stream.advance();
            expression
        }
        TokenTag::STRING_SLICE_LITERAL => {
            let span = Some(token_stream.current_span());
            let value = current_string_literal(token_stream, string_table)?
                .ok_or_else(|| malformed_literal_payload(token_stream))?;
            let expression = Expression::string_slice(value, span, ValueMode::ImmutableOwned);
            token_stream.advance();
            expression
        }

        // Negative numeric literal — consume the leading `-` then materialize via the helper.
        TokenTag::NEGATIVE => {
            let minus_sign_span = Some(token_stream.current_span());
            token_stream.advance();
            if token_stream.current_tag() != TokenTag::NUMERIC_LITERAL {
                return Err(CompilerDiagnostic::invalid_match_pattern(
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
                numeric_profile,
                fixed_scalar,
                float_subject,
            )?;
            token_stream.advance();
            expression
        }
        // Patterns that are never valid as literal matches.
        TokenTag::NONE_LITERAL => {
            return Err(CompilerDiagnostic::invalid_match_pattern(
                InvalidMatchPatternReason::NonePatternRequiresOptionalScrutinee,
                None,
                None,
                Some(token_stream.current_span()),
            )
            .into());
        }
        _ => {
            return Err(CompilerDiagnostic::invalid_match_pattern(
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
    if !is_type_compatible(subject_type_id, pattern.type_id, type_environment) {
        return Err(CompilerDiagnostic::type_mismatch(
            subject_type_id,
            pattern.type_id,
            TypeMismatchContext::MatchPattern,
            pattern.span,
        )
        .into());
    }

    Ok(pattern)
}

#[cfg(test)]
#[path = "tests/literal_tests.rs"]
mod tests;
