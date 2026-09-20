//! Bounded expression parsing regression tests.
//!
//! WHAT: validates `create_expression_until` edge cases after the token-window refactor.
//! WHY: the bounded window replaces token-copying with a temporary `length` cap; these tests
//!      prove the cap behaves correctly for delimiters, nesting, and EOF boundaries.

use super::*;
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression::ExpressionKind;
use crate::compiler_frontend::ast::expressions::parse_expression_input::{
    ExpressionParseInput, ExpressionParseResources,
};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::ast::{ContextKind, ScopeContext, TopLevelDeclarationTable};
use crate::compiler_frontend::compiler_messages::{
    DiagnosticKind, DiagnosticPayload, DiagnosticToken, SyntaxDiagnosticKind,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::numeric_text::token::NumericLiteralToken;
use crate::compiler_frontend::source::{LocalSpan, SourceId};
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{
    SourceTokens, TestSourceTokensBuilder, TokenTag,
};
use crate::compiler_frontend::type_coercion::compatibility::TypeCompatibilityCache;
use crate::compiler_frontend::type_coercion::parse_context::{CastTargetContext, ExpectedType};
use crate::compiler_frontend::value_mode::ValueMode;
use std::rc::Rc;
use std::sync::Arc;

fn test_scope(
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> (PathId, ScopeContext) {
    let scope = path_fork
        .try_intern_portable_path("test.moth", string_table)
        .expect("test path fits");
    let mut scratch = Vec::new();
    assert_eq!(
        path_fork.render_portable(scope, string_table, &mut scratch),
        "test.moth",
        "scope PathId must be owned by the caller's fork",
    );
    let context = ScopeContext::new_for_tests(
        ContextKind::Expression,
        scope,
        Rc::new(TopLevelDeclarationTable::new(vec![], path_fork)),
        Arc::new(ExternalPackageRegistry::new()),
        vec![],
        0,
    );
    (scope, context)
}

fn numeric_token(
    builder: &mut TestSourceTokensBuilder,
    value: &str,
    string_table: &mut StringTable,
) {
    builder
        .push_numeric(
            NumericLiteralToken::test_new(value, string_table),
            LocalSpan::source_start(),
        )
        .expect("numeric fixture token should build");
}

fn static_token(builder: &mut TestSourceTokensBuilder, tag: TokenTag) {
    builder
        .push_static(tag, LocalSpan::source_start())
        .expect("static fixture token should build");
}

fn finish_tokens(builder: TestSourceTokensBuilder) -> std::sync::Arc<SourceTokens> {
    builder
        .finish()
        .expect("canonical fixture tokens should build")
}

fn create_expression_until_for_test(
    stream: &mut AstCursor,
    context: &ScopeContext,
    expected_type: &mut ExpectedType,
    value_mode: &ValueMode,
    stop_tokens: &[TokenTag],
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
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
        path_fork,
    });
    create_expression_until(input, stop_tokens)
}

#[test]
fn bounded_expression_empty_at_delimiter_errors() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (_scope, context) = test_scope(&mut string_table, &mut path_fork);

    let mut builder = TestSourceTokensBuilder::new(SourceId::COMPILATION_ROOT);
    static_token(&mut builder, TokenTag::COMMA);
    static_token(&mut builder, TokenTag::EOF);
    let owner = finish_tokens(builder);
    let range = owner
        .full_range()
        .expect("test token stream must expose a checked full range");
    let mut stream = AstCursor::from_source_tokens(&owner, range)
        .expect("test token stream must expose an AST cursor");
    let mut data_type = ExpectedType::Infer;

    let error = create_expression_until_for_test(
        &mut stream,
        &context,
        &mut data_type,
        &ValueMode::ImmutableOwned,
        &[TokenTag::COMMA],
        &mut string_table,
        &mut path_fork,
    )
    .expect_err("empty expression should error");

    let ExpressionParseError::Diagnostic(diagnostic) = error else {
        panic!("expected user diagnostic, found infrastructure error: {error:?}");
    };
    assert_eq!(
        diagnostic.kind,
        DiagnosticKind::Syntax(SyntaxDiagnosticKind::UnexpectedToken)
    );
    assert_eq!(diagnostic.kind.code(), "MOTH-SYNTAX-0002");
    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::UnexpectedToken { found }
            if found == DiagnosticToken::from_static_tag(TokenTag::COMMA)
    ));
}

#[test]
fn bounded_expression_parses_simple_literal() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (_scope, context) = test_scope(&mut string_table, &mut path_fork);

    let mut builder = TestSourceTokensBuilder::new(SourceId::COMPILATION_ROOT);
    numeric_token(&mut builder, "42", &mut string_table);
    static_token(&mut builder, TokenTag::COMMA);
    static_token(&mut builder, TokenTag::EOF);
    let owner = finish_tokens(builder);
    let range = owner
        .full_range()
        .expect("test token stream must expose a checked full range");
    let mut stream = AstCursor::from_source_tokens(&owner, range)
        .expect("test token stream must expose an AST cursor");
    let mut data_type = ExpectedType::Infer;

    let expression = create_expression_until_for_test(
        &mut stream,
        &context,
        &mut data_type,
        &ValueMode::ImmutableOwned,
        &[TokenTag::COMMA],
        &mut string_table,
        &mut path_fork,
    )
    .expect("simple literal should parse");

    assert!(matches!(expression.kind, ExpressionKind::Int(42)));

    // The stop token (comma) should not be consumed.
    assert_eq!(stream.position(), 1);
    assert_eq!(stream.current_tag(), TokenTag::COMMA);
}

