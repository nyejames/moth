//! Literal match-pattern parsing regression tests.
//!
//! WHAT: validates that int, float, and negative numeric literal patterns are
//!       materialized with the same i32 range checks as expression literals.
//! WHY: match-pattern literal parsing has its own negative-literal fallback path,
//!      so it needs independent boundary coverage.

use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::statements::match_patterns::literal::parse_literal_pattern;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticPayload, NumberLiteralErrorReason,
};
use crate::compiler_frontend::datatypes::builtin_type_ids;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::numeric_text::token::{
    NumericExponentSign, NumericLiteralKind, NumericLiteralSign, NumericLiteralToken,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, SourceLocation, Token, TokenKind};

type LiteralPatternTestResult<T> = Result<T, Box<CompilerDiagnostic>>;

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
    assert_invalid_number_literal_outside_range(*error);

    let error = parse_whole_number_pattern(NumericLiteralSign::Negative, "2147483649").unwrap_err();
    assert_invalid_number_literal_outside_range(*error);
}

#[test]
fn parse_literal_pattern_negative_fallback_allows_i32_min() {
    let min = parse_negative_number_pattern("2147483648").unwrap();
    assert!(matches!(min.kind, ExpressionKind::Int(-2147483648)));
}

#[test]
fn parse_literal_pattern_negative_fallback_rejects_i32_underflow() {
    let error = parse_negative_number_pattern("2147483649").unwrap_err();
    assert_invalid_number_literal_outside_range(*error);
}

fn assert_invalid_number_literal_outside_range(diagnostic: CompilerDiagnostic) {
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
    let scope = InternedPath::from_single_str("test.moth", &mut string_table);
    let text = string_table.intern(normalized_text);
    // For signed tokens the source_text includes the sign prefix.
    let source = match sign {
        NumericLiteralSign::Positive => normalized_text.to_owned(),
        NumericLiteralSign::Negative => format!("-{normalized_text}"),
    };
    let source_text = string_table.intern(&source);
    let tokens = vec![
        Token::new(
            TokenKind::NumericLiteral(NumericLiteralToken::new(
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
            )),
            SourceLocation::default(),
        ),
        Token::new(TokenKind::Eof, SourceLocation::default()),
    ];
    let mut token_stream = FileTokens::new(scope, tokens);
    let type_environment = TypeEnvironment::new();

    parse_literal_pattern(
        &mut token_stream,
        builtin_type_ids::INT,
        &string_table,
        &type_environment,
    )
}

fn parse_negative_number_pattern(normalized_text: &str) -> LiteralPatternTestResult<Expression> {
    let mut string_table = StringTable::new();
    let scope = InternedPath::from_single_str("test.moth", &mut string_table);
    let text = string_table.intern(normalized_text);
    // In this path the Negative token is separate, so the literal is unsigned.
    let source_text = string_table.intern(normalized_text);
    let tokens = vec![
        Token::new(TokenKind::Negative, SourceLocation::default()),
        Token::new(
            TokenKind::NumericLiteral(NumericLiteralToken::new(
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
            )),
            SourceLocation::default(),
        ),
        Token::new(TokenKind::Eof, SourceLocation::default()),
    ];
    let mut token_stream = FileTokens::new(scope, tokens);
    let type_environment = TypeEnvironment::new();

    parse_literal_pattern(
        &mut token_stream,
        builtin_type_ids::INT,
        &string_table,
        &type_environment,
    )
}
