use super::*;
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::expression_store::{
    ExprId, ExpressionOperationView,
};
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::template::{
    CommentDirectiveKind, SlotKey, TemplateType,
};
use crate::compiler_frontend::ast::templates::template_head_parser::directive_args::{
    parse_optional_parenthesized_expression, parse_optional_slot_target_argument,
    parse_required_parenthesized_expression, parse_required_slot_name_argument,
    reject_unexpected_directive_arguments,
};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_messages::diagnostics::families::calls_data::{
    InvalidCallShapeCode, InvalidCallShapeView,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::templates::MalformedCssTemplateView;
use crate::compiler_frontend::compiler_messages::diagnostics::families::templates::{
    InvalidTemplateDirectiveCode, InvalidTemplateDirectiveView,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::token_lexical::{
    ExpectedTokenView, UnexpectedTokenShape, UnexpectedTokenView,
};
use crate::compiler_frontend::compiler_messages::diagnostics::foreign::SiteRef;
use crate::compiler_frontend::compiler_messages::diagnostics::test_support::CompactDiagnosticBagForTest;
use crate::compiler_frontend::compiler_messages::diagnostics::test_support::compact_bag_for_test;
use crate::compiler_frontend::compiler_messages::{
    DiagnosticBag, DiagnosticToken, InvalidTemplateDirectiveReason,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::source::{ExtendedSpanBuilder, LocalSpan, SourceId, SourceSpan};
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::type_coercion::compatibility::TypeCompatibilityCache;
use std::rc::Rc;
use std::sync::Arc;

type DirectiveStyleTestResult<T> = Result<T, DirectiveStyleTestError>;

#[derive(Debug)]
struct DirectiveStyleTestError {
    diagnostics: Vec<DirectiveStyleTestDiagnostic>,
}

impl DirectiveStyleTestError {
    fn first(&self) -> Option<&DirectiveStyleTestDiagnostic> {
        self.diagnostics.first()
    }
}

#[derive(Debug)]
struct DirectiveStyleTestDiagnostic {
    kind: DirectiveStyleTestDiagnosticKind,
    primary_site: DirectiveStyleTestSite,
    label_count: usize,
}

#[derive(Debug)]
enum DirectiveStyleTestDiagnosticKind {
    InvalidTemplateDirective {
        reason: InvalidTemplateDirectiveCode,
        detail: Option<StringId>,
    },
    UnexpectedToken(Option<DiagnosticToken>),
    ExpectedToken(DiagnosticToken),
    InvalidCallShape(InvalidCallShapeCode),
    Other,
}

#[derive(Clone, Copy, Debug)]
enum DirectiveStyleTestSite {
    Absent,
    Local(SourceSpan),
    Foreign,
}

fn directive_diagnostic(error: TemplateError) -> DirectiveStyleTestError {
    summarize_diagnostics(expect_template_diagnostic(error))
}

fn summarize_diagnostics(diagnostics: CompactDiagnosticBagForTest) -> DirectiveStyleTestError {
    DirectiveStyleTestError {
        diagnostics: diagnostics
            .views()
            .map(|view| {
                let kind = if let Some(facts) = view.family_view::<InvalidTemplateDirectiveView>() {
                    DirectiveStyleTestDiagnosticKind::InvalidTemplateDirective {
                        reason: facts.reason,
                        detail: facts.detail,
                    }
                } else if let Some(facts) = view.family_view::<UnexpectedTokenView>() {
                    let found = match facts.shape {
                        UnexpectedTokenShape::Token(found) => Some(found),
                        UnexpectedTokenShape::MultiBind { .. } => None,
                    };
                    DirectiveStyleTestDiagnosticKind::UnexpectedToken(found)
                } else if let Some(facts) = view.family_view::<ExpectedTokenView>() {
                    DirectiveStyleTestDiagnosticKind::ExpectedToken(facts.expected)
                } else if let Some(facts) = view.family_view::<InvalidCallShapeView>() {
                    DirectiveStyleTestDiagnosticKind::InvalidCallShape(facts.reason)
                } else {
                    DirectiveStyleTestDiagnosticKind::Other
                };
                let primary_site = match view.primary_site() {
                    SiteRef::Absent => DirectiveStyleTestSite::Absent,
                    SiteRef::Local(span) => DirectiveStyleTestSite::Local(span),
                    SiteRef::Foreign { .. } => DirectiveStyleTestSite::Foreign,
                };
                DirectiveStyleTestDiagnostic {
                    kind,
                    primary_site,
                    label_count: view.durable_labels().count(),
                }
            })
            .collect(),
    }
}

fn assert_invalid_template_directive(
    diagnostic: &DirectiveStyleTestError,
    expected: InvalidTemplateDirectiveCode,
) {
    let facts = diagnostic
        .first()
        .expect("directive error should compact to one record");
    match &facts.kind {
        DirectiveStyleTestDiagnosticKind::InvalidTemplateDirective { reason, .. } => {
            assert_eq!(*reason, expected);
        }
        other => panic!("expected invalid-template-directive facts, found {other:?}"),
    }
}

fn assert_unexpected_token(diagnostic: &DirectiveStyleTestError, expected: DiagnosticToken) {
    let facts = diagnostic
        .first()
        .expect("unexpected-token diagnostic should compact to one record");
    match &facts.kind {
        DirectiveStyleTestDiagnosticKind::UnexpectedToken(Some(found)) => {
            assert_eq!(*found, expected);
        }
        other => panic!("expected unexpected-token facts, found {other:?}"),
    }
}

fn assert_expected_token(diagnostic: &DirectiveStyleTestError, expected: DiagnosticToken) {
    let facts = diagnostic
        .first()
        .expect("expected-token diagnostic should compact to one record");
    match &facts.kind {
        DirectiveStyleTestDiagnosticKind::ExpectedToken(found) => assert_eq!(*found, expected),
        other => panic!("expected expected-token facts, found {other:?}"),
    }
}
fn directive_tokens(
    source: &str,
    string_table: &mut StringTable,
    span_builder: &mut ExtendedSpanBuilder,
    path_fork: &mut PathInternerFork,
) -> TemplateSourceFixture {
    let style_directives = frontend_test_style_directives();
    template_tokens_from_source_with_style_directives(
        source,
        &style_directives,
        string_table,
        span_builder,
        path_fork,
    )
}
fn test_context(scope: PathId, path_fork: &PathInternerFork) -> ScopeContext {
    ScopeContext::new_for_tests(
        ContextKind::Constant,
        scope,
        Rc::new(TopLevelDeclarationTable::new(vec![], path_fork)),
        Arc::new(ExternalPackageRegistry::default()),
        0,
    )
    .with_source_file_scope(scope)
}

fn parse_optional_slot_target_for_test(
    directive_name: StringId,
    tokens: &mut AstCursor,
    scope: PathId,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> DirectiveStyleTestResult<SlotKey> {
    let context = test_context(scope, path_fork);
    let mut type_environment = TypeEnvironment::new();
    let mut compatibility_cache = TypeCompatibilityCache::new();
    let mut type_interner = AstTypeInterner::new(&mut type_environment, &mut compatibility_cache);
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    parse_optional_slot_target_argument(
        directive_name,
        tokens,
        &context,
        &mut type_interner,
        &mut expression_builder,
        &mut const_values,
        string_table,
        path_fork,
    )
    .map_err(directive_diagnostic)
}

fn parse_required_slot_name_for_test(
    directive_name: StringId,
    tokens: &mut AstCursor,
    scope: PathId,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> DirectiveStyleTestResult<StringId> {
    let context = test_context(scope, path_fork);
    let mut type_environment = TypeEnvironment::new();
    let mut compatibility_cache = TypeCompatibilityCache::new();
    let mut type_interner = AstTypeInterner::new(&mut type_environment, &mut compatibility_cache);
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    parse_required_slot_name_argument(
        directive_name,
        tokens,
        &context,
        &mut type_interner,
        &mut expression_builder,
        &mut const_values,
        string_table,
        path_fork,
    )
    .map_err(directive_diagnostic)
}

fn parse_optional_parenthesized_expression_for_test(
    tokens: &mut AstCursor,
    context: &ScopeContext,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> DirectiveStyleTestResult<Option<ExprId>> {
    let mut type_environment = TypeEnvironment::new();
    let mut compatibility_cache = TypeCompatibilityCache::new();
    let mut type_interner = AstTypeInterner::new(&mut type_environment, &mut compatibility_cache);
    let directive_name = string_table.intern("test_directive");
    parse_optional_parenthesized_expression(
        directive_name,
        tokens,
        context,
        &mut type_interner,
        expression_builder,
        const_values,
        string_table,
        path_fork,
    )
    .map_err(directive_diagnostic)
}

fn parse_required_parenthesized_expression_for_test(
    tokens: &mut AstCursor,
    context: &ScopeContext,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> DirectiveStyleTestResult<ExprId> {
    let mut type_environment = TypeEnvironment::new();
    let mut compatibility_cache = TypeCompatibilityCache::new();
    let mut type_interner = AstTypeInterner::new(&mut type_environment, &mut compatibility_cache);
    let directive_name = string_table.intern("test_directive");
    parse_required_parenthesized_expression(
        directive_name,
        tokens,
        context,
        &mut type_interner,
        expression_builder,
        const_values,
        string_table,
        path_fork,
    )
    .map_err(|error| {
        directive_diagnostic(error.into_template_error(
            directive_name,
            InvalidTemplateDirectiveReason::EmptyArguments,
        ))
    })
}

fn parse_optional_expression_source(source: &str) -> DirectiveStyleTestResult<Option<ExprId>> {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let file_tokens =
        directive_tokens(source, &mut string_table, &mut span_builder, &mut path_fork);
    let context = test_context(file_tokens.source_path, &path_fork);
    let canonical_owner = file_tokens
        .canonical_owner()
        .expect("test token stream must expose canonical source tokens");
    let canonical_range = canonical_owner
        .full_range()
        .expect("test token stream must expose canonical source range");
    let mut cursor = AstCursor::from_source_tokens(&canonical_owner, canonical_range)
        .expect("test token stream must expose an AST cursor");
    cursor
        .set_position(file_tokens.opener_index + 1)
        .expect("test token stream position must remain in canonical range");
    parse_optional_parenthesized_expression_for_test(
        &mut cursor,
        &context,
        &mut expression_builder,
        &mut const_values,
        &mut string_table,
        &mut path_fork,
    )
}

// ------------------------------------------------------------------------
// reject_unexpected_directive_arguments
// ------------------------------------------------------------------------

#[test]
fn reject_arguments_succeeds_when_no_parens() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let file_tokens = directive_tokens(
        "[$note]",
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    );
    let directive_name = string_table.intern("note");
    let canonical_owner = file_tokens
        .canonical_owner()
        .expect("test token stream must expose canonical source tokens");
    let canonical_range = canonical_owner
        .full_range()
        .expect("test token stream must expose canonical source range");
    let mut cursor = AstCursor::from_source_tokens(&canonical_owner, canonical_range)
        .expect("test token stream must expose an AST cursor");
    cursor
        .set_position(file_tokens.opener_index + 1)
        .expect("test token stream position must remain in canonical range");
    let result = reject_unexpected_directive_arguments(directive_name, &cursor);
    assert!(result.is_ok());
}

#[test]
fn reject_arguments_fails_when_parens_present() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let file_tokens = directive_tokens(
        "[$note()]",
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    );
    let directive_name = string_table.intern("note");
    let canonical_owner = file_tokens
        .canonical_owner()
        .expect("test token stream must expose canonical source tokens");
    let canonical_range = canonical_owner
        .full_range()
        .expect("test token stream must expose canonical source range");
    let mut cursor = AstCursor::from_source_tokens(&canonical_owner, canonical_range)
        .expect("test token stream must expose an AST cursor");
    cursor
        .set_position(file_tokens.opener_index + 1)
        .expect("test token stream position must remain in canonical range");
    let result = reject_unexpected_directive_arguments(directive_name, &cursor);
    assert!(result.is_err());
    let diagnostic = directive_diagnostic(result.unwrap_err());
    assert_invalid_template_directive(
        &diagnostic,
        InvalidTemplateDirectiveCode::UnexpectedArguments,
    );
}

// ------------------------------------------------------------------------
// parse_optional_slot_target_argument
// ------------------------------------------------------------------------

#[test]
fn optional_slot_target_no_parens_returns_default() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let file_tokens = directive_tokens(
        "[$slot]",
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    );
    let directive_name = string_table.intern("slot");
    let canonical_owner = file_tokens
        .canonical_owner()
        .expect("test token stream must expose canonical source tokens");
    let canonical_range = canonical_owner
        .full_range()
        .expect("test token stream must expose canonical source range");
    let mut cursor = AstCursor::from_source_tokens(&canonical_owner, canonical_range)
        .expect("test token stream must expose an AST cursor");
    cursor
        .set_position(file_tokens.opener_index + 1)
        .expect("test token stream position must remain in canonical range");
    let result = parse_optional_slot_target_for_test(
        directive_name,
        &mut cursor,
        file_tokens.source_path,
        &mut string_table,
        &mut path_fork,
    );
    assert_eq!(
        result.unwrap_or_else(|_| panic!("slot target should parse successfully")),
        SlotKey::Default
    );
}

#[test]
fn optional_slot_target_named_string() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let file_tokens = directive_tokens(
        "[$slot(\"style\")]",
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    );
    let directive_name = string_table.intern("slot");
    let canonical_owner = file_tokens
        .canonical_owner()
        .expect("test token stream must expose canonical source tokens");
    let canonical_range = canonical_owner
        .full_range()
        .expect("test token stream must expose canonical source range");
    let mut cursor = AstCursor::from_source_tokens(&canonical_owner, canonical_range)
        .expect("test token stream must expose an AST cursor");
    cursor
        .set_position(file_tokens.opener_index + 1)
        .expect("test token stream position must remain in canonical range");
    let result = parse_optional_slot_target_for_test(
        directive_name,
        &mut cursor,
        file_tokens.source_path,
        &mut string_table,
        &mut path_fork,
    );
    assert!(matches!(
        result.unwrap_or_else(|_| panic!("named slot target should parse successfully")),
        SlotKey::Named(_)
    ));
}

#[test]
fn optional_slot_target_positive_positional() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let file_tokens = directive_tokens(
        "[$slot(1)]",
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    );
    let directive_name = string_table.intern("slot");
    let canonical_owner = file_tokens
        .canonical_owner()
        .expect("test token stream must expose canonical source tokens");
    let canonical_range = canonical_owner
        .full_range()
        .expect("test token stream must expose canonical source range");
    let mut cursor = AstCursor::from_source_tokens(&canonical_owner, canonical_range)
        .expect("test token stream must expose an AST cursor");
    cursor
        .set_position(file_tokens.opener_index + 1)
        .expect("test token stream position must remain in canonical range");
    let result = parse_optional_slot_target_for_test(
        directive_name,
        &mut cursor,
        file_tokens.source_path,
        &mut string_table,
        &mut path_fork,
    );
    assert_eq!(
        result.unwrap_or_else(|_| panic!("positional slot target should parse successfully")),
        SlotKey::Positional(1)
    );
}

#[test]
fn optional_slot_target_zero_errors() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let file_tokens = directive_tokens(
        "[$slot(0)]",
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    );
    let directive_name = string_table.intern("slot");
    let canonical_owner = file_tokens
        .canonical_owner()
        .expect("test token stream must expose canonical source tokens");
    let canonical_range = canonical_owner
        .full_range()
        .expect("test token stream must expose canonical source range");
    let mut cursor = AstCursor::from_source_tokens(&canonical_owner, canonical_range)
        .expect("test token stream must expose an AST cursor");
    cursor
        .set_position(file_tokens.opener_index + 1)
        .expect("test token stream position must remain in canonical range");
    let result = parse_optional_slot_target_for_test(
        directive_name,
        &mut cursor,
        file_tokens.source_path,
        &mut string_table,
        &mut path_fork,
    );
    assert!(result.is_err());
    assert_invalid_template_directive(
        &result.unwrap_err(),
        InvalidTemplateDirectiveCode::InvalidSlotTarget,
    );
}

#[test]
fn optional_slot_target_invalid_symbol_retains_exact_multibyte_span() {
    let source = "[$slot(π)]";
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let file_tokens =
        directive_tokens(source, &mut string_table, &mut span_builder, &mut path_fork);
    let directive_name = string_table.intern("slot");
    let canonical_owner = file_tokens
        .canonical_owner()
        .expect("test token stream must expose canonical source tokens");
    let canonical_range = canonical_owner
        .full_range()
        .expect("test token stream must expose canonical source range");
    let mut cursor = AstCursor::from_source_tokens(&canonical_owner, canonical_range)
        .expect("test token stream must expose an AST cursor");
    cursor
        .set_position(file_tokens.opener_index + 1)
        .expect("test token stream position must remain in canonical range");
    let diagnostic = parse_optional_slot_target_for_test(
        directive_name,
        &mut cursor,
        file_tokens.source_path,
        &mut string_table,
        &mut path_fork,
    )
    .expect_err("a symbol is not a valid slot target");
    let first = diagnostic
        .first()
        .expect("slot target diagnostic should compact to one record");
    let DirectiveStyleTestSite::Local(primary_span) = first.primary_site else {
        panic!("slot target diagnostic should retain its local offending-token span");
    };
    assert_eq!(primary_span.source(), SourceId::COMPILATION_ROOT);
    let range = primary_span.resolve_with(span_builder.resolver_for(SourceId::COMPILATION_ROOT));
    assert_eq!((range.start(), range.end()), (7, 9));
    assert_eq!(&source[range.start() as usize..range.end() as usize], "π");
    assert_eq!(first.label_count, 0);
}

#[test]
fn optional_slot_target_negative_errors() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let file_tokens = directive_tokens(
        "[$slot(-1)]",
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    );
    let directive_name = string_table.intern("slot");
    let canonical_owner = file_tokens
        .canonical_owner()
        .expect("test token stream must expose canonical source tokens");
    let canonical_range = canonical_owner
        .full_range()
        .expect("test token stream must expose canonical source range");
    let mut cursor = AstCursor::from_source_tokens(&canonical_owner, canonical_range)
        .expect("test token stream must expose an AST cursor");
    cursor
        .set_position(file_tokens.opener_index + 1)
        .expect("test token stream position must remain in canonical range");
    let result = parse_optional_slot_target_for_test(
        directive_name,
        &mut cursor,
        file_tokens.source_path,
        &mut string_table,
        &mut path_fork,
    );
    assert!(result.is_err());
}

