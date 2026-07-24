//! Bounded expression parsing regression tests.
//!
//! WHAT: validates `create_expression_until` edge cases after the token-window refactor.
//! WHY: the bounded window replaces token-copying with a temporary `length` cap; these tests
//!      prove the cap behaves correctly for delimiters, nesting, and EOF boundaries.

use super::*;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression::ExpressionKind;
use crate::compiler_frontend::ast::expressions::parse_expression_input::{
    ExpressionParseInput, ExpressionParseResources,
};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::ast::{ContextKind, ScopeContext, TopLevelDeclarationTable};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticKind, DiagnosticPayload, SyntaxDiagnosticKind,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::numeric_text::token::NumericLiteralToken;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, SourceLocation, Token, TokenKind};
use crate::compiler_frontend::type_coercion::compatibility::TypeCompatibilityCache;
use crate::compiler_frontend::type_coercion::parse_context::{CastTargetContext, ExpectedType};
use crate::compiler_frontend::value_mode::ValueMode;
use std::rc::Rc;
use std::sync::Arc;

fn test_scope(string_table: &mut StringTable) -> (InternedPath, ScopeContext) {
    let scope = InternedPath::from_single_str("test.moth", string_table);
    let context = ScopeContext::new_for_tests(
        ContextKind::Expression,
        scope.clone(),
        Rc::new(TopLevelDeclarationTable::new(vec![])),
        Arc::new(ExternalPackageRegistry::new()),
        vec![],
        0,
    );
    (scope, context)
}

fn numeric_token(value: &str, scope: &InternedPath, string_table: &mut StringTable) -> Token {
    Token::new(
        TokenKind::NumericLiteral(NumericLiteralToken::test_new(value, string_table)),
        SourceLocation::new(scope.clone(), Default::default(), Default::default()),
    )
}

fn token(kind: TokenKind, scope: &InternedPath) -> Token {
    Token::new(
        kind,
        SourceLocation::new(scope.clone(), Default::default(), Default::default()),
    )
}

fn create_expression_until_for_test(
    stream: &mut FileTokens,
    context: &ScopeContext,
    expected_type: &mut ExpectedType,
    value_mode: &ValueMode,
    stop_tokens: &[TokenKind],
    string_table: &mut StringTable,
) -> Result<crate::compiler_frontend::ast::expressions::expression::Expression, ExpressionParseError>
{
    let mut type_environment = TypeEnvironment::new();
    let mut compatibility_cache = TypeCompatibilityCache::new();
    let mut type_interner = AstTypeInterner::new(&mut type_environment, &mut compatibility_cache);
    let mut cast_target_context = CastTargetContext::None;
    let input = ExpressionParseInput::until(ExpressionParseResources {
        token_stream: stream,
        scope_context: context,
        type_interner: &mut type_interner,
        expected_type,
        cast_target_context: &mut cast_target_context,
        value_mode,
        string_table,
    });
    create_expression_until(input, stop_tokens)
}

fn typed_diagnostic_from_error(error: ExpressionParseError) -> CompilerDiagnostic {
    CompilerDiagnostic::from(error)
}

#[test]
fn bounded_expression_empty_at_delimiter_errors() {
    let mut string_table = StringTable::new();
    let (scope, context) = test_scope(&mut string_table);

    let tokens = vec![
        token(TokenKind::Comma, &scope),
        token(TokenKind::Eof, &scope),
    ];
    let mut stream = FileTokens::new(scope, tokens);
    let mut data_type = ExpectedType::Infer;

    let error = create_expression_until_for_test(
        &mut stream,
        &context,
        &mut data_type,
        &ValueMode::ImmutableOwned,
        &[TokenKind::Comma],
        &mut string_table,
    )
    .expect_err("empty expression should error");

    let diagnostic = typed_diagnostic_from_error(error);
    assert_eq!(
        diagnostic.kind,
        DiagnosticKind::Syntax(SyntaxDiagnosticKind::UnexpectedToken)
    );
    assert_eq!(diagnostic.kind.code(), "MOTH-SYNTAX-0002");
    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::UnexpectedToken {
            found: TokenKind::Comma
        }
    ));
}

