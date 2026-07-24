//! Literal expression parsing regression tests.
//!
//! WHAT: validates parsing of int, float, string, char, bool, and template literals plus
//!       malformed-literal diagnostics.
//! WHY: literals are the simplest expressions but span many token kinds; targeted coverage
//!      prevents silent changes to literal type inference.

use super::*;
use crate::compiler_frontend::ast::expressions::expression::ExpressionKind;
use crate::compiler_frontend::ast::expressions::expression_rpn::ExpressionRpnItem;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::ast::{ContextKind, ScopeContext, TopLevelDeclarationTable};
use crate::compiler_frontend::compiler_messages::{
    DiagnosticKind, DiagnosticPayload, NumberLiteralErrorReason, SyntaxDiagnosticKind,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::numeric_text::token::{
    NumericExponentSign, NumericLiteralKind, NumericLiteralSign, NumericLiteralToken,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, SourceLocation, Token, TokenKind};
use crate::compiler_frontend::type_coercion::compatibility::TypeCompatibilityCache;
use crate::compiler_frontend::type_coercion::parse_context::ExpectedType;
use crate::compiler_frontend::value_mode::ValueMode;
use std::rc::Rc;
use std::sync::Arc;

#[test]
fn parse_literal_expression_accepts_signed_i32_min_token() {
    let outcome = parse_whole_number_token(NumericLiteralSign::Negative, "2147483648", 10, false)
        .expect("signed i32 minimum boundary should parse");

    assert!(!outcome.next_number_negative);
    assert_eq!(outcome.expression.len(), 1);

    let ExpressionRpnItem::Operand(value) = &outcome.expression[0] else {
        panic!("literal should parse to an operand item");
    };
    assert!(matches!(value.kind, ExpressionKind::Int(v) if v == i32::MIN));
}

#[test]
fn parse_literal_expression_rejects_positive_i32_overflow() {
    let error = parse_whole_number_token(NumericLiteralSign::Positive, "2147483648", 10, false)
        .expect_err("positive i32 overflow should return a syntax diagnostic");

    let diagnostic = error
        .diagnostic()
        .expect("literal overflow should remain a typed diagnostic");
    assert_eq!(
        diagnostic.kind,
        DiagnosticKind::Syntax(SyntaxDiagnosticKind::InvalidNumberLiteral)
    );
    assert_eq!(diagnostic.kind.code(), "MOTH-SYNTAX-0008");
    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidNumberLiteral {
            reason: NumberLiteralErrorReason::OutsideIntRange,
            ..
        }
    ));
}

#[test]
fn parse_literal_expression_effective_negative_sign_allows_i32_min() {
    let outcome = parse_whole_number_token(NumericLiteralSign::Positive, "2147483648", 10, true)
        .expect("effective negative sign should allow -2147483648");

    assert!(!outcome.next_number_negative);
    assert_eq!(outcome.expression.len(), 1);

    let ExpressionRpnItem::Operand(value) = &outcome.expression[0] else {
        panic!("literal should parse to an operand item");
    };
    assert!(matches!(value.kind, ExpressionKind::Int(v) if v == i32::MIN));
}

#[test]
fn parse_literal_expression_effective_negative_sign_rejects_i32_underflow() {
    let error = parse_whole_number_token(NumericLiteralSign::Positive, "2147483649", 10, true)
        .expect_err("effective negative sign should reject -2147483649");

    let diagnostic = error
        .diagnostic()
        .expect("literal underflow should remain a typed diagnostic");
    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidNumberLiteral {
            reason: NumberLiteralErrorReason::OutsideIntRange,
            ..
        }
    ));
}

#[derive(Debug)]
struct LiteralParseOutcome {
    expression: Vec<ExpressionRpnItem>,
    next_number_negative: bool,
}

fn parse_whole_number_token(
    sign: NumericLiteralSign,
    normalized_text: &str,
    digit_count: u32,
    next_number_negative: bool,
) -> Result<LiteralParseOutcome, ExpressionParseError> {
    let mut string_table = StringTable::new();
    let scope = InternedPath::from_single_str("test.moth", &mut string_table);
    let context = ScopeContext::new_for_tests(
        ContextKind::Expression,
        scope.clone(),
        Rc::new(TopLevelDeclarationTable::new(vec![])),
        Arc::new(ExternalPackageRegistry::new()),
        vec![],
        0,
    );

    let min_text = string_table.intern(normalized_text);
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
                min_text,
                NumericLiteralKind::WholeNumber,
                digit_count,
                0,
                0,
                NumericExponentSign::None,
            )),
            SourceLocation::default(),
        ),
        Token::new(TokenKind::Eof, SourceLocation::default()),
    ];
    let mut token_stream = FileTokens::new(scope, tokens);
    let mut expression = Vec::new();
    let mut next_number_negative = next_number_negative;
    let mut type_environment = TypeEnvironment::new();
    let mut compatibility_cache = TypeCompatibilityCache::new();
    let mut type_interner = AstTypeInterner::new(&mut type_environment, &mut compatibility_cache);
    let expected_type = ExpectedType::Infer;
    let value_mode = ValueMode::ImmutableOwned;

    {
        let mut literal_state = LiteralParseState {
            expected_type: &expected_type,
            value_mode: &value_mode,
            expression: &mut expression,
            next_number_negative: &mut next_number_negative,
            allow_boundary_catch: true,
        };

        parse_literal_expression(
            &mut token_stream,
            &context,
            &mut type_interner,
            &mut literal_state,
            &mut string_table,
        )?;
    }

    Ok(LiteralParseOutcome {
        expression,
        next_number_negative,
    })
}