#[test]
fn optional_slot_target_empty_parens_errors() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let file_tokens = directive_tokens(
        "[$slot()]",
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    );
    let directive_name = string_table.intern("slot");
    let canonical_owner = file_tokens
        .canonical_owner()
        .expect("test token stream must expose canonical source tokens");
    let canonical_range = canonical_owner
        .full_range()
        .expect("test token stream must expose canonical source range");
    let mut cursor = AstCursor::from_source_tokens(&canonical_owner, canonical_range)
        .expect("test token stream must expose an AST cursor");
    cursor
        .set_position(file_tokens.opener_index + 1)
        .expect("test token stream position must remain in canonical range");
    let result = parse_optional_slot_target_for_test(
        directive_name,
        &mut cursor,
        file_tokens.source_path,
        &mut string_table,
        &mut path_fork,
    );
    assert!(result.is_err());
    assert_invalid_template_directive(
        &result.unwrap_err(),
        InvalidTemplateDirectiveCode::EmptyArguments,
    );
}

#[test]
fn optional_slot_target_missing_close_paren_errors() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let file_tokens = directive_tokens(
        "[$slot(\"style\"]",
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    );
    let directive_name = string_table.intern("slot");
    let canonical_owner = file_tokens
        .canonical_owner()
        .expect("test token stream must expose canonical source tokens");
    let canonical_range = canonical_owner
        .full_range()
        .expect("test token stream must expose canonical source range");
    let mut cursor = AstCursor::from_source_tokens(&canonical_owner, canonical_range)
        .expect("test token stream must expose an AST cursor");
    cursor
        .set_position(file_tokens.opener_index + 1)
        .expect("test token stream position must remain in canonical range");
    let result = parse_optional_slot_target_for_test(
        directive_name,
        &mut cursor,
        file_tokens.source_path,
        &mut string_table,
        &mut path_fork,
    );
    assert!(result.is_err());
    assert_unexpected_token(
        &result.unwrap_err(),
        DiagnosticToken::from_static_tag(TokenTag::TEMPLATE_CLOSE),
    );
}

