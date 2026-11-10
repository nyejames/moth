use super::*;
use crate::compiler_frontend::ast::const_values::store::{ConstStringValue, ConstValuePayload};
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::expression_store::{
    ExprId, ExpressionOperationView,
};
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::template::{
    CommentDirectiveKind, SlotDeclaration, SlotKey, TemplateType,
};
use crate::compiler_frontend::ast::templates::template_head_parser::core_directives::{
    parse_in_target, parse_slot_declaration,
};
use crate::compiler_frontend::ast::templates::template_head_parser::directive_args::{
    parse_optional_directive_operand, reject_directive_operand,
};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_messages::DiagnosticBag;
use crate::compiler_frontend::compiler_messages::diagnostics::families::calls_data::InvalidCallShapeView;
use crate::compiler_frontend::compiler_messages::diagnostics::families::templates::MalformedCssTemplateView;
use crate::compiler_frontend::compiler_messages::diagnostics::families::templates::{
    InvalidTemplateDirectiveCode, InvalidTemplateDirectiveView,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::token_lexical::UnexpectedTokenView;
use crate::compiler_frontend::compiler_messages::diagnostics::foreign::SiteRef;
use crate::compiler_frontend::compiler_messages::diagnostics::test_support::CompactDiagnosticBagForTest;
use crate::compiler_frontend::compiler_messages::diagnostics::test_support::compact_bag_for_test;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::folded_value::resolve_const_string_value;
use crate::compiler_frontend::source::{ExtendedSpanBuilder, LocalSpan, SourceId, SourceSpan};
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::synthetic_interface_provenance::{
    SyntheticInterfaceClass, SyntheticInterfaceMemberIdentity, SyntheticInterfaceProvenance,
};
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
    UnexpectedToken,
    InvalidCallShape,
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
                } else if view.family_view::<UnexpectedTokenView>().is_some() {
                    DirectiveStyleTestDiagnosticKind::UnexpectedToken
                } else if view.family_view::<InvalidCallShapeView>().is_some() {
                    DirectiveStyleTestDiagnosticKind::InvalidCallShape
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

fn parse_slot_declaration_for_test(
    directive_name: StringId,
    tokens: &mut AstCursor,
    scope: PathId,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> DirectiveStyleTestResult<SlotDeclaration> {
    parse_slot_declaration_and_constants_for_test(
        directive_name,
        tokens,
        scope,
        string_table,
        path_fork,
    )
    .map(|(declaration, _)| declaration)
}

fn parse_slot_declaration_and_constants_for_test(
    directive_name: StringId,
    tokens: &mut AstCursor,
    scope: PathId,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> DirectiveStyleTestResult<(SlotDeclaration, ConstValueStore)> {
    let context = test_context(scope, path_fork);
    let mut type_environment = TypeEnvironment::new();
    let mut compatibility_cache = TypeCompatibilityCache::new();
    let mut type_interner = AstTypeInterner::new(&mut type_environment, &mut compatibility_cache);
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let declaration = parse_slot_declaration(
        directive_name,
        tokens,
        &context,
        &mut type_interner,
        &mut expression_builder,
        &mut const_values,
        string_table,
        path_fork,
    )
    .map_err(directive_diagnostic)?;
    Ok((declaration, const_values))
}

fn parse_slot_declaration_with_shared_state_for_test(
    directive_name: StringId,
    tokens: &mut AstCursor,
    context: &ScopeContext,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> DirectiveStyleTestResult<SlotDeclaration> {
    let mut type_environment = TypeEnvironment::new();
    let mut compatibility_cache = TypeCompatibilityCache::new();
    let mut type_interner = AstTypeInterner::new(&mut type_environment, &mut compatibility_cache);
    parse_slot_declaration(
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

fn parse_in_target_for_test(
    directive_name: StringId,
    tokens: &mut AstCursor,
    string_table: &mut StringTable,
) -> DirectiveStyleTestResult<StringId> {
    parse_in_target(directive_name, tokens, string_table).map_err(directive_diagnostic)
}

fn parse_optional_directive_operand_for_test(
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
    parse_optional_directive_operand(
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

fn parse_optional_expression_source(source: &str) -> DirectiveStyleTestResult<Option<ExprId>> {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let file_tokens =
        directive_tokens(source, &mut string_table, &mut span_builder, &mut path_fork);
    let context = test_context(file_tokens.source_path, &path_fork);
    let canonical_owner = file_tokens.canonical_owner();
    let canonical_range = canonical_owner
        .full_range()
        .expect("test token stream must expose canonical source range");
    let mut cursor = AstCursor::from_source_tokens(&canonical_owner, canonical_range)
        .expect("test token stream must expose an AST cursor");
    cursor
        .set_position(file_tokens.opener_index + 1)
        .expect("test token stream position must remain in canonical range");
    parse_optional_directive_operand_for_test(
        &mut cursor,
        &context,
        &mut expression_builder,
        &mut const_values,
        &mut string_table,
        &mut path_fork,
    )
}

// ------------------------------------------------------------------------
// reject_directive_operand
// ------------------------------------------------------------------------

#[test]
fn reject_operand_succeeds_when_directive_is_bare() {
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
    let canonical_owner = file_tokens.canonical_owner();
    let canonical_range = canonical_owner
        .full_range()
        .expect("test token stream must expose canonical source range");
    let mut cursor = AstCursor::from_source_tokens(&canonical_owner, canonical_range)
        .expect("test token stream must expose an AST cursor");
    cursor
        .set_position(file_tokens.opener_index + 1)
        .expect("test token stream position must remain in canonical range");
    let result = reject_directive_operand(directive_name, &cursor);
    assert!(result.is_ok());
}

#[test]
fn reject_operand_fails_when_value_present() {
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
    let canonical_owner = file_tokens.canonical_owner();
    let canonical_range = canonical_owner
        .full_range()
        .expect("test token stream must expose canonical source range");
    let mut cursor = AstCursor::from_source_tokens(&canonical_owner, canonical_range)
        .expect("test token stream must expose an AST cursor");
    cursor
        .set_position(file_tokens.opener_index + 1)
        .expect("test token stream position must remain in canonical range");
    let result = reject_directive_operand(directive_name, &cursor);
    assert!(result.is_err());
    let diagnostic = directive_diagnostic(result.unwrap_err());
    assert_invalid_template_directive(
        &diagnostic,
        InvalidTemplateDirectiveCode::UnexpectedArguments,
    );
}

// ------------------------------------------------------------------------
// parse_slot_declaration_argument
// ------------------------------------------------------------------------

#[test]
fn slot_declaration_no_parens_returns_default() {
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
    let canonical_owner = file_tokens.canonical_owner();
    let canonical_range = canonical_owner
        .full_range()
        .expect("test token stream must expose canonical source range");
    let mut cursor = AstCursor::from_source_tokens(&canonical_owner, canonical_range)
        .expect("test token stream must expose an AST cursor");
    cursor
        .set_position(file_tokens.opener_index + 1)
        .expect("test token stream position must remain in canonical range");
    let result = parse_slot_declaration_for_test(
        directive_name,
        &mut cursor,
        file_tokens.source_path,
        &mut string_table,
        &mut path_fork,
    );
    let declaration =
        result.unwrap_or_else(|_| panic!("slot declaration should parse successfully"));
    assert_eq!(declaration.key, SlotKey::Default);
    assert_eq!(declaration.default, None);
}

#[test]
fn slot_declaration_named_target() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let file_tokens = directive_tokens(
        "[$slot style]",
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    );
    let directive_name = string_table.intern("slot");
    let canonical_owner = file_tokens.canonical_owner();
    let canonical_range = canonical_owner
        .full_range()
        .expect("test token stream must expose canonical source range");
    let mut cursor = AstCursor::from_source_tokens(&canonical_owner, canonical_range)
        .expect("test token stream must expose an AST cursor");
    cursor
        .set_position(file_tokens.opener_index + 1)
        .expect("test token stream position must remain in canonical range");
    let result = parse_slot_declaration_for_test(
        directive_name,
        &mut cursor,
        file_tokens.source_path,
        &mut string_table,
        &mut path_fork,
    );
    let declaration =
        result.unwrap_or_else(|_| panic!("named slot declaration should parse successfully"));
    assert!(matches!(declaration.key, SlotKey::Named(_)));
    assert_eq!(declaration.default, None);
}

#[test]
fn slot_declaration_positive_positional() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let file_tokens = directive_tokens(
        "[$slot 1]",
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    );
    let directive_name = string_table.intern("slot");
    let canonical_owner = file_tokens.canonical_owner();
    let canonical_range = canonical_owner
        .full_range()
        .expect("test token stream must expose canonical source range");
    let mut cursor = AstCursor::from_source_tokens(&canonical_owner, canonical_range)
        .expect("test token stream must expose an AST cursor");
    cursor
        .set_position(file_tokens.opener_index + 1)
        .expect("test token stream position must remain in canonical range");
    let result = parse_slot_declaration_for_test(
        directive_name,
        &mut cursor,
        file_tokens.source_path,
        &mut string_table,
        &mut path_fork,
    );
    let declaration =
        result.unwrap_or_else(|_| panic!("positional slot declaration should parse successfully"));
    assert_eq!(declaration.key, SlotKey::Positional(1));
    assert_eq!(declaration.default, None);
}

#[test]
fn slot_declaration_zero_errors() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let file_tokens = directive_tokens(
        "[$slot 0]",
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    );
    let directive_name = string_table.intern("slot");
    let canonical_owner = file_tokens.canonical_owner();
    let canonical_range = canonical_owner
        .full_range()
        .expect("test token stream must expose canonical source range");
    let mut cursor = AstCursor::from_source_tokens(&canonical_owner, canonical_range)
        .expect("test token stream must expose an AST cursor");
    cursor
        .set_position(file_tokens.opener_index + 1)
        .expect("test token stream position must remain in canonical range");
    let result = parse_slot_declaration_for_test(
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
fn slot_declaration_invalid_string_target_retains_exact_multibyte_span() {
    let source = "[$slot \"π\"]";
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let file_tokens =
        directive_tokens(source, &mut string_table, &mut span_builder, &mut path_fork);
    let directive_name = string_table.intern("slot");
    let canonical_owner = file_tokens.canonical_owner();
    let canonical_range = canonical_owner
        .full_range()
        .expect("test token stream must expose canonical source range");
    let mut cursor = AstCursor::from_source_tokens(&canonical_owner, canonical_range)
        .expect("test token stream must expose an AST cursor");
    cursor
        .set_position(file_tokens.opener_index + 1)
        .expect("test token stream position must remain in canonical range");
    let diagnostic = parse_slot_declaration_for_test(
        directive_name,
        &mut cursor,
        file_tokens.source_path,
        &mut string_table,
        &mut path_fork,
    )
    .expect_err("a string literal is not a valid slot target");
    let first = diagnostic
        .first()
        .expect("slot target diagnostic should compact to one record");
    let DirectiveStyleTestSite::Local(primary_span) = first.primary_site else {
        panic!("slot target diagnostic should retain its local offending-token span");
    };
    assert_eq!(primary_span.source(), SourceId::COMPILATION_ROOT);
    let range = primary_span.resolve_with(span_builder.resolver_for(SourceId::COMPILATION_ROOT));
    assert_eq!((range.start(), range.end()), (7, 11));
    assert_eq!(
        &source[range.start() as usize..range.end() as usize],
        "\"π\""
    );
    assert_eq!(first.label_count, 0);
}

#[test]
fn slot_declaration_negative_errors() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let file_tokens = directive_tokens(
        "[$slot -1]",
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    );
    let directive_name = string_table.intern("slot");
    let canonical_owner = file_tokens.canonical_owner();
    let canonical_range = canonical_owner
        .full_range()
        .expect("test token stream must expose canonical source range");
    let mut cursor = AstCursor::from_source_tokens(&canonical_owner, canonical_range)
        .expect("test token stream must expose an AST cursor");
    cursor
        .set_position(file_tokens.opener_index + 1)
        .expect("test token stream position must remain in canonical range");
    let result = parse_slot_declaration_for_test(
        directive_name,
        &mut cursor,
        file_tokens.source_path,
        &mut string_table,
        &mut path_fork,
    );
    assert!(result.is_err());
}

#[test]
fn slot_declaration_old_parenthesized_form_rejected() {
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
    let canonical_owner = file_tokens.canonical_owner();
    let canonical_range = canonical_owner
        .full_range()
        .expect("test token stream must expose canonical source range");
    let mut cursor = AstCursor::from_source_tokens(&canonical_owner, canonical_range)
        .expect("test token stream must expose an AST cursor");
    cursor
        .set_position(file_tokens.opener_index + 1)
        .expect("test token stream position must remain in canonical range");
    let result = parse_slot_declaration_for_test(
        directive_name,
        &mut cursor,
        file_tokens.source_path,
        &mut string_table,
        &mut path_fork,
    );
    assert!(result.is_err());
    assert_invalid_template_directive(
        &result.unwrap_err(),
        InvalidTemplateDirectiveCode::UnexpectedArguments,
    );
}

#[test]
fn slot_declaration_named_default_is_retained() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let file_tokens = directive_tokens(
        "[$slot title = \"Untitled\"]",
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    );
    let directive_name = string_table.intern("slot");
    let canonical_owner = file_tokens.canonical_owner();
    let canonical_range = canonical_owner
        .full_range()
        .expect("test token stream must expose canonical source range");
    let mut cursor = AstCursor::from_source_tokens(&canonical_owner, canonical_range)
        .expect("test token stream must expose an AST cursor");
    cursor
        .set_position(file_tokens.opener_index + 1)
        .expect("test token stream position must remain in canonical range");
    let (declaration, const_values) = parse_slot_declaration_and_constants_for_test(
        directive_name,
        &mut cursor,
        file_tokens.source_path,
        &mut string_table,
        &mut path_fork,
    )
    .unwrap_or_else(|_| panic!("slot default should parse successfully"));

    assert!(matches!(declaration.key, SlotKey::Named(_)));
    let default = declaration
        .default
        .expect("the named slot default should be retained");
    assert!(matches!(
        const_values.payload(default),
        Some(ConstValuePayload::String(ConstStringValue::Text(text)))
            if string_table.resolve(*text) == "Untitled"
    ));
}

#[test]
fn slot_declaration_folded_default_keeps_occurrence_provenance() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut expression_builder = AstExpressionBuilder::new().expect("test expression builder");
    let mut const_values = ConstValueStore::default();
    let file_tokens = directive_tokens(
        "[$slot title = [configured_label]]",
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    );
    let source_scope = file_tokens.source_path;

    let member = SyntheticInterfaceMemberIdentity::new(
        SyntheticInterfaceClass::ProjectContext,
        "source-config",
        "configured_label",
    );
    let provenance = SyntheticInterfaceProvenance::single(member.clone());
    let configured_label = expression_builder
        .string_slice(
            &mut const_values,
            string_table.intern("Configured label"),
            None,
            ValueMode::ImmutableOwned,
        )
        .expect("test default constant should construct");
    let configured_label = expression_builder
        .with_synthetic_interface_provenance(configured_label, provenance)
        .expect("test default constant should accept provenance")
        .id();
    let declaration_name = string_table.intern("configured_label");
    let declaration_path = path_fork
        .try_intern_child(source_scope, declaration_name)
        .expect("test path fits");
    let declarations = vec![make_test_variable(
        declaration_path,
        configured_label,
        &expression_builder,
    )];
    let style_directives = frontend_test_style_directives();
    let context = with_test_path_context(
        ScopeContext::new_for_tests(
            ContextKind::Constant,
            source_scope,
            Rc::new(TopLevelDeclarationTable::new(declarations, &path_fork)),
            Arc::new(ExternalPackageRegistry::default()),
            0,
        ),
        &source_scope,
        &style_directives,
    );

    let canonical_owner = file_tokens.canonical_owner();
    let canonical_range = canonical_owner
        .full_range()
        .expect("test token stream must expose canonical source range");
    let mut cursor = AstCursor::from_source_tokens(&canonical_owner, canonical_range)
        .expect("test token stream must expose an AST cursor");
    cursor
        .set_position(file_tokens.opener_index + 1)
        .expect("test token stream position must remain in canonical range");
    let directive_name = string_table.intern("slot");
    let declaration = parse_slot_declaration_with_shared_state_for_test(
        directive_name,
        &mut cursor,
        &context,
        &mut expression_builder,
        &mut const_values,
        &mut string_table,
        &mut path_fork,
    )
    .expect("folded slot default should parse");

    let default = declaration
        .default
        .expect("the folded slot default should be retained");
    let metadata = const_values
        .metadata(default)
        .expect("the folded default keeps content metadata");
    assert!(
        metadata.span.is_some(),
        "the staged default must retain the authored expression span"
    );
    assert_eq!(
        metadata.synthetic_interface_provenance.members(),
        std::slice::from_ref(&member),
        "the staged default must carry the folded occurrence provenance"
    );
    let (value, resolved_provenance) = resolve_const_string_value(&const_values, default)
        .expect("the folded default should resolve to its string");
    assert_eq!(
        value,
        &ConstStringValue::Text(string_table.intern("Configured label"))
    );
    assert_eq!(
        resolved_provenance.members(),
        &[member],
        "a folded default occurrence must keep its project dependency"
    );
}

#[test]
fn slot_declaration_non_string_default_rejected() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let file_tokens = directive_tokens(
        "[$slot title = 1]",
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    );
    let directive_name = string_table.intern("slot");
    let canonical_owner = file_tokens.canonical_owner();
    let canonical_range = canonical_owner
        .full_range()
        .expect("test token stream must expose canonical source range");
    let mut cursor = AstCursor::from_source_tokens(&canonical_owner, canonical_range)
        .expect("test token stream must expose an AST cursor");
    cursor
        .set_position(file_tokens.opener_index + 1)
        .expect("test token stream position must remain in canonical range");
    let result = parse_slot_declaration_for_test(
        directive_name,
        &mut cursor,
        file_tokens.source_path,
        &mut string_table,
        &mut path_fork,
    );
    assert_invalid_template_directive(
        &result.unwrap_err(),
        InvalidTemplateDirectiveCode::InvalidSlotDefault,
    );
}

// ------------------------------------------------------------------------
// parse_in_target_argument
// ------------------------------------------------------------------------

#[test]
fn in_target_missing_name_errors() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let file_tokens = directive_tokens(
        "[$in]",
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    );
    let directive_name = string_table.intern("in");
    let canonical_owner = file_tokens.canonical_owner();
    let canonical_range = canonical_owner
        .full_range()
        .expect("test token stream must expose canonical source range");
    let mut cursor = AstCursor::from_source_tokens(&canonical_owner, canonical_range)
        .expect("test token stream must expose an AST cursor");
    cursor
        .set_position(file_tokens.opener_index + 1)
        .expect("test token stream position must remain in canonical range");
    let result = parse_in_target_for_test(directive_name, &mut cursor, &mut string_table);
    assert_invalid_template_directive(
        &result.unwrap_err(),
        InvalidTemplateDirectiveCode::MissingArgument,
    );
}

#[test]
fn in_target_named_ok() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let file_tokens = directive_tokens(
        "[$in style]",
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    );
    let directive_name = string_table.intern("in");
    let canonical_owner = file_tokens.canonical_owner();
    let canonical_range = canonical_owner
        .full_range()
        .expect("test token stream must expose canonical source range");
    let mut cursor = AstCursor::from_source_tokens(&canonical_owner, canonical_range)
        .expect("test token stream must expose an AST cursor");
    cursor
        .set_position(file_tokens.opener_index + 1)
        .expect("test token stream position must remain in canonical range");
    let result = parse_in_target_for_test(directive_name, &mut cursor, &mut string_table);
    assert!(result.is_ok());
}

#[test]
fn in_target_positional_rejected() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let file_tokens = directive_tokens(
        "[$in 1]",
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    );
    let directive_name = string_table.intern("in");
    let canonical_owner = file_tokens.canonical_owner();
    let canonical_range = canonical_owner
        .full_range()
        .expect("test token stream must expose canonical source range");
    let mut cursor = AstCursor::from_source_tokens(&canonical_owner, canonical_range)
        .expect("test token stream must expose an AST cursor");
    cursor
        .set_position(file_tokens.opener_index + 1)
        .expect("test token stream position must remain in canonical range");
    let result = parse_in_target_for_test(directive_name, &mut cursor, &mut string_table);
    assert_invalid_template_directive(
        &result.unwrap_err(),
        InvalidTemplateDirectiveCode::InvalidInsertTarget,
    );
}

