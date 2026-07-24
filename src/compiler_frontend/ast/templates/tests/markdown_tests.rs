use super::MarkdownTemplateFormatter;
use super::{MarkdownInlineAtom, MarkdownLine};
use crate::compiler_frontend::ast::templates::formatter_contract::{
    FormatterAnchorId, FormatterInput, FormatterInputPiece, FormatterOpaqueKind,
    FormatterOpaquePiece, FormatterOutputPiece, FormatterTextPiece,
};
use crate::compiler_frontend::ast::templates::styles::markdown::render_markdown_stream;
use crate::compiler_frontend::ast::templates::template::TemplateFormatter;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;

fn child_anchor(id: usize) -> FormatterOpaquePiece {
    FormatterOpaquePiece {
        id: FormatterAnchorId(id),
        kind: FormatterOpaqueKind::ChildTemplate,
    }
}

fn dynamic_anchor(id: usize) -> FormatterOpaquePiece {
    FormatterOpaquePiece {
        id: FormatterAnchorId(id),
        kind: FormatterOpaqueKind::DynamicExpression,
    }
}

fn formatter_text_piece(text: &str, string_table: &mut StringTable) -> FormatterInputPiece {
    FormatterInputPiece::Text(FormatterTextPiece {
        text: string_table.intern(text),
        location: SourceLocation::default(),
    })
}

/// Splits plain text into markdown lines for test helpers.
///
/// WHAT: mirrors the production `split_formatter_input_into_lines` logic
/// without building a `FormatterInput`, so tests can drive `render_markdown_stream`
/// directly from source strings.
fn split_text_into_lines(content: &str) -> Vec<MarkdownLine> {
    let mut lines = vec![MarkdownLine::default()];

    for ch in content.chars() {
        if ch == '\n' {
            lines.push(MarkdownLine::default());
        } else {
            super::push_atom_to_current_line(&mut lines, MarkdownInlineAtom::Char(ch));
        }
    }

    lines
}

fn to_markdown(content: &str, default_tag: &str) -> String {
    let lines = split_text_into_lines(content);
    let pieces = render_markdown_stream(&lines, default_tag);

    pieces
        .into_iter()
        .map(|piece| match piece {
            FormatterOutputPiece::Text(text) => text,
            FormatterOutputPiece::Opaque(_) => {
                unreachable!("plain-text markdown helper should never emit opaque anchors")
            }
        })
        .collect()
}

fn markdown_formatter_output_from_text_and_anchors(
    pieces: &[(Option<&str>, Option<FormatterOpaquePiece>)],
) -> String {
    let mut string_table = StringTable::new();
    let input_pieces = pieces
        .iter()
        .map(|(text, anchor)| match (text, anchor) {
            (Some(text), None) => formatter_text_piece(text, &mut string_table),
            (None, Some(anchor)) => FormatterInputPiece::Opaque(*anchor),
            _ => unreachable!("test pieces should be either text or anchor"),
        })
        .collect();

    MarkdownTemplateFormatter
        .format(
            FormatterInput {
                pieces: input_pieces,
            },
            &mut string_table,
        )
        .expect("markdown formatter should succeed")
        .output
        .pieces
        .into_iter()
        .map(|piece| match piece {
            FormatterOutputPiece::Text(text) => text,
            FormatterOutputPiece::Opaque(anchor) => match anchor.kind {
                FormatterOpaqueKind::ChildTemplate => format!("{{child:{}}}", anchor.id.0),
                FormatterOpaqueKind::DynamicExpression => {
                    format!("{{dynamic:{}}}", anchor.id.0)
                }
            },
        })
        .collect()
}

#[test]
fn parses_links_for_all_supported_target_prefixes() {
    let cases = [
        ("@https://example.com (Example)", "https://example.com"),
        (
            "@//cdn.example.com/lib.js (CDN)",
            "//cdn.example.com/lib.js",
        ),
        ("@/docs/getting-started (Docs)", "/docs/getting-started"),
        ("@./local/path (Local)", "./local/path"),
        ("@../parent/path (Parent)", "../parent/path"),
        ("@#overview (Overview)", "#overview"),
        ("@?q=moth (Search)", "?q=moth"),
    ];

    for (input, target) in cases {
        let rendered = to_markdown(input, "p");
        let expected = format!(
            "<p><a href=\"{target}\">{}</a></p>",
            input
                .split_once('(')
                .expect("label start should exist")
                .1
                .trim_end_matches(')')
        );
        assert_eq!(rendered, expected);
    }
}