// ------------------------------------------------------------------------
// parse_required_slot_name_argument
// ------------------------------------------------------------------------

#[test]
fn required_slot_name_missing_parens_errors() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let file_tokens = directive_tokens(
        "[$insert]",
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    );
    let directive_name = string_table.intern("insert");
    let canonical_owner = file_tokens
        .canonical_owner()
        .expect("test token stream must expose canonical source tokens");
    let canonical_range = canonical_owner
        .full_range()
        .expect("test token stream must expose canonical source range");
    let mut cursor = AstCursor::from_source_tokens(&canonical_owner, canonical_range)
        .expect("test token stream must expose an AST cursor");
    cursor
        .set_position(file_tokens.opener_index + 1)
        .expect("test token stream position must remain in canonical range");
    let result = parse_required_slot_name_for_test(
        directive_name,
        &mut cursor,
        file_tokens.source_path,
        &mut string_table,
        &mut path_fork,
    );
    assert!(result.is_err());
    assert_expected_token(
        &result.unwrap_err(),
        DiagnosticToken::from_static_tag(TokenTag::OPEN_PARENTHESIS),
    );
}

#[test]
fn required_slot_name_string_literal_ok() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let file_tokens = directive_tokens(
        "[$insert(\"style\")]",
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    );
    let directive_name = string_table.intern("insert");
    let canonical_owner = file_tokens
        .canonical_owner()
        .expect("test token stream must expose canonical source tokens");
    let canonical_range = canonical_owner
        .full_range()
        .expect("test token stream must expose canonical source range");
    let mut cursor = AstCursor::from_source_tokens(&canonical_owner, canonical_range)
        .expect("test token stream must expose an AST cursor");
    cursor
        .set_position(file_tokens.opener_index + 1)
        .expect("test token stream position must remain in canonical range");
    let result = parse_required_slot_name_for_test(
        directive_name,
        &mut cursor,
        file_tokens.source_path,
        &mut string_table,
        &mut path_fork,
    );
    assert!(result.is_ok());
}