#[test]
fn in_target_old_parenthesized_form_rejected() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let file_tokens = directive_tokens(
        "[$in(\"style\")]",
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    );
    let directive_name = string_table.intern("in");
    let canonical_owner = file_tokens.canonical_owner();
    let canonical_range = canonical_owner
        .full_range()
        .expect("test token stream must expose canonical source range");
    let mut cursor = AstCursor::from_source_tokens(&canonical_owner, canonical_range)
        .expect("test token stream must expose an AST cursor");
    cursor
        .set_position(file_tokens.opener_index + 1)
        .expect("test token stream position must remain in canonical range");
    let result = parse_in_target_for_test(directive_name, &mut cursor, &mut string_table);
    assert_invalid_template_directive(
        &result.unwrap_err(),
        InvalidTemplateDirectiveCode::UnexpectedArguments,
    );
}

// ------------------------------------------------------------------------
// parse_optional_directive_operand
// ------------------------------------------------------------------------

#[test]
fn bare_directive_has_no_operand() {
    let result = parse_optional_expression_source("[$css]").expect("a bare directive must parse");
    assert!(result.is_none());
}

#[test]
fn operand_value_is_parsed_and_cursor_stops_at_boundary() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let file_tokens = directive_tokens(
        "[$code \"wrap\"]",
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    );
    let context = test_context(file_tokens.source_path, &path_fork);
    let canonical_owner = file_tokens.canonical_owner();
    let canonical_range = canonical_owner
        .full_range()
        .expect("test token stream must expose canonical source range");
    let mut cursor = AstCursor::from_source_tokens(&canonical_owner, canonical_range)
        .expect("test token stream must expose an AST cursor");
    cursor
        .set_position(file_tokens.opener_index + 1)
        .expect("test token stream position must remain in canonical range");
    let result = parse_optional_directive_operand_for_test(
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
        "the operand parser must leave the cursor on the head boundary",
    );
}