#[test]
fn requires_non_whitespace_or_start_before_at_sign() {
    let rendered = to_markdown("email@https://example.com (Example)", "p");
    assert_eq!(rendered, "<p>email@https://example.com (Example)</p>");
}

#[test]
fn invalid_scheme_like_targets_do_not_parse_as_links() {
    let rendered = to_markdown("@example.com (Example)", "p");
    assert_eq!(rendered, "<p>@example.com (Example)</p>");
}

#[test]
fn requires_horizontal_whitespace_before_label() {
    let rendered = to_markdown("@https://example.com(Example)", "p");
    assert_eq!(rendered, "<p>@https://example.com(Example)</p>");
}

#[test]
fn newline_between_target_and_label_breaks_link_recognition() {
    let rendered = to_markdown("@https://example.com\n(Example)", "p");
    assert_eq!(rendered, "<p>@https://example.com (Example)</p>");
}

#[test]
fn rejects_empty_or_whitespace_only_labels() {
    let empty_label = to_markdown("@/docs ()", "p");
    assert_eq!(empty_label, "<p>@/docs ()</p>");

    let whitespace_label = to_markdown("@/docs (   )", "p");
    assert_eq!(whitespace_label, "<p>@/docs (   )</p>");
}

#[test]
fn missing_closing_paren_falls_back_to_literal_text() {
    let rendered = to_markdown("@/docs (Docs", "p");
    assert_eq!(rendered, "<p>@/docs (Docs</p>");
}

#[test]
fn malformed_candidate_keeps_literal_at_symbol() {
    let rendered = to_markdown("Visit @docs (Docs) today", "p");
    assert_eq!(rendered, "<p>Visit @docs (Docs) today</p>");
}

#[test]
fn link_parsing_works_inside_heading_and_emphasis() {
    let heading = to_markdown("\n# @/docs (Docs)\n", "p");
    assert_eq!(heading, "<h1><a href=\"/docs\">Docs</a></h1>");

    let emphasis = to_markdown("\n*@/docs (Docs)*\n", "p");
    assert_eq!(emphasis, "<p><em><a href=\"/docs\">Docs</a></em></p>");
}

#[test]
fn escapes_html_characters_in_plain_markdown_text() {
    let rendered = to_markdown("<tag> & \"quote\" 'apostrophe'", "p");
    assert_eq!(
        rendered,
        "<p>&lt;tag&gt; &amp; &quot;quote&quot; &#39;apostrophe&#39;</p>"
    );
}

#[test]
fn escapes_html_characters_inside_heading_and_emphasis_content() {
    let heading = to_markdown("\n# <h1> & \"q\" 'x'\n", "p");
    assert_eq!(
        heading,
        "<h1>&lt;h1&gt; &amp; &quot;q&quot; &#39;x&#39;</h1>"
    );

    let emphasis = to_markdown("\n*<tag>&\"'*\n", "p");
    assert!(emphasis.contains("<em>&lt;tag&gt;&amp;&quot;&#39;</em>"));
}

#[test]
fn escapes_link_target_and_label_html_characters() {
    let rendered = to_markdown(
        "@https://example.com?a=1&b=2\"x\"<tag> (\"<Click>\" & 'Go')",
        "p",
    );
    assert_eq!(
        rendered,
        "<p><a href=\"https://example.com?a=1&amp;b=2&quot;x&quot;&lt;tag&gt;\">&quot;&lt;Click&gt;&quot; &amp; &#39;Go&#39;</a></p>"
    );
}

#[test]
fn malformed_links_still_escape_literal_html_characters() {
    let rendered = to_markdown("Visit @docs (<b>)", "p");
    assert_eq!(rendered, "<p>Visit @docs (&lt;b&gt;)</p>");
}

