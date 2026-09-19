use super::*;
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::expression::ExpressionKind;
use crate::compiler_frontend::ast::templates::template::TemplateType;
use crate::compiler_frontend::compiler_messages::{DiagnosticKind, SyntaxDiagnosticKind};
use crate::compiler_frontend::symbols::string_interning::StringTable;

#[test]
fn html_directive_rejects_arguments() {
    let style_directives = html_project_test_style_directives();
    let error = template_parse_error_with_style_directives(
        "[$html(\"inline\"):\n<div>Hello</div>\n]",
        &style_directives,
    );
    assert!(!error.is_empty());
}

#[test]
fn html_directive_sets_formatter_via_handler_behavior() {
    let style_directives = html_project_test_style_directives();
    let mut string_table = StringTable::new();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut path_fork = PathInternerFork::empty();
    let mut file_tokens = template_tokens_from_source_with_style_directives(
        "[$html:\n<div class=\"card\">x</div>\n]",
        &style_directives,
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    );
    let source_path = file_tokens.src_path;
    let canonical_owner = file_tokens
        .canonical_source_tokens_arc()
        .expect("test token stream must expose canonical source tokens");
    let canonical_range = canonical_owner
        .full_range()
        .expect("test token stream must expose canonical source range");
    let mut token_stream = AstCursor::from_source_tokens_for_handoff(
        &canonical_owner,
        file_tokens.canonical_os_path.clone(),
        canonical_range,
    )
    .expect("test token stream must expose an AST cursor");
    let context =
        new_constant_context_with_style_directives(source_path, &style_directives, &path_fork);

    let template = Template::new(
        &mut token_stream,
        source_path,
        &context,
        vec![],
        &mut string_table,
        &mut path_fork,
    )
    .expect("html template should parse");

    let effective_style = effective_tir_style(&template, &context);
    assert_eq!(effective_style.id, "html");
    assert!(effective_style.formatter.is_some());
}