#[test]
fn call_shaped_directive_is_rejected() {
    let diagnostic = parse_optional_expression_source("[$code(\"rust\")]")
        .expect_err("the removed call spelling must be rejected");

    let facts = diagnostic
        .first()
        .expect("call-shape diagnostic should compact to one record");
    match &facts.kind {
        DirectiveStyleTestDiagnosticKind::InvalidTemplateDirective { reason, detail } => {
            assert_eq!(*reason, InvalidTemplateDirectiveCode::UnexpectedArguments);
            assert_eq!(*detail, None);
        }
        other => panic!("expected invalid-template-directive facts, found {other:?}"),
    }
}

#[test]
fn call_shaped_directive_with_empty_parentheses_is_rejected() {
    // Empty parentheses are the same call-shape mistake, not a separate arity diagnostic.
    let diagnostic = parse_optional_expression_source("[$code()]")
        .expect_err("empty parentheses must be rejected");
    assert_invalid_template_directive(
        &diagnostic,
        InvalidTemplateDirectiveCode::UnexpectedArguments,
    );
}

#[test]
fn operand_on_a_directive_that_takes_none_is_rejected() {
    // The directive shape is the problem, so the diagnostic must not degrade into an ordinary
    // unknown-name failure for the operand expression.
    let error = template_parse_error("[$md markdown: body]");

    assert!(
        error.contains("`$md` takes no operand"),
        "unexpected no-operand diagnostic: {error}"
    );
}

