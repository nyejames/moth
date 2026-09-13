//! Option pattern parsing.
//!
//! WHAT: parses the `T?` match surface: `none =>` for absence, literal and
//! relational patterns for present values, and `|value|` for present capture.
//! WHY: option matching needs a narrow owner that preserves the compiler-owned
//! carrier model without exposing public `Option` constructors.

use crate::compiler_frontend::ast::statements::match_patterns::{
    MatchPattern, parse_non_choice_pattern,
};
use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, InvalidMatchPatternReason};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::symbols::path_interner::PathId;

use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, TokenKind};

/// Result for the option pattern family.
///
/// Option parsing returns plain `CompilerDiagnostic` values on the diagnosed
/// lane; the surrounding parser boundary owns any infrastructure failure.
type OptionPatternResult<T> = Result<T, CompilerDiagnostic>;

/// Parse a pattern for an optional scrutinee.
///
/// `none =>` is a presence check and does not require equality support from
/// the inner type. `|name|` matches any present value and binds the inner payload.
/// Literal present-value patterns compare the option payload for equality, so the
/// inner type must support runtime equality. Relational present-value patterns are
/// forwarded without additional validation.
pub fn parse_option_pattern(
    token_stream: &mut FileTokens,
    option_inner_type_id: TypeId,
    string_table: &StringTable,
    type_environment: &TypeEnvironment,
) -> OptionPatternResult<MatchPattern> {
    if token_stream.current_token_kind() == &TokenKind::NoneLiteral {
        let span = Some(token_stream.current_span());
        token_stream.advance();
        return Ok(MatchPattern::OptionNone { span });
    }

    if token_stream.current_token_kind() == &TokenKind::TypeParameterBracket {
        return parse_option_present_capture(token_stream, option_inner_type_id, string_table);
    }

    // Option present-value checks reuse the non-choice parser's plain
    // diagnostic boundary without an intermediate representation.
    let pattern = parse_non_choice_pattern(
        token_stream,
        option_inner_type_id,
        string_table,
        type_environment,
    )?;

    match pattern {
        MatchPattern::Literal(value) => {
            if !type_environment.supports_runtime_equality(option_inner_type_id) {
                return Err(CompilerDiagnostic::invalid_match_pattern(
                    InvalidMatchPatternReason::OptionValuePatternRequiresEquality,
                    None,
                    None,
                    value.span,
                ));
            }

            let span = value.span;
            Ok(MatchPattern::OptionValue { value, span })
        }

        MatchPattern::Relational { .. } => Ok(pattern),

        // parse_non_choice_pattern only returns Literal or Relational for
        // non-choice patterns, so any other variant is unreachable here.
        _ => {
            unreachable!("parse_non_choice_pattern returned unexpected pattern variant for option")
        }
    }
}
/// Parse `|name|` present capture for an optional scrutinee.
///
/// Validates:
/// - `||` is rejected.
/// - Multiple names are rejected.
/// - Type annotations inside `|...|` are rejected.
fn parse_option_present_capture(
    token_stream: &mut FileTokens,
    inner_type_id: TypeId,
    _string_table: &StringTable,
) -> OptionPatternResult<MatchPattern> {
    let span = Some(token_stream.current_span());
    token_stream.advance(); // consume opening '|'
    token_stream.skip_newlines();

    let name = match token_stream.current_token_kind() {
        TokenKind::Symbol(name) => *name,

        TokenKind::TypeParameterBracket => {
            return Err(CompilerDiagnostic::invalid_match_pattern(
                InvalidMatchPatternReason::EmptyOptionPresentCapture,
                None,
                None,
                Some(token_stream.current_span()),
            ));
        }

        _ => {
            return Err(CompilerDiagnostic::invalid_match_pattern(
                InvalidMatchPatternReason::ExpectedBindingInOptionPresentCapture,
                None,
                None,
                Some(token_stream.current_span()),
            ));
        }
    };
    let binding_span = Some(token_stream.current_span());
    token_stream.advance();

    // Reject type annotations such as `|name String|`.
    if matches!(token_stream.current_token_kind(), TokenKind::Symbol(_)) {
        return Err(CompilerDiagnostic::invalid_match_pattern(
            InvalidMatchPatternReason::OptionPresentCaptureTypeAnnotation,
            None,
            None,
            Some(token_stream.current_span()),
        ));
    }

    token_stream.skip_newlines();

    if token_stream.current_token_kind() != &TokenKind::TypeParameterBracket {
        return Err(CompilerDiagnostic::invalid_match_pattern(
            InvalidMatchPatternReason::MissingClosingPipe,
            None,
            None,
            Some(token_stream.current_span()),
        ));
    }
    token_stream.advance(); // consume closing '|'

    // Binding path is filled in by the caller (branching.rs) once the arm scope is known.
    Ok(MatchPattern::OptionPresentCapture {
        name,
        binding_path: PathId::ROOT,
        inner_type_id,
        span,
        binding_span,
    })
}
