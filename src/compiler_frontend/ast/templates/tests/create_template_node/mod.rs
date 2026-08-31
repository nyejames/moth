use super::*;
use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::const_values::store::ConstStringValue;
use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::template::{TemplateSegmentOrigin, TemplateType};
use crate::compiler_frontend::ast::templates::template_folding::{
    TemplateEmission, TemplateFoldResult,
};
use crate::compiler_frontend::ast::templates::tir::{
    TemplateIrNodeId, TemplateIrNodeKind, TemplateIrStore, TemplatePreparationMode,
    TemplatePreparationOutcome, TemplateTirPhase, TirView, fold_prepared_template,
    prepare_tir_view,
};
use crate::compiler_frontend::ast::{ContextKind, ScopeContext, TopLevelDeclarationTable};
use crate::compiler_frontend::compiler_messages::render::{
    DiagnosticRenderContext, terminal, terse,
};
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::numeric_text::token::NumericLiteralToken;
use crate::compiler_frontend::paths::path_format::PathStringFormatConfig;
use crate::compiler_frontend::paths::path_resolution::ProjectPathResolver;
use crate::compiler_frontend::style_directives::{StyleDirectiveRegistry, StyleDirectiveSpec};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::lexer::tokenize;
use crate::compiler_frontend::tokenizer::tokens::{
    CharPosition, FileTokens, SourceLocation, TemplateBodyMode, Token, TokenKind,
};
use crate::compiler_frontend::value_mode::ValueMode;
use crate::compiler_tests::test_support::frontend_test_style_directives;
use crate::projects::html_project::style_directives::html_project_style_directives;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

fn html_project_test_style_directives() -> StyleDirectiveRegistry {
    StyleDirectiveRegistry::merged(&html_project_style_directives())
        .expect("html project style directives should merge with core directives")
}

fn token(kind: TokenKind, line: i32) -> Token {
    Token::new(
        kind,
        SourceLocation {
            scope: InternedPath::new(),
            start_pos: CharPosition {
                line_number: line,
                char_column: 0,
            },
            end_pos: CharPosition {
                line_number: line,
                char_column: 120, // Arbitrary number
            },
        },
    )
}

fn numeric_token(value: &str, line: i32, string_table: &mut StringTable) -> Token {
    token(
        TokenKind::NumericLiteral(NumericLiteralToken::test_new(value, string_table)),
        line,
    )
}

fn template_tokens_from_source(source: &str, string_table: &mut StringTable) -> FileTokens {
    let style_directives = frontend_test_style_directives();
    template_tokens_from_source_with_style_directives(source, &style_directives, string_table)
}

fn template_tokens_from_source_with_style_directives(
    source: &str,
    style_directives: &StyleDirectiveRegistry,
    string_table: &mut StringTable,
) -> FileTokens {
    let scope = InternedPath::from_single_str("main.moth/#const_template0", string_table);
    let mut tokens = tokenize(
        source,
        &scope,
        crate::compiler_frontend::tokenizer::tokens::TokenizerEntryMode::SourceFile,
        style_directives,
        string_table,
        None,
    )
    .expect("tokenization should succeed");

    tokens.index = tokens
        .tokens
        .iter()
        .position(|token| matches!(token.kind, TokenKind::TemplateHead))
        .expect("expected a template opener");

    // Template AST parsing is downstream of the prepared-file freeze boundary. Preserve that
    // production lifecycle in direct parser tests so nested expression substreams share the
    // immutable table instead of a mutable tokenizer-owned one.
    tokens.freeze_path_syntax_for_test();

    tokens
}

fn template_tokens_from_source_with_directives(
    source: &str,
    directives: &[StyleDirectiveSpec],
    string_table: &mut StringTable,
) -> FileTokens {
    let registry = StyleDirectiveRegistry::merged(directives)
        .expect("test style directives should merge with core directives");
    let mut tokens =
        template_tokens_from_source_with_style_directives(source, &registry, string_table);

    tokens.index = tokens
        .tokens
        .iter()
        .position(|token| matches!(token.kind, TokenKind::TemplateHead))
        .expect("expected a template opener");

    tokens
}

fn test_project_path_resolver() -> ProjectPathResolver {
    let cwd = std::env::temp_dir();
    ProjectPathResolver::new(
        cwd.clone(),
        cwd,
        crate::compiler_frontend::source_packages::root_file::PreparedSourcePackageRoots::empty(),
        &crate::builder_surface::SourceFileKindRegistry::default(),
    )
    .expect("test path resolver should be valid")
}