#[test]
fn whitespace_separated_parenthesised_operand_is_not_a_call() {
    // Only an attached `(` is the removed call spelling. A spaced parenthesised value stays an
    // ordinary operand, so the failure is the operand type rather than the call shape.
    let error = template_parse_error("[$code (42): body]");

    assert!(
        error.contains("Invalid argument for template directive 'code'"),
        "unexpected spaced-operand diagnostic: {error}"
    );
    assert!(
        !error.contains("takes exactly one bare language operand"),
        "a spaced parenthesised value must not be reported as a call: {error}"
    );
}

#[test]
fn second_operand_is_rejected() {
    // A second bare value is not a directive argument list: the operand parser stops before it and
    // the directive boundary names the malformed directive at the extra value.
    let diagnostic = parse_optional_expression_source("[$code \"a\" \"b\"]")
        .expect_err("a second operand must be rejected");

    assert_invalid_template_directive(
        &diagnostic,
        InvalidTemplateDirectiveCode::UnexpectedArguments,
    );
    let facts = diagnostic
        .first()
        .expect("extra-operand diagnostic should compact to one record");
    assert!(
        matches!(facts.primary_site, DirectiveStyleTestSite::Local(_)),
        "the extra operand must be cited at its authored span, found {:?}",
        facts.primary_site
    );
}

