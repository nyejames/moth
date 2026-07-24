use super::*;
use crate::compiler_frontend::ast::templates::template::TemplateType;
use crate::compiler_frontend::symbols::string_interning::StringTable;

#[test]
fn markdown_formats_only_template_body_content() {
    let mut string_table = StringTable::new();
    let mut token_stream =
        template_tokens_from_source("[\"prefix\", $md:\n# Hello\n]", &mut string_table);
    let context = new_constant_context(token_stream.src_path.to_owned());

    let template = Template::new(&mut token_stream, &context, vec![], &mut string_table)
        .expect("template should parse");

    assert!(matches!(
        effective_tir_kind(&template, &context),
        TemplateType::String
    ));
    let folded = fold_template_in_context(&template, &context, &mut string_table);
    let rendered = string_table.resolve(folded);

    assert!(rendered.starts_with("prefix"));
    assert!(rendered.contains("<h1>Hello</h1>"));
    assert!(!rendered.starts_with("<p>prefix"));
}

#[test]
fn markdown_supports_h2_headings() {
    let rendered = folded_template_output("[$md:\n## Documentation\n]");

    assert!(rendered.contains("<h2>Documentation</h2>"));
}

#[test]
fn markdown_links_render_to_anchor_tags() {
    let rendered =
        folded_template_output("[$md:\nVisit @https://example.com/docs (Moth docs)\n]");

    assert!(rendered.contains("<a href=\"https://example.com/docs\">Moth docs</a>"));
}

#[test]
fn markdown_does_not_escape_html_inserted_from_template_head() {
    let rendered = folded_template_output("[\"<b>head-html</b>\", $md:\nbody\n]");

    assert!(rendered.starts_with("<b>head-html</b>"));
    assert!(!rendered.contains("&lt;b&gt;head-html&lt;/b&gt;"));
}

#[test]
fn markdown_does_not_reformat_plain_child_template_bodies() {
    let rendered = folded_template_output("[$md:\n[\"<i>child-head</i>\": <b>child-body</b>]\n]");

    assert!(rendered.contains("<i>child-head</i>"));
    assert!(!rendered.contains("&lt;i&gt;child-head&lt;/i&gt;"));
    assert!(rendered.contains("<b>child-body</b>"));
    assert!(!rendered.contains("&lt;b&gt;child-body&lt;/b&gt;"));
}

#[test]
fn markdown_redeclaration_formats_child_template_bodies() {
    let rendered =
        folded_template_output("[$md:\n[\"<i>child-head</i>\", $md: <b>child-body</b>]\n]");

    assert!(rendered.contains("<i>child-head</i>"));
    assert!(!rendered.contains("&lt;i&gt;child-head&lt;/i&gt;"));
    assert!(rendered.contains("&lt;b&gt;child-body&lt;/b&gt;"));
    assert!(!rendered.contains("<b>child-body</b>"));
}

#[test]
fn markdown_allows_literal_head_insert_inside_inline_code_span() {
    let rendered = folded_template_output("[$md:\nLiteral syntax `[\"[slot]\"]`\n]");

    assert_eq!(rendered, "<p>Literal syntax <code>[slot]</code></p>");
}

#[test]
fn markdown_keeps_inline_child_templates_inside_current_paragraph() {
    let rendered = folded_template_output("[$md:\nhello [:child] world\n]");

    assert_eq!(rendered, "<p>hello child world</p>");
}

#[test]
fn markdown_single_newline_before_child_template_closes_paragraph() {
    let rendered = folded_template_output("[$md:\nhello\n[:child]\nworld\n]");

    assert_eq!(rendered, "<p>hello</p>child<p>world</p>");
}

#[test]
fn markdown_escapes_html_characters_in_body_text() {
    let rendered = folded_template_output("[$md:\n<b>Hello & \"World\" 'x'</b>\n]");

    assert!(rendered.contains("&lt;b&gt;Hello &amp; &quot;World&quot; &#39;x&#39;&lt;/b&gt;"));
    assert!(!rendered.contains("<b>Hello"));
}

#[test]
fn non_markdown_templates_do_not_escape_html_body_text() {
    let rendered = folded_template_output("[:<b>Hello & \"World\" 'x'</b>]");

    assert!(rendered.contains("<b>Hello & \"World\" 'x'</b>"));
    assert!(!rendered.contains("&lt;b&gt;"));
}

#[test]
fn markdown_renders_unordered_and_ordered_lists() {
    let rendered = folded_template_output("[$md:\n- first\n- second\n1. third\n2) fourth\n]");

    assert_eq!(
        rendered,
        "<ul><li>first</li><li>second</li></ul><ol><li>third</li><li>fourth</li></ol>"
    );
}

#[test]
fn markdown_list_items_absorb_immediate_newline_text() {
    let rendered = folded_template_output(
        "[$md:\n- Square brackets are NOT used for arrays, curly braces are used instead.\nSquare brackets are only used for string templates. Items in collections are accessed via methods.\n- Equality and other logical operators use keywords like \"is\" and \"not\"\n(you can't use == or ! for example)\n]",
    );

    assert_eq!(
        rendered,
        "<ul><li>Square brackets are NOT used for arrays, curly braces are used instead. Square brackets are only used for string templates. Items in collections are accessed via methods.</li><li>Equality and other logical operators use keywords like &quot;is&quot; and &quot;not&quot; (you can&#39;t use == or ! for example)</li></ul>"
    );
}

#[test]
fn markdown_list_breaks_immediately_on_heading_line() {
    let rendered = folded_template_output("[$md:\n- first\n## Heading\nplain paragraph\n]");

    assert_eq!(
        rendered,
        "<ul><li>first</li></ul><h2>Heading</h2><p>plain paragraph</p>"
    );
}

#[test]
fn markdown_keeps_inline_child_templates_inside_same_list_item() {
    let rendered = folded_template_output("[$md:\n- hello [:child] world\n]");

    assert_eq!(rendered, "<ul><li>hello child world</li></ul>");
}

#[test]
fn markdown_single_newline_before_child_template_stays_in_same_list_item() {
    let rendered = folded_template_output("[$md:\n- hello\n[:child]\nworld\n]");

    assert_eq!(rendered, "<ul><li><p>hello</p>child<p>world</p></li></ul>");
}