/// Creates a real file inside the test resolver's entry root and returns its resource path.
///
/// A path head names one existing regular file, so a template test that needs a rendered path head
/// has to give the resolver something to find.
fn test_resource_file(file_name: &str) -> String {
    let file_path = std::env::temp_dir().join(file_name);
    std::fs::write(&file_path, b"test resource").expect("test resource file should be writable");

    file_name.to_owned()
}

fn with_test_path_context(
    context: ScopeContext,
    source_scope: &InternedPath,
    style_directives: &StyleDirectiveRegistry,
) -> ScopeContext {
    context
        .with_style_directives(style_directives)
        .with_project_path_resolver(Some(test_project_path_resolver()))
        .with_source_file_scope(source_scope.to_owned())
        .with_path_format_config(PathStringFormatConfig::default())
}

fn new_constant_context(scope: InternedPath) -> ScopeContext {
    let style_directives = frontend_test_style_directives();
    new_constant_context_with_style_directives(scope, &style_directives)
}

fn new_constant_context_with_style_directives(
    scope: InternedPath,
    style_directives: &StyleDirectiveRegistry,
) -> ScopeContext {
    let parent = with_test_path_context(
        ScopeContext::new_for_tests(
            ContextKind::Constant,
            scope.to_owned(),
            Rc::new(TopLevelDeclarationTable::new(vec![])),
            Arc::new(ExternalPackageRegistry::default()),
            vec![],
            0,
        ),
        &scope,
        style_directives,
    );
    ScopeContext::new_constant(scope, &parent)
}

fn fold_template_in_context(
    template: &Template,
    context: &ScopeContext,
    string_table: &mut StringTable,
) -> StringId {
    // Every caller supplies a parser-emitted or direct module-local TIR
    // reference, so folding never needs a content-finalizer fallback here.

    let mut fold_context = context.new_tir_fold_context(string_table);
    fold_template_with_fold_context(
        template,
        &context.template_ir_store,
        &mut fold_context,
        TemplatePreparationMode::Value,
    )
    .expect("template should fold")
}

fn fold_template_with_fold_context(
    template: &Template,
    store_handle: &Rc<RefCell<TemplateIrStore>>,
    fold_context: &mut crate::compiler_frontend::ast::templates::template_folding::TirFoldContext<
        '_,
    >,
    preparation_mode: TemplatePreparationMode,
) -> Result<StringId, crate::compiler_frontend::ast::templates::error::TemplateError> {
    let store = store_handle.borrow();
    let reference = &template.tir_reference;
    let view = TirView::with_minimum_phase(
        &store,
        reference.root,
        reference.phase,
        TemplateTirPhase::Composed,
        reference.context,
    )?;
    let prepared = prepare_tir_view(&view, preparation_mode)?;
    if !matches!(prepared.outcome, TemplatePreparationOutcome::Foldable) {
        panic!("test template expected to be foldable")
    }
    // This test helper returns only rendered text for parser assertions.
    let TemplateFoldResult { emission, .. } =
        fold_prepared_template(&prepared, view, fold_context)?;
    match emission {
        TemplateEmission::Output(ConstStringValue::Text(output)) => Ok(output),
        TemplateEmission::Output(ConstStringValue::Pieces(_)) => {
            panic!("structural emission reached a text-only test helper")
        }
        TemplateEmission::NoOutput => Ok(fold_context.string_table.intern("")),
        TemplateEmission::Break(_) | TemplateEmission::Continue(_) => {
            panic!("test template fold signal escaped its loop")
        }
    }
}

fn effective_tir_style(template: &Template, context: &ScopeContext) -> Style {
    let reference = &template.tir_reference;
    let store = context.template_ir_store.borrow();
    let view = TirView::new(&store, reference.root, reference.phase, reference.context)
        .expect("template reference should resolve through its module TIR store");
    let template_ir = view
        .root_template()
        .expect("template TIR entry should remain available");

    template_ir.style.clone()
}

fn effective_tir_kind(template: &Template, context: &ScopeContext) -> TemplateType {
    let store = context.template_ir_store.borrow();
    store
        .get_template(template.tir_reference.root)
        .expect("template TIR entry should remain available")
        .kind
        .clone()
}

fn runtime_template_context(scope: &InternedPath, string_table: &mut StringTable) -> ScopeContext {
    let style_directives = frontend_test_style_directives();
    runtime_template_context_with_style_directives(scope, &style_directives, string_table)
}

