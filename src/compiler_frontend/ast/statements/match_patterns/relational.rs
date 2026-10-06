//! Relational pattern parsing.
//!
//! WHAT: parses `<`, `<=`, `>`, `>=` match patterns and validates the subject
//! type is an ordered scalar.
//! WHY: relational patterns share literal parsing but have distinct validation
//! rules, so they live in a dedicated submodule.

use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::statements::match_patterns::{
    MatchPattern, RelationalPatternOp, literal::parse_literal_pattern,
};
use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, InvalidMatchPatternReason};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::TokenTag;
use moth_lexical::numeric::profile::NumericProfile;
/// Result for the relational pattern family.
///
/// Authored mismatches remain diagnostics while malformed retained payloads stay infrastructure.
type RelationalPatternResult<T> = Result<T, ExpressionParseError>;

/// Parse a relational comparison pattern (`<`, `<=`, `>`, `>=`).
///
/// Validates that the subject type supports ordering, then parses the literal
/// operand that follows the operator.
pub(super) fn parse_relational_pattern(
    token_stream: &mut AstCursor,
    subject_type_id: TypeId,
    numeric_profile: NumericProfile,
    string_table: &mut StringTable,
    type_environment: &TypeEnvironment,
) -> RelationalPatternResult<MatchPattern> {
    let span = Some(token_stream.current_span());

    let op = match token_stream.current_tag() {
        TokenTag::LESS_THAN => RelationalPatternOp::LessThan,
        TokenTag::LESS_THAN_OR_EQUAL => RelationalPatternOp::LessThanOrEqual,
        TokenTag::GREATER_THAN => RelationalPatternOp::GreaterThan,
        TokenTag::GREATER_THAN_OR_EQUAL => RelationalPatternOp::GreaterThanOrEqual,
        _ => unreachable!("caller checked relational lead token"),
    };

    token_stream.advance();
    token_stream.skip_newlines();

    // Reject relational patterns on unsupported subject types before attempting
    // to parse the literal value. This ensures the diagnostic refers to the
    // pattern category (relational) rather than a literal type mismatch.
    ensure_relational_subject_type(subject_type_id, span, string_table, type_environment)?;

    let value = parse_literal_pattern(
        token_stream,
        subject_type_id,
        numeric_profile,
        string_table,
        type_environment,
    )?;

    Ok(MatchPattern::Relational { op, value, span })
}
/// Ensure the subject type supports relational ordering.
///
/// Int, Uint, Float, Char, fixed-width numeric scalars, Byte and the Dec family have ordered
/// comparisons.
fn ensure_relational_subject_type(
    subject_type_id: TypeId,
    span: Option<SourceSpan>,
    _string_table: &StringTable,
    type_environment: &TypeEnvironment,
) -> RelationalPatternResult<()> {
    let builtins = type_environment.builtins();

    let is_ordered_scalar = subject_type_id == builtins.int
        || subject_type_id == builtins.uint
        || subject_type_id == builtins.float
        || subject_type_id == builtins.char
        || type_environment.fixed_scalar(subject_type_id).is_some()
        || type_environment.number_scale(subject_type_id).is_some();

    if !is_ordered_scalar {
        return Err(CompilerDiagnostic::invalid_match_pattern(
            InvalidMatchPatternReason::ScrutineeTypeUnsupportedForRelational,
            None,
            None,
            span,
        )
        .into());
    }

    Ok(())
}