#[test]
fn required_slot_name_positional_rejected() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let file_tokens = directive_tokens(
        "[$insert(1)]",
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    );
    let directive_name = string_table.intern("insert");
    let canonical_owner = file_tokens
        .canonical_owner()
        .expect("test token stream must expose canonical source tokens");
    let canonical_range = canonical_owner
        .full_range()
        .expect("test token stream must expose canonical source range");
    let mut cursor = AstCursor::from_source_tokens(&canonical_owner, canonical_range)
        .expect("test token stream must expose an AST cursor");
    cursor
        .set_position(file_tokens.opener_index + 1)
        .expect("test token stream position must remain in canonical range");
    let result = parse_required_slot_name_for_test(
        directive_name,
        &mut cursor,
        file_tokens.source_path,
        &mut string_table,
        &mut path_fork,
    );
    assert!(result.is_err());
    assert_invalid_template_directive(
        &result.unwrap_err(),
        InvalidTemplateDirectiveCode::InvalidInsertTarget,
    );
}

#[test]
fn required_slot_name_empty_parens_errors() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let file_tokens = directive_tokens(
        "[$insert()]",
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    );
    let directive_name = string_table.intern("insert");
    let canonical_owner = file_tokens
        .canonical_owner()
        .expect("test token stream must expose canonical source tokens");
    let canonical_range = canonical_owner
        .full_range()
        .expect("test token stream must expose canonical source range");
    let mut cursor = AstCursor::from_source_tokens(&canonical_owner, canonical_range)
        .expect("test token stream must expose an AST cursor");
    cursor
        .set_position(file_tokens.opener_index + 1)
        .expect("test token stream position must remain in canonical range");
    let result = parse_required_slot_name_for_test(
        directive_name,
        &mut cursor,
        file_tokens.source_path,
        &mut string_table,
        &mut path_fork,
    );
    assert!(result.is_err());
    assert_invalid_template_directive(
        &result.unwrap_err(),
        InvalidTemplateDirectiveCode::EmptyArguments,
    );
}

#[test]
fn optional_string_literal_not_a_string_errors() {
    // After the $code refactor into the HTML project builder, argument type
    // validation lives in normalize_provided_style_argument_value (called by
    // apply_handler_style_directive), not in parse_optional_parenthesized_expression.
    let error = template_parse_error("[$code(42): body]");
    assert!(!error.is_empty());
}

// ------------------------------------------------------------------------
// parse_optional_parenthesized_expression
// ------------------------------------------------------------------------

#[test]
fn optional_expression_no_parens_returns_none() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let file_tokens = directive_tokens(
        "[$css]",
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    );
    let context = test_context(file_tokens.source_path, &path_fork);
    let canonical_owner = file_tokens
        .canonical_owner()
        .expect("test token stream must expose canonical source tokens");
    let canonical_range = canonical_owner
        .full_range()
        .expect("test token stream must expose canonical source range");
    let mut cursor = AstCursor::from_source_tokens(&canonical_owner, canonical_range)
        .expect("test token stream must expose an AST cursor");
    cursor
        .set_position(file_tokens.opener_index + 1)
        .expect("test token stream position must remain in canonical range");
    let result = parse_optional_parenthesized_expression_for_test(
        &mut cursor,
        &context,
        &mut expression_builder,
        &mut const_values,
        &mut string_table,
        &mut path_fork,
    );
    assert!(matches!(result, Ok(None)));
}

#[test]
fn optional_expression_with_parens_and_one_trailing_comma_returns_some() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let file_tokens = directive_tokens(
        "[$children(\n\"wrap\"\n,)]",
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    );
    let context = test_context(file_tokens.source_path, &path_fork);
    let canonical_owner = file_tokens
        .canonical_owner()
        .expect("test token stream must expose canonical source tokens");
    let canonical_range = canonical_owner
        .full_range()
        .expect("test token stream must expose canonical source range");
    let mut cursor = AstCursor::from_source_tokens(&canonical_owner, canonical_range)
        .expect("test token stream must expose an AST cursor");
    cursor
        .set_position(file_tokens.opener_index + 1)
        .expect("test token stream position must remain in canonical range");
    let result = parse_optional_parenthesized_expression_for_test(
        &mut cursor,
        &context,
        &mut expression_builder,
        &mut const_values,
        &mut string_table,
        &mut path_fork,
    );
    assert!(matches!(result, Ok(Some(_))));
    assert_eq!(
        cursor.current_tag(),
        TokenTag::TEMPLATE_CLOSE,
        "shared directive parser must leave the cursor after its closing parenthesis",
    );
}