fn runtime_template_context_with_style_directives(
    scope: &InternedPath,
    style_directives: &StyleDirectiveRegistry,
    string_table: &mut StringTable,
) -> ScopeContext {
    let value_name = string_table.intern("value");
    let declaration = Declaration {
        id: scope.append(value_name),
        value: Expression::string_slice(
            string_table.intern("dynamic"),
            SourceLocation {
                scope: InternedPath::new(),
                start_pos: CharPosition {
                    line_number: 1,
                    char_column: 0,
                },
                end_pos: CharPosition {
                    line_number: 1,
                    char_column: 120, // Arbitrary number
                },
            },
            ValueMode::ImmutableOwned,
        ),
    };

    with_test_path_context(
        ScopeContext::new_for_tests(
            ContextKind::Template,
            scope.to_owned(),
            Rc::new(TopLevelDeclarationTable::new(vec![declaration])),
            Arc::new(ExternalPackageRegistry::default()),
            vec![],
            0,
        ),
        scope,
        style_directives,
    )
}

fn constant_template_context(scope: &InternedPath, declarations: &[Declaration]) -> ScopeContext {
    let style_directives = frontend_test_style_directives();
    constant_template_context_with_style_directives(scope, declarations, &style_directives)
}

fn constant_template_context_with_style_directives(
    scope: &InternedPath,
    declarations: &[Declaration],
    style_directives: &StyleDirectiveRegistry,
) -> ScopeContext {
    with_test_path_context(
        ScopeContext::new_for_tests(
            ContextKind::Constant,
            scope.to_owned(),
            Rc::new(TopLevelDeclarationTable::new(declarations.to_vec())),
            Arc::new(ExternalPackageRegistry::default()),
            vec![],
            0,
        ),
        scope,
        style_directives,
    )
}

fn folded_template_output(source: &str) -> String {
    let style_directives = frontend_test_style_directives();
    folded_template_output_with_style_directives(source, &style_directives)
}

fn folded_template_output_with_style_directives(
    source: &str,
    style_directives: &StyleDirectiveRegistry,
) -> String {
    let mut string_table = StringTable::new();
    let mut token_stream = template_tokens_from_source_with_style_directives(
        source,
        style_directives,
        &mut string_table,
    );
    let context = new_constant_context_with_style_directives(
        token_stream.src_path.to_owned(),
        style_directives,
    );

    let template = Template::new(&mut token_stream, &context, vec![], &mut string_table)
        .expect("template should parse");
    let folded = fold_template_in_context(&template, &context, &mut string_table);

    string_table.resolve(folded).to_owned()
}

fn template_parse_error(source: &str) -> String {
    let style_directives = frontend_test_style_directives();
    template_parse_error_with_style_directives(source, &style_directives)
}

fn template_parse_rendered_error_with_style_directives(
    source: &str,
    style_directives: &StyleDirectiveRegistry,
) -> String {
    let mut string_table = StringTable::new();
    let scope = InternedPath::from_single_str("main.moth/#const_template0", &mut string_table);
    let mut token_stream = match tokenize(
        source,
        &scope,
        crate::compiler_frontend::tokenizer::tokens::TokenizerEntryMode::SourceFile,
        style_directives,
        &mut string_table,
        None,
    ) {
        Ok(tokens) => tokens,
        Err(error) => {
            return render_test_diagnostic(&error, &string_table);
        }
    };
    token_stream.index = token_stream
        .tokens
        .iter()
        .position(|token| matches!(token.kind, TokenKind::TemplateHead))
        .expect("expected a template opener");
    token_stream.freeze_path_syntax_for_test();
    let context = new_constant_context_with_style_directives(
        token_stream.src_path.to_owned(),
        style_directives,
    );

    let error = Template::new(&mut token_stream, &context, vec![], &mut string_table)
        .expect_err("template should fail to parse");
    let diagnostic = expect_template_diagnostic(error);

    render_test_diagnostic(&diagnostic, &string_table)
}

/// Extract the source-diagnostic lane expected by a template parser test.
///
/// Template construction now preserves internal retained-data failures separately. Tests that
/// exercise authored source rejection must state that expectation instead of treating a compiler
/// infrastructure error as an interchangeable diagnostic.
fn expect_template_diagnostic(
    error: TemplateError,
) -> crate::compiler_frontend::compiler_messages::CompilerDiagnostic {
    match error {
        TemplateError::Diagnostic(diagnostic) => *diagnostic,
        TemplateError::Infrastructure(error) => {
            panic!("expected a template source diagnostic, got infrastructure failure: {error:?}")
        }
    }
}