#[test]
fn css_directive_sets_style_and_formatter_identity() {
    let style_directives = html_project_test_style_directives();
    let mut string_table = StringTable::new();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut path_fork = PathInternerFork::empty();
    let mut file_tokens = template_tokens_from_source_with_style_directives(
        "[$css:\n.button { color: red; }\n]",
        &style_directives,
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    );
    let source_path = file_tokens.src_path;
    let canonical_owner = file_tokens
        .canonical_source_tokens_arc()
        .expect("test token stream must expose canonical source tokens");
    let canonical_range = canonical_owner
        .full_range()
        .expect("test token stream must expose canonical source range");
    let mut token_stream = AstCursor::from_source_tokens_for_handoff(
        &canonical_owner,
        file_tokens.canonical_os_path.clone(),
        canonical_range,
    )
    .expect("test token stream must expose an AST cursor");
    let context =
        new_constant_context_with_style_directives(source_path, &style_directives, &path_fork);

    let template = Template::new(
        &mut token_stream,
        source_path,
        &context,
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
fn markdown_directive_sets_style_and_formatter_identity() {
    let mut string_table = StringTable::new();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut path_fork = PathInternerFork::empty();
    let mut file_tokens = template_tokens_from_source(
        "[$md:\n# Hello\n]",
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    );
    let source_path = file_tokens.src_path;
    let canonical_owner = file_tokens
        .canonical_source_tokens_arc()
        .expect("test token stream must expose canonical source tokens");
    let canonical_range = canonical_owner
        .full_range()
        .expect("test token stream must expose canonical source range");
    let mut token_stream = AstCursor::from_source_tokens_for_handoff(
        &canonical_owner,
        file_tokens.canonical_os_path.clone(),
        canonical_range,
    )
    .expect("test token stream must expose an AST cursor");
    let context = new_constant_context(source_path, &path_fork);

    let template = Template::new(
        &mut token_stream,
        source_path,
        &context,
        vec![],
        &mut string_table,
        &mut path_fork,
    )
    .expect("markdown template should parse");

    let effective_style = effective_tir_style(&template, &context);
    assert_eq!(effective_style.id, "markdown");
    assert!(effective_style.formatter.is_some());
}

#[test]
fn code_directive_sets_style_and_formatter_identity() {
    let mut string_table = StringTable::new();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut path_fork = PathInternerFork::empty();
    let mut file_tokens = template_tokens_from_source(
        "[$code:\nloop x\n]",
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    );
    let source_path = file_tokens.src_path;
    let canonical_owner = file_tokens
        .canonical_source_tokens_arc()
        .expect("test token stream must expose canonical source tokens");
    let canonical_range = canonical_owner
        .full_range()
        .expect("test token stream must expose canonical source range");
    let mut token_stream = AstCursor::from_source_tokens_for_handoff(
        &canonical_owner,
        file_tokens.canonical_os_path.clone(),
        canonical_range,
    )
    .expect("test token stream must expose an AST cursor");
    let context = new_constant_context(source_path, &path_fork);

    let template = Template::new(
        &mut token_stream,
        source_path,
        &context,
        vec![],
        &mut string_table,
        &mut path_fork,
    )
    .expect("code template should parse");

    let effective_style = effective_tir_style(&template, &context);
    assert_eq!(effective_style.id, "code");
    assert!(effective_style.formatter.is_some());
}

#[test]
fn escape_html_directive_sets_style_and_formatter_identity() {
    let style_directives = html_project_test_style_directives();
    let mut string_table = StringTable::new();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut path_fork = PathInternerFork::empty();
    let mut file_tokens = template_tokens_from_source_with_style_directives(
        "[$escape_html:\n<b>Hello</b>\n]",
        &style_directives,
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    );
    let source_path = file_tokens.src_path;
    let canonical_owner = file_tokens
        .canonical_source_tokens_arc()
        .expect("test token stream must expose canonical source tokens");
    let canonical_range = canonical_owner
        .full_range()
        .expect("test token stream must expose canonical source range");
    let mut token_stream = AstCursor::from_source_tokens_for_handoff(
        &canonical_owner,
        file_tokens.canonical_os_path.clone(),
        canonical_range,
    )
    .expect("test token stream must expose an AST cursor");
    let context =
        new_constant_context_with_style_directives(source_path, &style_directives, &path_fork);

    let template = Template::new(
        &mut token_stream,
        source_path,
        &context,
        vec![],
        &mut string_table,
        &mut path_fork,
    )
    .expect("escape_html template should parse");

    let effective_style = effective_tir_style(&template, &context);
    assert_eq!(effective_style.id, "escape_html");
    assert!(effective_style.formatter.is_some());
}

#[test]
fn const_html_template_emits_sanitation_warnings() {
    let style_directives = html_project_test_style_directives();
    let warnings = template_warnings_with_style_directives(
        "[$html:\n<script>alert(1)</script>\n<div onclick=\"run()\"></div>\n<a href=\"javascript:alert(1)\">x</a>\n]",
        false,
        &style_directives,
    );

    assert!(!warnings.is_empty());
    assert!(warnings.iter().all(|warning| {
        matches!(
            warning.kind,
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::MalformedHtmlTemplate)
        )
    }));
    assert!(warnings.iter().any(|warning| matches!(
        warning.kind,
        DiagnosticKind::Syntax(SyntaxDiagnosticKind::MalformedHtmlTemplate)
    )));
    assert!(warnings.iter().any(|warning| matches!(
        warning.kind,
        DiagnosticKind::Syntax(SyntaxDiagnosticKind::MalformedHtmlTemplate)
    )));
    assert!(warnings.iter().any(|warning| matches!(
        warning.kind,
        DiagnosticKind::Syntax(SyntaxDiagnosticKind::MalformedHtmlTemplate)
    )));
}

#[test]
fn html_validation_warnings_keep_authored_spans() {
    let style_directives = html_project_test_style_directives();
    let warnings = template_warnings_with_style_directives(
        "[$html:\n<script>alert(1)</script>\n<a href=\"javascript:bad()\">x</a>\n<div onclick=\"run()\"></div>\n]",
        false,
        &style_directives,
    );

    assert!(!warnings.is_empty());
    assert!(
        warnings
            .iter()
            .all(|warning| warning.primary_span.is_some()),
        "html warnings should keep authored source spans"
    );
}

#[test]
fn runtime_html_templates_emit_warnings_for_static_body_segments() {
    let style_directives = html_project_test_style_directives();
    let warnings = template_warnings_with_style_directives(
        "[value, $html:\n<script>alert(1)</script>\n]",
        true,
        &style_directives,
    );
    assert!(!warnings.is_empty());
    assert!(warnings.iter().all(|warning| matches!(
        warning.kind,
        DiagnosticKind::Syntax(SyntaxDiagnosticKind::MalformedHtmlTemplate)
    )));
}

#[test]
fn runtime_templates_format_static_body_strings_only() {
    let mut string_table = StringTable::new();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut path_fork = PathInternerFork::empty();
    let mut file_tokens = template_tokens_from_source(
        "[value, $md:\n# Hello\n]",
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    );
    let source_path = file_tokens.src_path;
    let canonical_owner = file_tokens
        .canonical_source_tokens_arc()
        .expect("test token stream must expose canonical source tokens");
    let canonical_range = canonical_owner
        .full_range()
        .expect("test token stream must expose canonical source range");
    let mut token_stream = AstCursor::from_source_tokens_for_handoff(
        &canonical_owner,
        file_tokens.canonical_os_path.clone(),
        canonical_range,
    )
    .expect("test token stream must expose an AST cursor");
    let context = runtime_template_context(&source_path, &mut string_table, &mut path_fork);

    let template = Template::new(
        &mut token_stream,
        source_path,
        &context,
        vec![],
        &mut string_table,
        &mut path_fork,
    )
    .expect("template should parse");

    assert!(matches!(
        effective_tir_kind(&template, &context),
        TemplateType::StringFunction
    ));
    let store = context.template_ir_store.borrow();
    assert!(tir_root_has_head_dynamic_expression(
        &template,
        &store,
        |expression| matches!(expression.kind, ExpressionKind::Reference(_))
    ));

    // Formatted body text is authoritative in the TIR root.
    let body_texts = collect_body_text_from_tir(&template, &store, &string_table);
    assert!(
        body_texts
            .iter()
            .any(|text| text.contains("<h1>Hello</h1>")),
        "expected formatted body text to contain markdown-rendered heading"
    );
}