#[test]
fn bounded_expression_parses_simple_literal() {
    let mut string_table = StringTable::new();
    let (scope, context) = test_scope(&mut string_table);

    let tokens = vec![
        numeric_token("42", &scope, &mut string_table),
        token(TokenKind::Comma, &scope),
        token(TokenKind::Eof, &scope),
    ];
    let mut stream = FileTokens::new(scope.clone(), tokens);
    let mut data_type = ExpectedType::Infer;

    let expression = create_expression_until_for_test(
        &mut stream,
        &context,
        &mut data_type,
        &ValueMode::ImmutableOwned,
        &[TokenKind::Comma],
        &mut string_table,
    )
    .expect("simple literal should parse");

    assert!(matches!(expression.kind, ExpressionKind::Int(42)));

    // The stop token (comma) should not be consumed.
    assert_eq!(stream.index, 1);
    assert_eq!(stream.current_token_kind(), &TokenKind::Comma);
}

#[test]
fn bounded_expression_nested_parentheses() {
    let mut string_table = StringTable::new();
    let (scope, context) = test_scope(&mut string_table);

    let tokens = vec![
        numeric_token("1", &scope, &mut string_table),
        token(TokenKind::Add, &scope),
        token(TokenKind::OpenParenthesis, &scope),
        numeric_token("2", &scope, &mut string_table),
        token(TokenKind::Add, &scope),
        numeric_token("3", &scope, &mut string_table),
        token(TokenKind::CloseParenthesis, &scope),
        token(TokenKind::Comma, &scope),
        token(TokenKind::Eof, &scope),
    ];
    let mut stream = FileTokens::new(scope.clone(), tokens);
    let mut data_type = ExpectedType::Infer;

    let expression = create_expression_until_for_test(
        &mut stream,
        &context,
        &mut data_type,
        &ValueMode::ImmutableOwned,
        &[TokenKind::Comma],
        &mut string_table,
    )
    .expect("nested parentheses should parse");

    // All literals fold to a single Int(6).
    assert!(matches!(expression.kind, ExpressionKind::Int(6)));

    // Stop token should remain unconsumed.
    assert_eq!(stream.index, 7);
    assert_eq!(stream.current_token_kind(), &TokenKind::Comma);
}

#[test]
fn bounded_expression_nested_curly_braces() {
    let mut string_table = StringTable::new();
    let (scope, context) = test_scope(&mut string_table);

    // A collection literal `{2, 3}` followed by a comma.
    // The comma inside the collection must not terminate the bounded expression.
    let tokens = vec![
        token(TokenKind::OpenCurly, &scope),
        numeric_token("2", &scope, &mut string_table),
        token(TokenKind::Comma, &scope),
        numeric_token("3", &scope, &mut string_table),
        token(TokenKind::CloseCurly, &scope),
        token(TokenKind::Comma, &scope),
        token(TokenKind::Eof, &scope),
    ];
    let mut stream = FileTokens::new(scope.clone(), tokens);
    let mut data_type = ExpectedType::Infer;

    let expression = create_expression_until_for_test(
        &mut stream,
        &context,
        &mut data_type,
        &ValueMode::ImmutableOwned,
        &[TokenKind::Comma],
        &mut string_table,
    )
    .expect("nested curly braces should parse");

    // Should parse as a collection expression.
    assert!(matches!(expression.kind, ExpressionKind::Collection(_)));
    assert_eq!(stream.index, 5);
    assert_eq!(stream.current_token_kind(), &TokenKind::Comma);
}

#[test]
fn bounded_expression_missing_delimiter_reaches_eof() {
    let mut string_table = StringTable::new();
    let (scope, context) = test_scope(&mut string_table);

    let tokens = vec![
        numeric_token("1", &scope, &mut string_table),
        token(TokenKind::Add, &scope),
        numeric_token("2", &scope, &mut string_table),
        token(TokenKind::Eof, &scope),
    ];
    let mut stream = FileTokens::new(scope, tokens);
    let mut data_type = ExpectedType::Infer;

    let error = create_expression_until_for_test(
        &mut stream,
        &context,
        &mut data_type,
        &ValueMode::ImmutableOwned,
        &[TokenKind::Comma],
        &mut string_table,
    )
    .expect_err("missing delimiter should error");

    let diagnostic = typed_diagnostic_from_error(error);
    assert_eq!(
        diagnostic.kind,
        DiagnosticKind::Syntax(SyntaxDiagnosticKind::UnexpectedToken)
    );
    assert_eq!(diagnostic.kind.code(), "MOTH-SYNTAX-0002");
    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::UnexpectedToken {
            found: TokenKind::Eof
        }
    ));
}