#[test]
fn second_operand_after_a_template_operand_is_rejected() {
    // A template operand ends the directive's one operand just like a literal does, including
    // past discarded comment templates, so the boundary still reports the extra value.
    let diagnostic = parse_optional_expression_source("[$code [:prefix] \"moth\"]")
        .expect_err("a value after a template operand must be rejected");

    assert_invalid_template_directive(
        &diagnostic,
        InvalidTemplateDirectiveCode::UnexpectedArguments,
    );
}

#[test]
fn mutable_operand_is_rejected() {
    // The ordinary mutable-value rule owns this rejection, and it cites the operand itself.
    let error = template_parse_error("[$code ~1: body]");
    assert!(
        error.contains("`~` is not an ordinary value operator"),
        "unexpected mutable-operand diagnostic: {error}"
    );
}

#[test]
fn wrong_operand_type_is_rejected() {
    let error = template_parse_error("[$code 42: body]");
    assert!(!error.is_empty());
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
    let expected = "`$slot` cannot be combined with an expression because a slot declaration must be the template head's only meaningful item.";
    let before_error = template_parse_error("[\"prefix\", $slot]");
    let after_error = template_parse_error("[$slot, \"suffix\"]");

    assert!(
        before_error.contains(expected),
        "expected the slot-exclusivity reason for a later $slot, got: {before_error}"
    );
    assert!(
        after_error.contains(expected),
        "expected the slot-exclusivity reason for an earlier $slot, got: {after_error}"
    );

    let directive_error = template_parse_error("[$slot, $fresh]");
    assert!(
        directive_error.contains(
            "`$slot` cannot be combined with `$fresh` because a slot declaration must be the template head's only meaningful item."
        ),
        "expected the slot-exclusivity reason for a later directive, got: {directive_error}"
    );
}