#[test]
fn bounded_expression_nested_parentheses() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (_scope, context) = test_scope(&mut string_table, &mut path_fork);

    let mut builder = TestSourceTokensBuilder::new(SourceId::COMPILATION_ROOT);
    numeric_token(&mut builder, "1", &mut string_table);
    static_token(&mut builder, TokenTag::ADD);
    static_token(&mut builder, TokenTag::OPEN_PARENTHESIS);
    numeric_token(&mut builder, "2", &mut string_table);
    static_token(&mut builder, TokenTag::ADD);
    numeric_token(&mut builder, "3", &mut string_table);
    static_token(&mut builder, TokenTag::CLOSE_PARENTHESIS);
    static_token(&mut builder, TokenTag::COMMA);
    static_token(&mut builder, TokenTag::EOF);
    let owner = finish_tokens(builder);
    let range = owner
        .full_range()
        .expect("test token stream must expose a checked full range");
    let mut stream = AstCursor::from_source_tokens(&owner, range)
        .expect("test token stream must expose an AST cursor");
    let mut data_type = ExpectedType::Infer;

    let expression = create_expression_until_for_test(
        &mut stream,
        &context,
        &mut data_type,
        &ValueMode::ImmutableOwned,
        &[TokenTag::COMMA],
        &mut string_table,
        &mut path_fork,
    )
    .expect("nested parentheses should parse");

    // All literals fold to a single Int(6).
    assert!(matches!(expression.kind, ExpressionKind::Int(6)));

    // Stop token should remain unconsumed.
    assert_eq!(stream.position(), 7);
    assert_eq!(stream.current_tag(), TokenTag::COMMA);
}

#[test]
fn bounded_expression_nested_curly_braces() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (_scope, context) = test_scope(&mut string_table, &mut path_fork);

    // A collection literal `{2, 3}` followed by a comma.
    // The comma inside the collection must not terminate the bounded expression.
    let mut builder = TestSourceTokensBuilder::new(SourceId::COMPILATION_ROOT);
    static_token(&mut builder, TokenTag::OPEN_CURLY);
    numeric_token(&mut builder, "2", &mut string_table);
    static_token(&mut builder, TokenTag::COMMA);
    numeric_token(&mut builder, "3", &mut string_table);
    static_token(&mut builder, TokenTag::CLOSE_CURLY);
    static_token(&mut builder, TokenTag::COMMA);
    static_token(&mut builder, TokenTag::EOF);
    let owner = finish_tokens(builder);
    let range = owner
        .full_range()
        .expect("test token stream must expose a checked full range");
    let mut stream = AstCursor::from_source_tokens(&owner, range)
        .expect("test token stream must expose an AST cursor");
    let mut data_type = ExpectedType::Infer;

    let expression = create_expression_until_for_test(
        &mut stream,
        &context,
        &mut data_type,
        &ValueMode::ImmutableOwned,
        &[TokenTag::COMMA],
        &mut string_table,
        &mut path_fork,
    )
    .expect("nested curly braces should parse");

    // Should parse as a collection expression.
    assert!(matches!(expression.kind, ExpressionKind::Collection(_)));
    assert_eq!(stream.position(), 5);
    assert_eq!(stream.current_tag(), TokenTag::COMMA);
}

#[test]
fn bounded_expression_missing_delimiter_reaches_eof() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (_scope, context) = test_scope(&mut string_table, &mut path_fork);

    let mut builder = TestSourceTokensBuilder::new(SourceId::COMPILATION_ROOT);
    numeric_token(&mut builder, "1", &mut string_table);
    static_token(&mut builder, TokenTag::ADD);
    numeric_token(&mut builder, "2", &mut string_table);
    static_token(&mut builder, TokenTag::EOF);
    let owner = finish_tokens(builder);
    let range = owner
        .full_range()
        .expect("test token stream must expose a checked full range");
    let mut stream = AstCursor::from_source_tokens(&owner, range)
        .expect("test token stream must expose an AST cursor");
    let mut data_type = ExpectedType::Infer;

    let error = create_expression_until_for_test(
        &mut stream,
        &context,
        &mut data_type,
        &ValueMode::ImmutableOwned,
        &[TokenTag::COMMA],
        &mut string_table,
        &mut path_fork,
    )
    .expect_err("missing delimiter should error");

    let ExpressionParseError::Diagnostic(diagnostic) = error else {
        panic!("expected user diagnostic, found infrastructure error: {error:?}");
    };
    assert_eq!(
        diagnostic.kind,
        DiagnosticKind::Syntax(SyntaxDiagnosticKind::UnexpectedToken)
    );
    assert_eq!(diagnostic.kind.code(), "MOTH-SYNTAX-0002");
    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::UnexpectedToken { found }
            if found == DiagnosticToken::from_static_tag(TokenTag::EOF)
    ));
}
