//! Literal pattern parsing.
//!
//! WHAT: parses int, float, bool, char, string, and negative-numeric literals
//! and dispatches to relational pattern parsing when the lead token is a comparator.
//! WHY: separating literal parsing from relational and choice parsing keeps each
//! submodule focused on one pattern category.

use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidMatchPatternReason, TypeMismatchContext,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::numeric_text::parse::{materialize_f64, materialize_i32_with_sign};
use crate::compiler_frontend::numeric_text::token::{
    NumericLiteralKind, NumericLiteralSign, NumericLiteralToken,
};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, TokenKind};
use crate::compiler_frontend::type_coercion::compatibility::is_type_compatible;
use crate::compiler_frontend::value_mode::ValueMode;

use super::diagnostics::reject_deferred_pattern_lead_token;
use super::relational::parse_relational_pattern;
use super::types::MatchPattern;

/// Result for the literal pattern family.
///
/// Literal parsing returns plain `CompilerDiagnostic` values on the diagnosed
/// lane; the surrounding parser boundary owns any infrastructure failure.
type LiteralPatternResult<T> = Result<T, CompilerDiagnostic>;

/// Materialize a `NumericLiteralToken` into an `Expression` with an explicit sign and location.
///
/// WHAT: handles both whole-number (`i32`) and decimal/exponent (`f64`) materialization
/// from a single token, applying the provided sign for range checks (whole numbers)
/// and float-value negation (decimal/exponent literals).
/// WHY: the positive and negative numeric arms in `parse_literal_pattern` share identical
/// materialization logic; this helper avoids duplicating the branch on `NumericLiteralKind`
/// and the two error-construction paths.
fn materialize_numeric_literal(
    token: &NumericLiteralToken,
    sign: NumericLiteralSign,
    span: Option<SourceSpan>,
    string_table: &StringTable,
) -> LiteralPatternResult<Expression> {
    if token.kind == NumericLiteralKind::WholeNumber {
        let value_i32 = materialize_i32_with_sign(token, sign, string_table).map_err(|reason| {
            CompilerDiagnostic::invalid_number_literal(token.source_text, reason, span)
        })?;

        Ok(Expression::int(value_i32, span, ValueMode::ImmutableOwned))
    } else {
        let value = materialize_f64(token, string_table).map_err(|reason| {
            CompilerDiagnostic::invalid_number_literal(token.source_text, reason, span)
        })?;

        // Negate the float when the sign is negative; the normalised text is unsigned
        // and always carries the positive magnitude.
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

/// Parse a non-choice match pattern, dispatching to relational or literal parsers.
pub fn parse_non_choice_pattern(
    token_stream: &mut FileTokens,
    subject_type_id: TypeId,
    string_table: &StringTable,
    type_environment: &TypeEnvironment,
) -> LiteralPatternResult<MatchPattern> {
    match token_stream.current_token_kind() {
        TokenKind::LessThan
        | TokenKind::LessThanOrEqual
        | TokenKind::GreaterThan
        | TokenKind::GreaterThanOrEqual => parse_relational_pattern(
            token_stream,
            subject_type_id,
            string_table,
            type_environment,
        ),

        _ => {
            let literal = parse_literal_pattern(
                token_stream,
                subject_type_id,
                string_table,
                type_environment,
            )?;
            Ok(MatchPattern::Literal(literal))
        }
    }
}

/// Parse a literal value pattern and type-check it against the scrutinee.
///
/// WHAT: accepts int, float, bool, char, string, and negative numeric literals and
/// verifies the pattern type is compatible with the scrutinee type.
/// WHY: catching type mismatches at parse time produces better source-located errors
/// than deferring the check to HIR lowering.
pub(super) fn parse_literal_pattern(
    token_stream: &mut FileTokens,
    subject_type_id: TypeId,
    string_table: &StringTable,
    type_environment: &TypeEnvironment,
) -> LiteralPatternResult<Expression> {
    if let Some(diagnostic) = reject_deferred_pattern_lead_token(token_stream) {
        return Err(diagnostic);
    }

    let pattern = match token_stream.current_token_kind() {
        // Numeric literal — use the shared materialization helper.
        TokenKind::NumericLiteral(token) => {
            let span = Some(token_stream.current_span());
            let token = token.to_owned();

            let expression = materialize_numeric_literal(&token, token.sign, span, string_table)?;
            token_stream.advance();
            expression
        }

        // Bool, char, and string literals.
        TokenKind::BoolLiteral(value) => {
            let span = Some(token_stream.current_span());
            let expression = Expression::bool(*value, span, ValueMode::ImmutableOwned);
            token_stream.advance();
            expression
        }
        TokenKind::CharLiteral(value) => {
            let span = Some(token_stream.current_span());
            let expression = Expression::char(*value, span, ValueMode::ImmutableOwned);
            token_stream.advance();
            expression
        }
        TokenKind::StringSliceLiteral(value) => {
            let span = Some(token_stream.current_span());
            let expression = Expression::string_slice(*value, span, ValueMode::ImmutableOwned);
            token_stream.advance();
            expression
        }

        // Negative numeric literal — consume the leading `-` then materialize via the helper.
        TokenKind::Negative => {
            let minus_sign_span = Some(token_stream.current_span());
            token_stream.advance();

            match token_stream.current_token_kind() {
                TokenKind::NumericLiteral(token) => {
                    let token = token.to_owned();
                    let expression = materialize_numeric_literal(
                        &token,
                        NumericLiteralSign::Negative,
                        minus_sign_span,
                        string_table,
                    )?;
                    token_stream.advance();
                    expression
                }
                _ => {
                    return Err(CompilerDiagnostic::invalid_match_pattern(
                        InvalidMatchPatternReason::NegativeLiteralNotNumeric,
                        None,
                        None,
                        Some(token_stream.current_span()),
                    ));
                }
            }
        }

        // Patterns that are never valid as literal matches.
        TokenKind::NoneLiteral => {
            return Err(CompilerDiagnostic::invalid_match_pattern(
                InvalidMatchPatternReason::NonePatternRequiresOptionalScrutinee,
                None,
                None,
                Some(token_stream.current_span()),
            ));
        }
        _ => {
            return Err(CompilerDiagnostic::invalid_match_pattern(
                InvalidMatchPatternReason::LiteralTypeUnsupported,
                None,
                None,
                Some(token_stream.current_span()),
            ));
        }
    };

    // -------------------------------
    //  Type-check the literal pattern
    // -------------------------------
    //
    // Reject literal patterns whose type is incompatible with the scrutinee type
    // at parse time so the user gets a source-located error immediately.
    if !is_type_compatible(subject_type_id, pattern.type_id, type_environment) {
        return Err(CompilerDiagnostic::type_mismatch(
            subject_type_id,
            pattern.type_id,
            TypeMismatchContext::MatchPattern,
            pattern.span,
        ));
    }

    Ok(pattern)
}

#[cfg(test)]
#[path = "tests/literal_tests.rs"]
mod tests;