#[test]
fn optional_expression_empty_parens_errors() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let file_tokens = directive_tokens(
        "[$code()]",
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    );
    let context = test_context(file_tokens.source_path, &path_fork);
    let canonical_owner = file_tokens
        .canonical_owner()
        .expect("test token stream must expose canonical source tokens");
    let canonical_range = canonical_owner
        .full_range()
        .expect("test token stream must expose canonical source range");
    let mut cursor = AstCursor::from_source_tokens(&canonical_owner, canonical_range)
        .expect("test token stream must expose an AST cursor");
    cursor
        .set_position(file_tokens.opener_index + 1)
        .expect("test token stream position must remain in canonical range");
    let result = parse_optional_parenthesized_expression_for_test(
        &mut cursor,
        &context,
        &mut expression_builder,
        &mut const_values,
        &mut string_table,
        &mut path_fork,
    );
    assert!(result.is_err());
}

#[test]
fn optional_expression_whitespace_only_parens_use_generic_empty_arguments() {
    // A whitespace-only argument stays on the generic empty-arguments owner
    // rather than the `$children`-specific reason, proving non-`$children`
    // directives do not receive children wording.
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let file_tokens = directive_tokens(
        "[$code(\n)]",
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    );
    let context = test_context(file_tokens.source_path, &path_fork);
    let canonical_owner = file_tokens
        .canonical_owner()
        .expect("test token stream must expose canonical source tokens");
    let canonical_range = canonical_owner
        .full_range()
        .expect("test token stream must expose canonical source range");
    let mut cursor = AstCursor::from_source_tokens(&canonical_owner, canonical_range)
        .expect("test token stream must expose an AST cursor");
    cursor
        .set_position(file_tokens.opener_index + 1)
        .expect("test token stream position must remain in canonical range");
    let result = parse_optional_parenthesized_expression_for_test(
        &mut cursor,
        &context,
        &mut expression_builder,
        &mut const_values,
        &mut string_table,
        &mut path_fork,
    );
    let diagnostic = result.expect_err("whitespace-only directive argument should error");
    assert_invalid_template_directive(&diagnostic, InvalidTemplateDirectiveCode::EmptyArguments);
}

#[test]
fn optional_expression_extra_comma_errors() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let source = "[$children(\"a\", \"b\")]";
    let file_tokens =
        directive_tokens(source, &mut string_table, &mut span_builder, &mut path_fork);
    let context = test_context(file_tokens.source_path, &path_fork);
    let canonical_owner = file_tokens
        .canonical_owner()
        .expect("test token stream must expose canonical source tokens");
    let canonical_range = canonical_owner
        .full_range()
        .expect("test token stream must expose canonical source range");
    let mut cursor = AstCursor::from_source_tokens(&canonical_owner, canonical_range)
        .expect("test token stream must expose an AST cursor");
    cursor
        .set_position(file_tokens.opener_index + 1)
        .expect("test token stream position must remain in canonical range");
    let result = parse_optional_parenthesized_expression_for_test(
        &mut cursor,
        &context,
        &mut expression_builder,
        &mut const_values,
        &mut string_table,
        &mut path_fork,
    );
    assert!(result.is_err());
    let diagnostic = result.unwrap_err();
    assert_unexpected_token(
        &diagnostic,
        DiagnosticToken::from_static_tag(TokenTag::COMMA),
    );
    let first = diagnostic
        .first()
        .expect("extra-argument diagnostic should compact to one record");
    let DirectiveStyleTestSite::Local(primary_span) = first.primary_site else {
        panic!("extra-argument diagnostic should retain its local offending-comma span");
    };
    let range = primary_span.resolve_with(span_builder.resolver_for(SourceId::COMPILATION_ROOT));
    assert_eq!((range.start(), range.end()), (14, 15));
    assert_eq!(&source[range.start() as usize..range.end() as usize], ",");
}

#[test]
fn optional_expression_named_argument_is_rejected() {
    let diagnostic = parse_optional_expression_source("[$children(value = 1)]")
        .expect_err("directive expressions must reject named arguments");

    let facts = diagnostic
        .first()
        .expect("named-argument diagnostic should compact to one record");
    assert!(matches!(
        &facts.kind,
        DirectiveStyleTestDiagnosticKind::InvalidCallShape(reason)
            if *reason == InvalidCallShapeCode::NamedArgumentsNotSupported
    ));
}

#[test]
fn optional_expression_mutable_argument_is_rejected() {
    let diagnostic = parse_optional_expression_source("[$children(~1)]")
        .expect_err("directive expressions must reject mutable arguments");

    let facts = diagnostic
        .first()
        .expect("mutable argument diagnostic should compact to one record");
    match &facts.kind {
        DirectiveStyleTestDiagnosticKind::InvalidTemplateDirective { reason, detail } => {
            assert_eq!(*reason, InvalidTemplateDirectiveCode::InvalidArgument);
            assert_eq!(*detail, None);
        }
        other => panic!("expected invalid-template-directive facts, found {other:?}"),
    }
}

#[test]
fn optional_expression_double_comma_errors() {
    let diagnostic = parse_optional_expression_source("[$children(\"a\",,)]")
        .expect_err("directive expressions must reject double commas");

    assert_unexpected_token(
        &diagnostic,
        DiagnosticToken::from_static_tag(TokenTag::COMMA),
    );
}

#[test]
fn optional_expression_missing_close_paren_errors() {
    let diagnostic = parse_optional_expression_source("[$children(\"a\"]")
        .expect_err("directive expressions must require a closing parenthesis");

    assert_unexpected_token(
        &diagnostic,
        DiagnosticToken::from_static_tag(TokenTag::TEMPLATE_CLOSE),
    );
}
#[test]
fn required_expression_missing_parens_errors() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let file_tokens = directive_tokens(
        "[$children]",
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    );
    let context = test_context(file_tokens.source_path, &path_fork);
    let canonical_owner = file_tokens
        .canonical_owner()
        .expect("test token stream must expose canonical source tokens");
    let canonical_range = canonical_owner
        .full_range()
        .expect("test token stream must expose canonical source range");
    let mut cursor = AstCursor::from_source_tokens(&canonical_owner, canonical_range)
        .expect("test token stream must expose an AST cursor");
    cursor
        .set_position(file_tokens.opener_index + 1)
        .expect("test token stream position must remain in canonical range");
    let result = parse_required_parenthesized_expression_for_test(
        &mut cursor,
        &context,
        &mut expression_builder,
        &mut const_values,
        &mut string_table,
        &mut path_fork,
    );
    assert!(result.is_err());
    let diagnostic = result.unwrap_err();
    assert_expected_token(
        &diagnostic,
        DiagnosticToken::from_static_tag(TokenTag::OPEN_PARENTHESIS),
    );
}