#[test]
fn opaque_anchor_inside_link_candidate_falls_back_to_literal_text() {
    let rendered = markdown_formatter_output_from_text_and_anchors(&[
        (Some("@/docs"), None),
        (None, Some(child_anchor(7))),
        (Some(" (Docs)"), None),
    ]);

    assert_eq!(rendered, "<p>@/docs{child:7} (Docs)</p>");
}

#[test]
fn parses_unordered_list_markers_into_list_items() {
    let rendered = to_markdown("- first\n* second\n+ third", "p");
    assert_eq!(
        rendered,
        "<ul><li>first</li><li>second</li><li>third</li></ul>"
    );
}

#[test]
fn parses_ordered_list_markers_with_dot_or_paren() {
    let rendered = to_markdown("1. first\n2) second\n3. third", "p");
    assert_eq!(
        rendered,
        "<ol><li>first</li><li>second</li><li>third</li></ol>"
    );
}

#[test]
fn parses_nested_mixed_lists_by_indentation() {
    let rendered = to_markdown(
        "- parent\n  - child bullet\n  1. child ordered\n- sibling",
        "p",
    );

    assert!(rendered.starts_with("<ul><li>parent"));
    assert!(rendered.contains("<ul><li>child bullet</li></ul>"));
    assert!(rendered.contains("<ol><li>child ordered</li></ol>"));
    assert!(rendered.ends_with("<li>sibling</li></ul>"));
}

#[test]
fn newline_text_continues_previous_list_item() {
    let rendered = to_markdown(
        "- Square brackets are NOT used for arrays, curly braces are used instead.\nSquare brackets are only used for string templates. Items in collections are accessed via methods.\n- Equality and other logical operators use keywords like \"is\" and \"not\"\n(you can't use == or ! for example)",
        "p",
    );

    assert_eq!(
        rendered,
        "<ul><li>Square brackets are NOT used for arrays, curly braces are used instead. Square brackets are only used for string templates. Items in collections are accessed via methods.</li><li>Equality and other logical operators use keywords like &quot;is&quot; and &quot;not&quot; (you can&#39;t use == or ! for example)</li></ul>"
    );
}

#[test]
fn blank_line_breaks_list_continuation() {
    let rendered = to_markdown("- first line\ncontinuation line\n\nplain paragraph", "p");

    assert_eq!(
        rendered,
        "<ul><li>first line continuation line</li></ul><p>plain paragraph</p>"
    );
}

#[test]
fn heading_line_breaks_out_of_list_without_blank_line() {
    let rendered = to_markdown("- first line\n## Heading\nplain paragraph", "p");

    assert_eq!(
        rendered,
        "<ul><li>first line</li></ul><h2>Heading</h2><p>plain paragraph</p>"
    );
}

#[test]
fn list_items_keep_inline_markdown_links_and_escaping() {
    let rendered = to_markdown("- item *bold* @/docs (Docs) <tag>", "p");

    assert!(rendered.contains("<li>item <em>bold</em>"));
    assert!(rendered.contains("<a href=\"/docs\">Docs</a>"));
    assert!(rendered.contains("&lt;tag&gt;"));
}

#[test]
fn non_list_lines_remain_plain_markdown_blocks() {
    let rendered = to_markdown("-not a list\nstill plain text", "p");
    assert_eq!(rendered, "<p>-not a list still plain text</p>");
}

#[test]
fn line_leading_child_anchor_with_inline_text_opens_paragraph_before_anchor() {
    let rendered = markdown_formatter_output_from_text_and_anchors(&[
        (None, Some(child_anchor(7))),
        (Some(" introduces **compiler-handled directives**"), None),
    ]);

    assert_eq!(
        rendered,
        "<p>{child:7} introduces <strong>compiler-handled directives</strong></p>"
    );
}

#[test]
fn inline_child_anchor_stays_inside_one_paragraph() {
    let rendered = markdown_formatter_output_from_text_and_anchors(&[
        (Some("hello "), None),
        (None, Some(child_anchor(7))),
        (Some(" world"), None),
    ]);

    assert_eq!(rendered, "<p>hello {child:7} world</p>");
}

#[test]
fn newline_before_child_anchor_with_inline_text_starts_new_paragraph() {
    let rendered = markdown_formatter_output_from_text_and_anchors(&[
        (Some("previous paragraph\n"), None),
        (None, Some(child_anchor(7))),
        (Some(" introduces directives"), None),
    ]);

    assert_eq!(
        rendered,
        "<p>previous paragraph</p><p>{child:7} introduces directives</p>"
    );
}

