use super::*;
use crate::compiler_frontend::ast::expressions::expression::ExpressionKind;
use crate::compiler_frontend::symbols::string_interning::StringTable;

#[test]
fn default_whitespace_normalizer_trims_initial_newline_and_dedents() {
    let rendered = folded_template_output("[:\n    Hello\n    World\n]");
    assert_eq!(rendered, "Hello\nWorld");
}

#[test]
fn default_whitespace_normalizer_preserves_consecutive_blank_lines() {
    let rendered = folded_template_output("[:\n    Hello\n\n    World\n]");
    assert_eq!(rendered, "Hello\n\nWorld");
}

#[test]
fn default_whitespace_normalizer_trims_only_from_final_newline() {
    let rendered = folded_template_output("[:\n    Hello   \n]");
    assert_eq!(rendered, "Hello   ");
}

#[test]
fn default_whitespace_normalizer_preserves_leading_spaces_without_initial_newline() {
    let rendered = folded_template_output("[: world]");
    assert_eq!(rendered, " world");
}

#[test]
fn default_whitespace_normalizer_preserves_middle_run_newline_boundaries() {
    // A child template in the body is now an opaque TIR anchor that skips the
    // content mirror. Verify whitespace normalization through the folded TIR
    // output instead of the content-mirror render plan.
    let rendered = folded_template_output("[:\n    before\n    [:dynamic]\n    after\n]");

    assert_eq!(rendered, "before\ndynamic\nafter");
}

#[test]
fn raw_directive_preserves_authored_whitespace() {
    let rendered = folded_template_output("[$raw:\n    Hello\n    World\n]");
    assert_eq!(rendered, "\n    Hello\n    World\n");
}

#[test]
fn escape_html_escapes_body_html_sensitive_characters() {
    let style_directives = html_project_test_style_directives();
    let rendered = folded_template_output_with_style_directives(
        "[$escape_html:\n    <b>Hello & \"World\" 'x'</b>\n]",
        &style_directives,
    );

    assert!(rendered.contains("&lt;b&gt;Hello &amp; &quot;World&quot; &#39;x&#39;&lt;/b&gt;"));
    assert!(!rendered.contains("<b>Hello"));
}

#[test]
fn escape_html_preserves_runtime_head_references() {
    let style_directives = html_project_test_style_directives();
    let mut string_table = StringTable::new();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut token_stream = template_tokens_from_source_with_style_directives(
        "[value, $escape_html:\n    <b>body</b>\n]",
        &style_directives,
        &mut string_table,
        &mut span_builder,
    );
    let context = runtime_template_context_with_style_directives(
        &token_stream.src_path,
        &style_directives,
        &mut string_table,
    );

    let template = Template::new(&mut token_stream, &context, vec![], &mut string_table, &mut PathInternerFork::empty())
        .expect("template should parse");

    let store = context.template_ir_store.borrow();
    assert!(tir_root_has_head_dynamic_expression(
        &template,
        &store,
        |expression| matches!(expression.kind, ExpressionKind::Reference(_))
    ));

    // Formatted body text is in the TIR root after the escape pass.
    let body_texts = collect_body_text_from_tir(&template, &store, &string_table);
    let escaped_body = body_texts
        .into_iter()
        .find(|text| !text.is_empty())
        .expect("expected escaped body text in formatted body");

    assert_eq!(escaped_body, "\n    &lt;b&gt;body&lt;/b&gt;\n");
}

#[test]
fn literal_brackets_via_string_insertion() {
    let rendered = folded_template_output("[:[\"[code]\"]]");
    assert_eq!(rendered, "[code]");

    let closing = folded_template_output("[:[\"]\"]]");
    assert_eq!(closing, "]");
}

#[test]
fn literal_backslash_and_backtick_preserved_in_folded_output() {
    let rendered = folded_template_output("[:path\\file`name]");
    assert!(rendered.contains("\\"));
    assert!(rendered.contains("`"));
    assert_eq!(rendered, "path\\file`name");
}