#[test]
fn required_expression_compile_time_constant_ok() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let file_tokens = directive_tokens(
        "[$children(\"wrap\")]",
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    );
    let context = test_context(file_tokens.source_path, &path_fork);
    let canonical_owner = file_tokens
        .canonical_owner()
        .expect("test token stream must expose canonical source tokens");
    let canonical_range = canonical_owner
        .full_range()
        .expect("test token stream must expose canonical source range");
    let mut cursor = AstCursor::from_source_tokens(&canonical_owner, canonical_range)
        .expect("test token stream must expose an AST cursor");
    cursor
        .set_position(file_tokens.opener_index + 1)
        .expect("test token stream position must remain in canonical range");
    let result = parse_required_parenthesized_expression_for_test(
        &mut cursor,
        &context,
        &mut expression_builder,
        &mut const_values,
        &mut string_table,
        &mut path_fork,
    );
    assert!(result.is_ok());
    let expr = result.unwrap_or_else(|_| panic!("required expression should parse successfully"));
    let view = expression_builder
        .view(expr)
        .expect("directive expression should be owned by the test builder");
    assert!(matches!(
        view.operation(),
        ExpressionOperationView::Constant(id) if const_values.string_value(id).is_some()
    ));
}

#[test]
fn note_and_todo_templates_do_not_render_content() {
    let note_rendered = folded_template_output("[:before[$note:ignored]after]");
    let todo_rendered = folded_template_output("[:before[$todo:ignored]after]");

    assert_eq!(note_rendered, "beforeafter");
    assert_eq!(todo_rendered, "beforeafter");
}

#[test]
fn note_and_todo_directives_reject_arguments() {
    let note_error = template_parse_error("[$note(\"x\"): ignored]");
    let todo_error = template_parse_error("[$todo(\"x\"): ignored]");

    assert!(!note_error.is_empty());
    assert!(!todo_error.is_empty());
}

#[test]
fn slot_directive_must_be_alone_in_template_head() {
    let before_error = template_parse_error("[\"prefix\", $slot]");
    let after_error = template_parse_error("[$slot, \"suffix\"]");

    assert!(before_error.contains("incompatible"));
    assert!(after_error.contains("incompatible"));
}

#[test]
fn comment_directives_must_be_alone_in_template_head() {
    for directive in ["note", "todo", "doc"] {
        let before = format!("[\"prefix\", ${directive}]");
        let after = format!("[${directive}, \"suffix\"]");
        let before_error = template_parse_error(&before);
        let after_error = template_parse_error(&after);

        assert!(
            before_error.contains("incompatible"),
            "expected mixed-head compatibility error for '{before}', got: {before_error}"
        );
        assert!(
            after_error.contains("incompatible"),
            "expected mixed-head compatibility error for '{after}', got: {after_error}"
        );
    }
}

#[test]
fn insert_directive_can_coexist_with_other_meaningful_head_items() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let file_tokens = template_tokens_from_source(
        "[\"prefix\", $insert(\"style\"): body]",
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    );
    let source_path = file_tokens.source_path;
    let context = new_constant_context(source_path, &path_fork);
    let canonical_owner = file_tokens
        .canonical_owner()
        .expect("test token stream must expose canonical source tokens");
    let canonical_range = canonical_owner
        .full_range()
        .expect("test token stream must expose canonical source range");
    let mut token_stream = AstCursor::from_source_tokens(&canonical_owner, canonical_range)
        .expect("test token stream must expose an AST cursor");
    token_stream
        .set_position(file_tokens.opener_index)
        .expect("test token stream position must remain in canonical range");
    let template = Template::new(
        &mut token_stream,
        source_path,
        &context,
        &mut expression_builder,
        &mut const_values,
        vec![],
        &mut string_table,
        &mut path_fork,
    )
    .expect("insert directive should coexist with other meaningful head items");

    assert!(matches!(
        effective_tir_kind(&template, &context),
        TemplateType::SlotInsert(_)
    ));
}

#[test]
fn duplicate_insert_directives_are_rejected_in_one_head() {
    let error = template_parse_error("[$insert(\"a\"), $insert(\"b\"):]");
    assert!(error.contains("incompatible"));
}

#[test]
fn formatter_directives_are_exclusive_per_template_head() {
    let error = template_parse_error("[$md, $raw: body]");
    assert!(error.contains("incompatible"));
}

#[test]
fn project_owned_formatter_directives_are_exclusive_per_template_head() {
    let style_directives = html_project_test_style_directives();
    let error =
        template_parse_error_with_style_directives("[$html, $css: body]", &style_directives);
    assert!(error.contains("incompatible"));
}

#[test]
fn non_formatter_and_formatter_directives_can_coexist() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let file_tokens = template_tokens_from_source(
        "[1, $md:\n# Hello\n]",
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    );
    let source_path = file_tokens.source_path;
    let context = new_constant_context(source_path, &path_fork);
    let canonical_owner = file_tokens
        .canonical_owner()
        .expect("test token stream must expose canonical source tokens");
    let canonical_range = canonical_owner
        .full_range()
        .expect("test token stream must expose canonical source range");
    let mut token_stream = AstCursor::from_source_tokens(&canonical_owner, canonical_range)
        .expect("test token stream must expose an AST cursor");
    token_stream
        .set_position(file_tokens.opener_index)
        .expect("test token stream position must remain in canonical range");
    let template = Template::new(
        &mut token_stream,
        source_path,
        &context,
        &mut expression_builder,
        &mut const_values,
        vec![],
        &mut string_table,
        &mut path_fork,
    )
    .expect("non-formatter and formatter directives should coexist in the same head");

    assert_eq!(effective_tir_style(&template, &context).id, "markdown");
}

#[test]
fn doc_templates_treat_brackets_as_literal_text() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let file_tokens = template_tokens_from_source(
        "[$doc:\n[value]\n]",
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    );
    let source_path = file_tokens.source_path;
    let context = runtime_template_context(
        &source_path,
        &mut expression_builder,
        &mut const_values,
        &mut string_table,
        &mut path_fork,
    );
    let canonical_owner = file_tokens
        .canonical_owner()
        .expect("test token stream must expose canonical source tokens");
    let canonical_range = canonical_owner
        .full_range()
        .expect("test token stream must expose canonical source range");
    let mut token_stream = AstCursor::from_source_tokens(&canonical_owner, canonical_range)
        .expect("test token stream must expose an AST cursor");
    token_stream
        .set_position(file_tokens.opener_index)
        .expect("test token stream position must remain in canonical range");
    // With suppress_child_templates, brackets are balanced literal text,
    // not nested child templates. Parsing succeeds even with a runtime context.
    let template = Template::new(
        &mut token_stream,
        source_path,
        &context,
        &mut expression_builder,
        &mut const_values,
        vec![],
        &mut string_table,
        &mut path_fork,
    )
    .expect("doc template should parse brackets as literal text");

    let folded = fold_template_in_context(
        &template,
        &context,
        &mut string_table,
        &mut expression_builder,
        &mut const_values,
    );
    let result = string_table.resolve(folded);
    // Each bracket and the symbol become separate atoms, each markdown-formatted
    // as individual paragraphs. The key assertion is that all three appear in the output.
    assert!(
        result.contains("["),
        "Opening bracket should appear as literal text: {result}"
    );
    assert!(
        result.contains("value"),
        "Inner symbol should appear as literal text: {result}"
    );
    assert!(
        result.contains("]"),
        "Closing bracket should appear as literal text: {result}"
    );
}