#[test]
fn child_anchor_alone_on_line_keeps_existing_standalone_behavior() {
    let rendered = markdown_formatter_output_from_text_and_anchors(&[
        (Some("hello\n"), None),
        (None, Some(child_anchor(7))),
        (Some("\nworld"), None),
    ]);

    assert_eq!(rendered, "<p>hello</p>{child:7}<p>world</p>");
}

#[test]
fn two_newlines_before_child_anchor_keep_normal_paragraph_break_behavior() {
    let rendered = markdown_formatter_output_from_text_and_anchors(&[
        (Some("hello\n\n"), None),
        (None, Some(child_anchor(7))),
        (Some("\nworld"), None),
    ]);

    assert_eq!(rendered, "<p>hello</p>{child:7}<p>world</p>");
}

#[test]
fn single_newline_before_normal_text_keeps_paragraph_open() {
    let rendered = to_markdown("hello\nworld", "p");
    assert_eq!(rendered, "<p>hello world</p>");
}

#[test]
fn single_newline_before_dynamic_anchor_does_not_use_child_rule() {
    let rendered = markdown_formatter_output_from_text_and_anchors(&[
        (Some("hello\n"), None),
        (None, Some(dynamic_anchor(9))),
        (Some("\nworld"), None),
    ]);

    assert_eq!(rendered, "<p>hello {dynamic:9} world</p>");
}

#[test]
fn line_leading_dynamic_anchor_does_not_use_child_template_boundary_rule() {
    let rendered = markdown_formatter_output_from_text_and_anchors(&[
        (Some("hello\n"), None),
        (None, Some(dynamic_anchor(9))),
        (Some(" world"), None),
    ]);

    assert_eq!(rendered, "<p>hello {dynamic:9} world</p>");
}

#[test]
fn inline_child_anchor_stays_inside_same_list_item() {
    let rendered = markdown_formatter_output_from_text_and_anchors(&[
        (Some("- hello "), None),
        (None, Some(child_anchor(4))),
        (Some(" world"), None),
    ]);

    assert_eq!(rendered, "<ul><li>hello {child:4} world</li></ul>");
}

#[test]
fn list_item_starting_with_child_anchor_and_inline_text_stays_one_item() {
    let rendered = markdown_formatter_output_from_text_and_anchors(&[
        (Some("- "), None),
        (None, Some(child_anchor(4))),
        (Some(" introduces directives"), None),
    ]);

    assert_eq!(
        rendered,
        "<ul><li>{child:4} introduces directives</li></ul>"
    );
}

#[test]
fn single_newline_before_child_anchor_inside_list_keeps_same_li() {
    let rendered = markdown_formatter_output_from_text_and_anchors(&[
        (Some("- hello\n"), None),
        (None, Some(child_anchor(4))),
        (Some("\nworld"), None),
    ]);

    assert_eq!(
        rendered,
        "<ul><li><p>hello</p>{child:4}<p>world</p></li></ul>"
    );
}

#[test]
fn list_continuation_text_remains_inside_item_around_inline_child_anchor() {
    let rendered = markdown_formatter_output_from_text_and_anchors(&[
        (Some("- hello "), None),
        (None, Some(child_anchor(4))),
        (Some("\nworld"), None),
    ]);

    assert_eq!(rendered, "<ul><li>hello {child:4} world</li></ul>");
}

#[test]
fn list_items_support_italic_bold_and_bold_italic_at_block_start() {
    assert_eq!(
        to_markdown("- *italic*", "p"),
        "<ul><li><em>italic</em></li></ul>"
    );
    assert_eq!(
        to_markdown("- **bold**", "p"),
        "<ul><li><strong>bold</strong></li></ul>"
    );
    assert_eq!(
        to_markdown("- ***bold italic***", "p"),
        "<ul><li><em><strong>bold italic</strong></em></li></ul>"
    );
}

#[test]
fn basic_inline_code_in_paragraph() {
    let rendered = to_markdown("use `Thing` here", "p");
    assert_eq!(rendered, "<p>use <code>Thing</code> here</p>");
}