fn render_test_diagnostic(
    diagnostic: &crate::compiler_frontend::compiler_messages::CompilerDiagnostic,
    string_table: &StringTable,
) -> String {
    let render_context = DiagnosticRenderContext::new(string_table);
    let mut rendered = terse::format_terse_diagnostic_with_context(diagnostic, render_context);

    let guidance = terminal::format_payload_guidance(&diagnostic.payload, render_context);
    if !guidance.is_empty() {
        rendered.push('\n');
        rendered.push_str(&guidance.join("\n"));
    }

    rendered
}

fn template_parse_error_with_style_directives(
    source: &str,
    style_directives: &StyleDirectiveRegistry,
) -> String {
    template_parse_rendered_error_with_style_directives(source, style_directives)
}

fn template_warnings_with_style_directives(
    source: &str,
    runtime_context: bool,
    style_directives: &StyleDirectiveRegistry,
) -> Vec<crate::compiler_frontend::compiler_messages::CompilerDiagnostic> {
    let mut string_table = StringTable::new();
    let mut token_stream = template_tokens_from_source_with_style_directives(
        source,
        style_directives,
        &mut string_table,
    );
    let context = if runtime_context {
        runtime_template_context_with_style_directives(
            &token_stream.src_path,
            style_directives,
            &mut string_table,
        )
    } else {
        new_constant_context_with_style_directives(
            token_stream.src_path.to_owned(),
            style_directives,
        )
    };

    let _ = Template::new(&mut token_stream, &context, vec![], &mut string_table)
        .expect("template should parse for warning checks");
    context.take_emitted_warnings()
}

fn collect_body_text_from_tir(
    template: &Template,
    store: &TemplateIrStore,
    string_table: &StringTable,
) -> Vec<String> {
    let reference = &template.tir_reference;
    let template_ir = match store.get_template(reference.root) {
        Some(t) => t,
        None => return Vec::new(),
    };
    let root_node = match store.get_node(template_ir.root) {
        Some(n) => n,
        None => return Vec::new(),
    };
    let children: Vec<TemplateIrNodeId> = match &root_node.kind {
        TemplateIrNodeKind::Sequence { children } => children.clone(),
        TemplateIrNodeKind::Text { text, origin, .. } if *origin == TemplateSegmentOrigin::Body => {
            return vec![string_table.resolve(*text).to_owned()];
        }
        _ => return Vec::new(),
    };
    children
        .iter()
        .filter_map(|&child_id| {
            let child = store.get_node(child_id)?;
            match &child.kind {
                TemplateIrNodeKind::Text { text, origin, .. }
                    if *origin == TemplateSegmentOrigin::Body =>
                {
                    Some(string_table.resolve(*text).to_owned())
                }
                _ => None,
            }
        })
        .collect()
}

fn tir_root_has_head_dynamic_expression(
    template: &Template,
    store: &TemplateIrStore,
    predicate: impl Fn(&Expression) -> bool,
) -> bool {
    let reference = &template.tir_reference;
    let Some(template_ir) = store.get_template(reference.root) else {
        return false;
    };
    let Some(root) = store.get_node(template_ir.root) else {
        return false;
    };

    let matches_node = |node_id| {
        store.get_node(node_id).is_some_and(|node| {
            matches!(
                &node.kind,
                TemplateIrNodeKind::DynamicExpression {
                    expression,
                    origin: TemplateSegmentOrigin::Head,
                    ..
                } if predicate(expression)
            )
        })
    };

    match &root.kind {
        TemplateIrNodeKind::Sequence { children } => children.iter().copied().any(matches_node),
        TemplateIrNodeKind::DynamicExpression {
            expression,
            origin: TemplateSegmentOrigin::Head,
            ..
        } => predicate(expression),
        _ => false,
    }
}

fn is_default_text_location(location: &SourceLocation) -> bool {
    location.scope == InternedPath::new()
        && location.start_pos == CharPosition::default()
        && location.end_pos == CharPosition::default()
}

fn is_default_error_location(location: &SourceLocation) -> bool {
    location.scope == InternedPath::new()
        && location.start_pos == CharPosition::default()
        && location.end_pos == CharPosition::default()
}

mod builder_tests;
mod children_tests;
mod directive_built_in_tests;
mod directive_style_tests;
mod head_tests;
mod markdown_tests;
mod parser_tir_malformed_tests;
mod parser_tir_tests;
mod render_tests;
mod slot_constant_tests;
mod whitespace_tests;
mod wrapper_tests;