#[test]
fn doc_templates_are_markdown_formatted_by_default() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let file_tokens = template_tokens_from_source(
        "[$doc:\n# Heading\n]",
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    );
    let source_path = file_tokens.source_path;
    let context = new_constant_context(source_path, &path_fork);
    let canonical_owner = file_tokens
        .canonical_owner()
        .expect("test token stream must expose canonical source tokens");
    let canonical_range = canonical_owner
        .full_range()
        .expect("test token stream must expose canonical source range");
    let mut token_stream = AstCursor::from_source_tokens(&canonical_owner, canonical_range)
        .expect("test token stream must expose an AST cursor");
    token_stream
        .set_position(file_tokens.opener_index)
        .expect("test token stream position must remain in canonical range");
    let template = Template::new(
        &mut token_stream,
        source_path,
        &context,
        &mut expression_builder,
        &mut const_values,
        vec![],
        &mut string_table,
        &mut path_fork,
    )
    .expect("doc template should parse");
    assert!(matches!(
        effective_tir_kind(&template, &context),
        TemplateType::Comment(CommentDirectiveKind::Doc)
    ));

    let folded = fold_template_in_context(
        &template,
        &context,
        &mut string_table,
        &mut expression_builder,
        &mut const_values,
    );
    assert!(
        string_table
            .resolve(folded)
            .contains("<h1 id=\"heading\">Heading</h1>")
    );
}

#[test]
fn doc_brackets_remain_literal_text() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let file_tokens = template_tokens_from_source(
        "[$doc:\n[else][break][continue]\n]",
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    );
    let source_path = file_tokens.source_path;
    let context = new_constant_context(source_path, &path_fork);
    let canonical_owner = file_tokens
        .canonical_owner()
        .expect("test token stream must expose canonical source tokens");
    let canonical_range = canonical_owner
        .full_range()
        .expect("test token stream must expose canonical source range");
    let mut token_stream = AstCursor::from_source_tokens(&canonical_owner, canonical_range)
        .expect("test token stream must expose an AST cursor");
    token_stream
        .set_position(file_tokens.opener_index)
        .expect("test token stream position must remain in canonical range");
    let template = Template::new(
        &mut token_stream,
        source_path,
        &context,
        &mut expression_builder,
        &mut const_values,
        vec![],
        &mut string_table,
        &mut path_fork,
    )
    .expect("doc template should parse");

    let folded = fold_template_in_context(
        &template,
        &context,
        &mut string_table,
        &mut expression_builder,
        &mut const_values,
    );
    let result = string_table.resolve(folded);
    // Direct source markers remain intact when `$doc` suppresses child-template parsing.
    for marker in ["[else]", "[break]", "[continue]"] {
        assert!(
            result.contains(marker),
            "suppressed body should preserve literal marker {marker:?}: {result}"
        );
    }
}

#[test]
fn css_without_argument_uses_css_formatter() {
    let style_directives = html_project_test_style_directives();
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let file_tokens = template_tokens_from_source_with_style_directives(
        "[$css:\n.button { color: red; }\n]",
        &style_directives,
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    );
    let source_path = file_tokens.source_path;
    let context =
        new_constant_context_with_style_directives(source_path, &style_directives, &path_fork);
    let canonical_owner = file_tokens
        .canonical_owner()
        .expect("test token stream must expose canonical source tokens");
    let canonical_range = canonical_owner
        .full_range()
        .expect("test token stream must expose canonical source range");
    let mut token_stream = AstCursor::from_source_tokens(&canonical_owner, canonical_range)
        .expect("test token stream must expose an AST cursor");
    token_stream
        .set_position(file_tokens.opener_index)
        .expect("test token stream position must remain in canonical range");
    let template = Template::new(
        &mut token_stream,
        source_path,
        &context,
        &mut expression_builder,
        &mut const_values,
        vec![],
        &mut string_table,
        &mut path_fork,
    )
    .expect("css template should parse");

    let effective_style = effective_tir_style(&template, &context);
    assert_eq!(effective_style.id, "css");
    assert!(effective_style.formatter.is_some());
}

#[test]
fn css_inline_argument_parses_correctly() {
    let style_directives = html_project_test_style_directives();
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let file_tokens = template_tokens_from_source_with_style_directives(
        "[$css(\"inline\"):\ncolor: blue;\n]",
        &style_directives,
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    );
    let source_path = file_tokens.source_path;
    let context =
        new_constant_context_with_style_directives(source_path, &style_directives, &path_fork);
    let canonical_owner = file_tokens
        .canonical_owner()
        .expect("test token stream must expose canonical source tokens");
    let canonical_range = canonical_owner
        .full_range()
        .expect("test token stream must expose canonical source range");
    let mut token_stream = AstCursor::from_source_tokens(&canonical_owner, canonical_range)
        .expect("test token stream must expose an AST cursor");
    token_stream
        .set_position(file_tokens.opener_index)
        .expect("test token stream position must remain in canonical range");
    let template = Template::new(
        &mut token_stream,
        source_path,
        &context,
        &mut expression_builder,
        &mut const_values,
        vec![],
        &mut string_table,
        &mut path_fork,
    )
    .expect("inline css template should parse");

    let effective_style = effective_tir_style(&template, &context);
    assert_eq!(effective_style.id, "css");
    assert!(effective_style.formatter.is_some());
}

#[test]
fn css_inline_argument_must_be_quoted_string_literal() {
    let style_directives = html_project_test_style_directives();
    let error = template_parse_error_with_style_directives(
        "[$css(inline): color: blue;]",
        &style_directives,
    );
    assert!(
        error.contains("compile-time string argument")
            || error.contains("not declared")
            || error.contains("Undefined variable")
            || error.contains("Unknown value name")
            || error.contains("Unknown Value name"),
        "unexpected error message: {error}"
    );
}

#[test]
fn css_rejects_unknown_arguments() {
    let style_directives = html_project_test_style_directives();
    let error = template_parse_error_with_style_directives(
        "[$css(\"scoped\"): color: blue;]",
        &style_directives,
    );
    assert!(!error.is_empty());
}

#[test]
fn const_css_template_emits_malformed_css_warnings() {
    let style_directives = html_project_test_style_directives();
    let warnings = template_warnings_with_style_directives(
        "[$css:\n.button { color red; }\n]",
        false,
        &style_directives,
    );
    let warnings = compact_bag_for_test(DiagnosticBag::from_diagnostics(warnings), None);

    assert!(!warnings.is_empty());
    assert!(
        warnings
            .views()
            .all(|warning| { warning.family_view::<MalformedCssTemplateView>().is_some() })
    );
    assert!(
        warnings
            .views()
            .any(|warning| { warning.family_view::<MalformedCssTemplateView>().is_some() })
    );
}