#[test]
fn multiple_inline_code_spans_on_one_line() {
    let rendered = to_markdown("`first` and `second`", "p");
    assert_eq!(
        rendered,
        "<p><code>first</code> and <code>second</code></p>"
    );
}

#[test]
fn unmatched_single_backtick_renders_literally() {
    let rendered = to_markdown("foo ` bar", "p");
    assert_eq!(rendered, "<p>foo ` bar</p>");
}

#[test]
fn consecutive_backtick_runs_render_literally() {
    let rendered = to_markdown("``not code`` and ```also not```", "p");
    assert_eq!(rendered, "<p>``not code`` and ```also not```</p>");
}

#[test]
fn trailing_backtick_run_does_not_close_code_span() {
    let rendered = to_markdown("`not code``", "p");
    assert_eq!(rendered, "<p>`not code``</p>");
}

#[test]
fn html_escaping_inside_code_spans() {
    let rendered = to_markdown("`<tag> & \"thing\"`", "p");
    assert_eq!(
        rendered,
        "<p><code>&lt;tag&gt; &amp; &quot;thing&quot;</code></p>"
    );
}

#[test]
fn emphasis_and_links_do_not_parse_inside_code_spans() {
    let rendered = to_markdown("`*bold* @/docs (Docs)`", "p");
    assert_eq!(rendered, "<p><code>*bold* @/docs (Docs)</code></p>");
}

#[test]
fn code_spans_inside_active_emphasis() {
    let rendered = to_markdown("*Use `Thing` here*", "p");
    assert_eq!(rendered, "<p><em>Use <code>Thing</code> here</em></p>");
}

#[test]
fn inline_code_in_headings() {
    let rendered = to_markdown("# `Title`", "p");
    assert_eq!(rendered, "<h1><code>Title</code></h1>");
}

#[test]
fn inline_code_in_list_items() {
    let rendered = to_markdown("- `item`", "p");
    assert_eq!(rendered, "<ul><li><code>item</code></li></ul>");
}

#[test]
fn dynamic_expression_anchor_inside_code_span() {
    let rendered = markdown_formatter_output_from_text_and_anchors(&[
        (Some("`foo "), None),
        (None, Some(dynamic_anchor(1))),
        (Some(" bar`"), None),
    ]);
    assert_eq!(rendered, "<p><code>foo {dynamic:1} bar</code></p>");
}

#[test]
fn backticks_emitted_by_dynamic_expression_anchors_do_not_participate_in_parsing() {
    // Dynamic anchors are opaque; the formatter never sees their runtime output,
    // so any backticks they might emit cannot be parsed as inline code delimiters.
    let missing_closing_delimiter = markdown_formatter_output_from_text_and_anchors(&[
        (Some("`foo "), None),
        (None, Some(dynamic_anchor(1))),
        (Some(" bar"), None),
    ]);
    assert_eq!(missing_closing_delimiter, "<p>`foo {dynamic:1} bar</p>");

    let missing_opening_delimiter = markdown_formatter_output_from_text_and_anchors(&[
        (Some("foo "), None),
        (None, Some(dynamic_anchor(1))),
        (Some(" bar`"), None),
    ]);
    assert_eq!(missing_opening_delimiter, "<p>foo {dynamic:1} bar`</p>");

    let parent_authored_delimiters = markdown_formatter_output_from_text_and_anchors(&[
        (Some("`"), None),
        (None, Some(dynamic_anchor(1))),
        (Some("`"), None),
    ]);
    assert_eq!(
        parent_authored_delimiters,
        "<p><code>{dynamic:1}</code></p>"
    );
}

#[test]
fn child_template_anchor_blocks_code_span_pairing() {
    let rendered = markdown_formatter_output_from_text_and_anchors(&[
        (Some("`foo "), None),
        (None, Some(child_anchor(1))),
        (Some(" bar`"), None),
    ]);
    assert_eq!(rendered, "<p>`foo {child:1} bar`</p>");
}

#[test]
fn code_spans_do_not_cross_soft_line_boundaries() {
    let rendered = to_markdown("`foo\nbar`", "p");
    assert_eq!(rendered, "<p>`foo bar`</p>");
}
