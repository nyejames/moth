//! Literal match-pattern parsing regression tests.
//!
//! WHAT: validates that int, float, and negative numeric literal patterns are
//!       materialized with the same i32 range checks as expression literals.
//! WHY: match-pattern literal parsing has its own negative-literal fallback path,
//!      so it needs independent boundary coverage.

use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::statements::match_patterns::literal::parse_literal_pattern;
use crate::compiler_frontend::compiler_messages::{DiagnosticPayload, NumberLiteralErrorReason};
use crate::compiler_frontend::datatypes::builtin_type_ids;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::numeric_text::token::{
    NumericExponentSign, NumericLiteralKind, NumericLiteralSign, NumericLiteralToken,
};
use crate::compiler_frontend::source::{LocalSpan, SourceId};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{TestSourceTokensBuilder, TokenTag};

type LiteralPatternTestResult<T> = Result<T, ExpressionParseError>;

#[test]
fn parse_literal_pattern_accepts_i32_boundary_values() {
    let max = parse_whole_number_pattern(NumericLiteralSign::Positive, "2147483647").unwrap();
    assert!(matches!(max.kind, ExpressionKind::Int(2147483647)));

    let min = parse_whole_number_pattern(NumericLiteralSign::Negative, "2147483648").unwrap();
    assert!(matches!(min.kind, ExpressionKind::Int(-2147483648)));
}

#[test]
fn parse_literal_pattern_rejects_i32_out_of_range() {
    let error = parse_whole_number_pattern(NumericLiteralSign::Positive, "2147483648").unwrap_err();
    assert_invalid_number_literal_outside_range(error);

    let error = parse_whole_number_pattern(NumericLiteralSign::Negative, "2147483649").unwrap_err();
    assert_invalid_number_literal_outside_range(error);
}

#[test]
fn parse_literal_pattern_negative_fallback_allows_i32_min() {
    let min = parse_negative_number_pattern("2147483648").unwrap();
    assert!(matches!(min.kind, ExpressionKind::Int(-2147483648)));
}

#[test]
fn parse_literal_pattern_negative_fallback_rejects_i32_underflow() {
    let error = parse_negative_number_pattern("2147483649").unwrap_err();
    assert_invalid_number_literal_outside_range(error);
}

#[test]
fn malformed_numeric_literal_payload_is_infrastructure_error() {
    let source = SourceId::COMPILATION_ROOT;
    let mut builder = TestSourceTokensBuilder::new(source);
    builder
        .push_bool(TokenTag::BOOL_LITERAL, true, LocalSpan::source_start())
        .expect("valid test tokens should build");
    builder
        .push_static(TokenTag::EOF, LocalSpan::source_start())
        .expect("valid test tokens should build");
    let mut owner = builder
        .finish()
        .expect("valid test tokens should build a canonical source owner");
    std::sync::Arc::get_mut(&mut owner)
        .expect("the test owner should remain uniquely owned")
        .corrupt_payload_for_test(0, TokenTag::NUMERIC_LITERAL);
    let range = owner
        .full_range()
        .expect("the canonical test owner should expose a full range");
    let mut token_stream = AstCursor::from_source_tokens(&owner, range)
        .expect("the malformed canonical owner should still construct a cursor");
    let type_environment = TypeEnvironment::new();

    let error = parse_literal_pattern(
        &mut token_stream,
        builtin_type_ids::INT,
        &mut StringTable::new(),
        &type_environment,
    )
    .expect_err("a malformed trusted literal payload must not become syntax");

    assert!(matches!(error, ExpressionParseError::Infrastructure(_)));
}

fn assert_invalid_number_literal_outside_range(error: ExpressionParseError) {
    let diagnostic = match error {
        ExpressionParseError::Diagnostic(diagnostic) => diagnostic,
        ExpressionParseError::Infrastructure(error) => {
            panic!("literal pattern infrastructure failure is not a source diagnostic: {error:?}")
        }
    };

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidNumberLiteral {
            reason: NumberLiteralErrorReason::OutsideIntRange,
            ..
        }
    ));
}

fn parse_whole_number_pattern(
    sign: NumericLiteralSign,
    normalized_text: &str,
) -> LiteralPatternTestResult<Expression> {
    let mut string_table = StringTable::new();
    let text = string_table.intern(normalized_text);
    // For signed tokens the source_text includes the sign prefix.
    let source = match sign {
        NumericLiteralSign::Positive => normalized_text.to_owned(),
        NumericLiteralSign::Negative => format!("-{normalized_text}"),
    };
    let source_text = string_table.intern(&source);
    let literal = NumericLiteralToken::new(
        sign,
        source_text,
        text,
        NumericLiteralKind::WholeNumber,
        normalized_text
            .chars()
            .filter(|c| c.is_ascii_digit())
            .count() as u32,
        0,
        0,
        NumericExponentSign::None,
    );
    let mut builder = TestSourceTokensBuilder::new(SourceId::COMPILATION_ROOT);
    builder
        .push_numeric(literal, LocalSpan::source_start())
        .expect("numeric fixture token should build");
    builder
        .push_static(TokenTag::EOF, LocalSpan::source_start())
        .expect("EOF fixture token should build");
    let owner = builder
        .finish()
        .expect("test token stream must build a canonical source owner");
    let range = owner
        .full_range()
        .expect("test token stream must expose a checked full range");
    let mut token_stream = AstCursor::from_source_tokens(&owner, range)
        .expect("test token stream must expose an AST cursor");
    let type_environment = TypeEnvironment::new();

    parse_literal_pattern(
        &mut token_stream,
        builtin_type_ids::INT,
        &mut string_table,
        &type_environment,
    )
}

fn parse_negative_number_pattern(normalized_text: &str) -> LiteralPatternTestResult<Expression> {
    let mut string_table = StringTable::new();
    let text = string_table.intern(normalized_text);
    // In this path the Negative token is separate, so the literal is unsigned.
    let source_text = string_table.intern(normalized_text);
    let literal = NumericLiteralToken::new(
        NumericLiteralSign::Positive,
        source_text,
        text,
        NumericLiteralKind::WholeNumber,
        normalized_text
            .chars()
            .filter(|c| c.is_ascii_digit())
            .count() as u32,
        0,
        0,
        NumericExponentSign::None,
    );
    let mut builder = TestSourceTokensBuilder::new(SourceId::COMPILATION_ROOT);
    builder
        .push_static(TokenTag::NEGATIVE, LocalSpan::source_start())
        .expect("negative fixture token should build");
    builder
        .push_numeric(literal, LocalSpan::source_start())
        .expect("numeric fixture token should build");
    builder
        .push_static(TokenTag::EOF, LocalSpan::source_start())
        .expect("EOF fixture token should build");
    let owner = builder
        .finish()
        .expect("test token stream must build a canonical source owner");
    let range = owner
        .full_range()
        .expect("test token stream must expose a checked full range");
    let mut token_stream = AstCursor::from_source_tokens(&owner, range)
        .expect("test token stream must expose an AST cursor");
    let type_environment = TypeEnvironment::new();

    parse_literal_pattern(
        &mut token_stream,
        builtin_type_ids::INT,
        &mut string_table,
        &type_environment,
    )
}