#[test]
fn css_validation_warnings_keep_authored_spans() {
    let style_directives = html_project_test_style_directives();
    let warnings = template_warnings_with_style_directives(
        "[$css:\n.button { color red; }\n.button { color blue }\n]",
        false,
        &style_directives,
    );
    let warnings = compact_bag_for_test(DiagnosticBag::from_diagnostics(warnings), None);
    let expected_span = SourceSpan::new(
        SourceId::COMPILATION_ROOT,
        LocalSpan::exact(6, 47, &mut ExtendedSpanBuilder::new())
            .expect("CSS body literal span should fit"),
    );

    assert!(!warnings.is_empty());
    assert!(
        warnings.views().all(|warning| {
            warning.family_view::<MalformedCssTemplateView>().is_some()
                && matches!(warning.primary_site(), SiteRef::Local(span) if span == expected_span)
        }),
        "css warnings should keep their exact authored source span: {:?}",
        warnings
            .views()
            .map(|warning| warning.primary_site())
            .collect::<Vec<_>>(),
    );
}

#[test]
fn inline_css_warns_when_blocks_are_used() {
    let style_directives = html_project_test_style_directives();
    let warnings = template_warnings_with_style_directives(
        "[$css(\"inline\"):\n.button { color: red; }\n]",
        false,
        &style_directives,
    );
    let warnings = compact_bag_for_test(DiagnosticBag::from_diagnostics(warnings), None);

    assert!(
        warnings
            .views()
            .any(|warning| { warning.family_view::<MalformedCssTemplateView>().is_some() })
    );
}

#[test]
fn runtime_css_templates_emit_warnings_for_static_body_segments() {
    let style_directives = html_project_test_style_directives();
    let warnings = template_warnings_with_style_directives(
        "[value, $css:\n.button { color red; }\n]",
        true,
        &style_directives,
    );
    let warnings = compact_bag_for_test(DiagnosticBag::from_diagnostics(warnings), None);
    assert!(!warnings.is_empty());
    assert!(
        warnings
            .views()
            .all(|warning| { warning.family_view::<MalformedCssTemplateView>().is_some() })
    );
}

#[test]
fn code_without_argument_uses_generic_highlighting() {
    let rendered = folded_template_output("[$code:\nloop(x + 1)\n]");

    assert!(rendered.contains("<code class='codeblock'>"));
    assert!(rendered.contains("<span class='moth-code-delimiter'>(</span>"));
    assert!(!rendered.contains("moth-code-keyword"));
}

#[test]
fn code_moth_argument_highlights_moth_rules() {
    let rendered = folded_template_output("[$code(\"moth\"):\nloop x\n-- hi\n]");

    assert!(rendered.contains("<span class='moth-code-keyword'>loop</span>"));
    assert!(rendered.contains("<span class='moth-code-comment'>-- hi</span>"));
}

#[test]
fn code_javascript_argument_highlights_js_comments() {
    let rendered = folded_template_output("[$code(\"js\"):\nconst x = 1\n// hi\n]");

    assert!(rendered.contains("<span class='moth-code-keyword'>const</span>"));
    assert!(rendered.contains("<span class='moth-code-comment'>// hi</span>"));
}

#[test]
fn code_python_argument_highlights_python_comments() {
    let rendered = folded_template_output("[$code(\"py\"):\ndef run():\n# hi\n]");

    assert!(rendered.contains("<span class='moth-code-keyword'>def</span>"));
    assert!(rendered.contains("<span class='moth-code-comment'># hi</span>"));
}

#[test]
fn code_typescript_argument_highlights_typescript_types() {
    let rendered = folded_template_output("[$code(\"ts\"):\ntype Name = string\n]");

    assert!(rendered.contains("<span class='moth-code-keyword'>type</span>"));
    assert!(rendered.contains("<span class='moth-code-type'>string</span>"));
}

#[test]
fn code_empty_parentheses_error_cleanly() {
    let error = template_parse_error("[$code(): body]");

    assert!(
        error.contains("empty parentheses") || error.contains("EmptyArguments"),
        "unexpected error message: {error}"
    );
}

#[test]
fn code_rejects_unknown_language_aliases() {
    let error = template_parse_error("[$code(\"unknown\"): body]");

    assert!(!error.is_empty());
}

#[test]
fn code_rejects_multiple_language_arguments() {
    let error = template_parse_error("[$code(\"moth\", \"js\"): body]");

    assert!(
        error.contains("Unexpected token `,`."),
        "unexpected error message: {error}"
    );
}

#[test]
fn runtime_templates_with_code_format_only_static_body_strings() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let file_tokens = template_tokens_from_source(
        "[value, $code(\"moth\"):\nloop x\n]",
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    );
    let source_path = file_tokens.source_path;
    let context = runtime_template_context(
        &source_path,
        &mut expression_builder,
        &mut const_values,
        &mut string_table,
        &mut path_fork,
    );
    let canonical_owner = file_tokens
        .canonical_owner()
        .expect("test token stream must expose canonical source tokens");
    let canonical_range = canonical_owner
        .full_range()
        .expect("test token stream must expose canonical source range");
    let mut token_stream = AstCursor::from_source_tokens(&canonical_owner, canonical_range)
        .expect("test token stream must expose an AST cursor");
    token_stream
        .set_position(file_tokens.opener_index)
        .expect("test token stream position must remain in canonical range");
    let template = Template::new(
        &mut token_stream,
        source_path,
        &context,
        &mut expression_builder,
        &mut const_values,
        vec![],
        &mut string_table,
        &mut path_fork,
    )
    .expect("template should parse");

    let store = context.template_ir_store.borrow();
    assert!(tir_root_has_head_dynamic_expression(
        &template,
        &store,
        &expression_builder,
        |expression| match expression.operation() {
            ExpressionOperationView::Reference(_) => true,
            ExpressionOperationView::Constant(id) => const_values.string_value(id).is_some(),
            _ => false,
        },
    ));

    // Formatted body text is authoritative in the TIR root.
    let body_texts = collect_body_text_from_tir(&template, &store, &string_table);
    assert!(
        body_texts
            .iter()
            .any(|text| text.contains("<code class='codeblock'>")),
        "expected code HTML block in formatted body text"
    );
    assert!(
        body_texts
            .iter()
            .any(|text| text.contains("<span class='moth-code-keyword'>loop</span>")),
        "expected highlighted keyword span in formatted body text"
    );
}

#[test]
fn code_templates_keep_nested_square_brackets_as_literal_body_text() {
    let rendered = folded_template_output(
        "[$code(\"moth\"):\nconcatenated_strings = [string_slice, a_mutable_string]\n]",
    );

    assert!(rendered.contains("<code class='codeblock'>"));
    assert!(rendered.contains("concatenated_strings"));
    assert!(rendered.contains("string_slice"));
    assert!(rendered.contains("a_mutable_string"));
    assert!(rendered.contains("<span class='moth-code-delimiter'>[</span>"));
    assert!(rendered.contains("<span class='moth-code-delimiter'>]</span>"));
}