#[test]
fn comment_directives_must_be_alone_in_template_head() {
    for (directive, clause) in [
        (
            "note",
            "a discarded comment directive must be the template's only meaningful head item",
        ),
        (
            "todo",
            "a discarded comment directive must be the template's only meaningful head item",
        ),
        (
            "doc",
            "a documentation directive must be the template's only meaningful head item",
        ),
    ] {
        let expected =
            format!("`${directive}` cannot be combined with an expression because {clause}.");
        let before = format!("[\"prefix\", ${directive}]");
        let after = format!("[${directive}, \"suffix\"]");
        let before_error = template_parse_error(&before);
        let after_error = template_parse_error(&after);

        assert!(
            before_error.contains(&expected),
            "expected the comment-directive reason for '{before}', got: {before_error}"
        );
        assert!(
            after_error.contains(&expected),
            "expected the comment-directive reason for '{after}', got: {after_error}"
        );
    }
}

#[test]
fn comment_directive_conflict_with_another_directive_names_both() {
    for (source, expected) in [
        (
            "[$note, $fresh]",
            "`$note` cannot be combined with `$fresh` because a discarded comment directive must be the template's only meaningful head item.",
        ),
        (
            "[$doc, $fresh]",
            "`$doc` cannot be combined with `$fresh` because a documentation directive must be the template's only meaningful head item.",
        ),
    ] {
        let error = template_parse_error(source);
        assert!(
            error.contains(expected),
            "unexpected comment-directive pair reason for '{source}': {error}"
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
        "[\"prefix\", $in style: body]",
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    );
    let source_path = file_tokens.source_path;
    let context = new_constant_context(source_path, &path_fork);
    let canonical_owner = file_tokens.canonical_owner();
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
    let error = template_parse_error("[$in a, $in b: body]");
    assert!(
        error.contains("`$in` cannot appear more than once in a template head."),
        "unexpected duplicate-insert reason: {error}"
    );
}

#[test]
fn frontend_formatter_directives_are_exclusive_per_template_head() {
    let style_directives = frontend_test_style_directives();
    let error =
        template_parse_error_with_style_directives("[$html, $css: body]", &style_directives);
    assert!(
        error.contains(
            "`$html` and `$css` cannot be combined because both control template body formatting. Choose one formatter."
        ),
        "unexpected formatter-conflict reason: {error}"
    );
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
    let canonical_owner = file_tokens.canonical_owner();
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
    let canonical_owner = file_tokens.canonical_owner();
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
    let canonical_owner = file_tokens.canonical_owner();
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
    let canonical_owner = file_tokens.canonical_owner();
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
    let style_directives = frontend_test_style_directives();
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
    let canonical_owner = file_tokens.canonical_owner();
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
    let style_directives = frontend_test_style_directives();
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let file_tokens = template_tokens_from_source_with_style_directives(
        "[$css \"inline\":\ncolor: blue;\n]",
        &style_directives,
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    );
    let source_path = file_tokens.source_path;
    let context =
        new_constant_context_with_style_directives(source_path, &style_directives, &path_fork);
    let canonical_owner = file_tokens.canonical_owner();
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
    let style_directives = frontend_test_style_directives();
    let error = template_parse_error_with_style_directives(
        "[$css inline: color: blue;]",
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
    let style_directives = frontend_test_style_directives();
    let error = template_parse_error_with_style_directives(
        "[$css \"scoped\": color: blue;]",
        &style_directives,
    );
    assert!(!error.is_empty());
}

#[test]
fn const_css_template_emits_malformed_css_warnings() {
    let style_directives = frontend_test_style_directives();
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
    let style_directives = frontend_test_style_directives();
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
    let style_directives = frontend_test_style_directives();
    let warnings = template_warnings_with_style_directives(
        "[$css \"inline\":\n.button { color: red; }\n]",
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
    let style_directives = frontend_test_style_directives();
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
    let rendered = folded_template_output("[$code \"moth\":\nloop x\n-- hi\n]");

    assert!(rendered.contains("<span class='moth-code-keyword'>loop</span>"));
    assert!(rendered.contains("<span class='moth-code-comment'>-- hi</span>"));
}

#[test]
fn code_javascript_argument_highlights_js_comments() {
    let rendered = folded_template_output("[$code \"js\":\nconst x = 1\n// hi\n]");

    assert!(rendered.contains("<span class='moth-code-keyword'>const</span>"));
    assert!(rendered.contains("<span class='moth-code-comment'>// hi</span>"));
}

#[test]
fn code_python_argument_highlights_python_comments() {
    let rendered = folded_template_output("[$code \"py\":\ndef run():\n# hi\n]");

    assert!(rendered.contains("<span class='moth-code-keyword'>def</span>"));
    assert!(rendered.contains("<span class='moth-code-comment'># hi</span>"));
}

#[test]
fn code_typescript_argument_highlights_typescript_types() {
    let rendered = folded_template_output("[$code \"ts\":\ntype Name = string\n]");

    assert!(rendered.contains("<span class='moth-code-keyword'>type</span>"));
    assert!(rendered.contains("<span class='moth-code-type'>string</span>"));
}

#[test]
fn code_call_shape_is_rejected() {
    let error = template_parse_error("[$code(): body]");

    assert!(
        error.contains("`$code` takes at most one bare language operand."),
        "unexpected error message: {error}"
    );
}

#[test]
fn code_rejects_unknown_language_aliases() {
    let error = template_parse_error("[$code \"unknown\": body]");

    assert!(!error.is_empty());
}

#[test]
fn code_rejects_multiple_language_arguments() {
    let error = template_parse_error("[$code \"moth\" \"js\": body]");

    assert!(
        error.contains("`$code` takes at most one bare language operand."),
        "unexpected extra-operand diagnostic: {error}"
    );
    assert!(
        !error.contains("Expected an operator"),
        "an extra directive operand must not surface as an expression operator error: {error}"
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
        "[value, $code \"moth\":\nloop x\n]",
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
    let canonical_owner = file_tokens.canonical_owner();
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
        "[$code \"moth\":\nconcatenated_strings = [string_slice, a_mutable_string]\n]",
    );

    assert!(rendered.contains("<code class='codeblock'>"));
    assert!(rendered.contains("concatenated_strings"));
    assert!(rendered.contains("string_slice"));
    assert!(rendered.contains("a_mutable_string"));
    assert!(rendered.contains("<span class='moth-code-delimiter'>[</span>"));
    assert!(rendered.contains("<span class='moth-code-delimiter'>]</span>"));
}
