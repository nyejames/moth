use super::MarkdownTemplateFormatter;
use super::{MarkdownInlineAtom, MarkdownLine};
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::formatter_contract::{
    FormatterAnchorId, FormatterInput, FormatterInputPiece, FormatterOpaqueKind,
    FormatterOpaquePiece, FormatterOutputPiece, FormatterTextPiece,
};
use crate::compiler_frontend::ast::templates::styles::markdown::render_markdown_stream;
use crate::compiler_frontend::ast::templates::template::TemplateFormatter;
use crate::compiler_frontend::compiler_messages::diagnostics::diagnostic_string_builder::DiagnosticStringBuilder;
use crate::compiler_frontend::source::{ExtendedSpanBuilder, LocalSpan, SourceId, SourceSpan};
use crate::compiler_frontend::symbols::string_interning::StringTable;

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
        span: None,
    })
}

/// Splits plain text into markdown lines through the production formatter splitter.
fn split_text_into_lines(content: &str) -> Vec<MarkdownLine> {
    let mut string_table = StringTable::new();
    let input = FormatterInput {
        pieces: vec![formatter_text_piece(content, &mut string_table)],
    };

    super::split_formatter_input_into_lines(input, &string_table)
}

fn to_markdown(content: &str, default_tag: &str) -> String {
    let lines = split_text_into_lines(content);
    let pieces = render_markdown_stream(&lines, default_tag)
        .expect("markdown rendering should not produce a source error");

    pieces
        .into_iter()
        .map(|piece| match piece {
            FormatterOutputPiece::Text(text) => text,
            FormatterOutputPiece::Opaque(anchor) => match anchor.kind {
                FormatterOpaqueKind::SiteRoot => "{site-root}".to_owned(),
                _ => unreachable!("plain-text markdown helper should only emit site-root anchors"),
            },
        })
        .collect()
}

fn formatter_text_piece_with_source_span(
    text: &str,
    start: u32,
    string_table: &mut StringTable,
) -> FormatterInputPiece {
    let length = u32::try_from(text.len()).expect("test text length should fit u32");
    let local = LocalSpan::exact_inline(start, length)
        .expect("test source spans should fit inline encoding");
    FormatterInputPiece::Text(FormatterTextPiece {
        text: string_table.intern(text),
        span: Some(SourceSpan::new(SourceId::COMPILATION_ROOT, local)),
    })
}

fn formatter_text_piece_with_extended_source_span(
    text: &str,
    start: u32,
    string_table: &mut StringTable,
    span_builder: &mut ExtendedSpanBuilder,
) -> FormatterInputPiece {
    let length = u32::try_from(text.len()).expect("test text length should fit u32");
    let local = LocalSpan::exact(start, length, span_builder)
        .expect("test source span should fit the extended table");
    FormatterInputPiece::Text(FormatterTextPiece {
        text: string_table.intern(text),
        span: Some(SourceSpan::new(SourceId::COMPILATION_ROOT, local)),
    })
}

fn spanned_pieces_input(pieces: &[&str], string_table: &mut StringTable) -> FormatterInput {
    let mut start = 0u32;
    let input_pieces = pieces
        .iter()
        .map(|text| {
            let piece = formatter_text_piece_with_source_span(text, start, string_table);
            start += u32::try_from(text.len()).expect("test text length should fit u32");
            piece
        })
        .collect();
    FormatterInput {
        pieces: input_pieces,
    }
}

fn markdown_output_from_spanned_pieces(pieces: &[&str]) -> String {
    let mut string_table = StringTable::new();
    let input = spanned_pieces_input(pieces, &mut string_table);
    let mut diagnostic_strings = DiagnosticStringBuilder::new(&mut string_table);

    MarkdownTemplateFormatter
        .format(input, &mut diagnostic_strings)
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
                FormatterOpaqueKind::SiteRoot => "{site-root}".to_owned(),
            },
        })
        .collect()
}

fn markdown_error_from_spanned_pieces(pieces: &[&str]) -> TemplateError {
    let mut string_table = StringTable::new();
    let input = spanned_pieces_input(pieces, &mut string_table);
    let mut diagnostic_strings = DiagnosticStringBuilder::new(&mut string_table);

    MarkdownTemplateFormatter
        .format(input, &mut diagnostic_strings)
        .expect_err("markdown content should produce a source error")
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

    let input = FormatterInput {
        pieces: input_pieces,
    };
    let mut diagnostic_strings = DiagnosticStringBuilder::new(&mut string_table);
    MarkdownTemplateFormatter
        .format(input, &mut diagnostic_strings)
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
                FormatterOpaqueKind::SiteRoot => "{site-root}".to_owned(),
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
        let expected_target = if target.starts_with('/') && !target.starts_with("//") {
            format!("{{site-root}}{}", &target[1..])
        } else {
            target.to_owned()
        };
        let expected = format!(
            "<p><a href=\"{expected_target}\">{}</a></p>",
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
fn single_slash_link_emits_site_root_anchor_before_suffix() {
    let lines = split_text_into_lines("@/docs (Docs)");
    let pieces = render_markdown_stream(&lines, "p")
        .expect("markdown rendering should not produce a source error");

    assert_eq!(pieces.len(), 3);
    assert!(matches!(
        &pieces[0],
        FormatterOutputPiece::Text(text) if text == "<p><a href=\""
    ));
    assert!(matches!(
        &pieces[1],
        FormatterOutputPiece::Opaque(FormatterOpaquePiece {
            kind: FormatterOpaqueKind::SiteRoot,
            ..
        })
    ));
    assert!(matches!(
        &pieces[2],
        FormatterOutputPiece::Text(text) if text == "docs\">Docs</a></p>"
    ));
}

#[test]
fn non_site_root_link_targets_remain_literal_without_resource_anchors() {
    let targets = [
        "https://example.com",
        "//cdn.example.com/lib.js",
        "./local/path",
        "../parent/path",
        "#overview",
        "?q=moth",
    ];

    for target in targets {
        let input = format!("@{target} (Target)");
        let pieces = render_markdown_stream(&split_text_into_lines(&input), "p")
            .expect("markdown rendering should not produce a source error");
        let rendered = pieces
            .into_iter()
            .map(|piece| match piece {
                FormatterOutputPiece::Text(text) => text,
                FormatterOutputPiece::Opaque(anchor) => panic!(
                    "non-site-root target {target:?} emitted an opaque {:?} anchor",
                    anchor.kind
                ),
            })
            .collect::<String>();

        assert_eq!(
            rendered,
            format!("<p><a href=\"{target}\">Target</a></p>"),
            "non-site-root markdown link targets must stay literal URLs"
        );
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
fn requires_separator_before_label() {
    let rendered = to_markdown("@https://example.com(Example)", "p");
    assert_eq!(rendered, "<p>@https://example.com(Example)</p>");
}

#[test]
fn one_soft_newline_between_target_and_label_forms_link() {
    for input in [
        "@./#numeric-literals\n(Numeric literals).",
        "@./#numeric-literals \t\n\t  (Numeric literals).",
    ] {
        assert_eq!(
            to_markdown(input, "p"),
            "<p><a href=\"./#numeric-literals\">Numeric literals</a>.</p>",
            "input: {input:?}"
        );
    }
}

#[test]
fn link_separator_rejects_invalid_whitespace_runs() {
    // Blank lines split blocks before inline parsing, so the parser's own upper bound
    // is only observable on raw atoms. NBSP protects the ASCII space/tab separator set.
    for separator in ["\n\n", "\n \t\n", "\u{00a0}"] {
        let atoms: Vec<MarkdownInlineAtom> = format!("@./#target{separator}(Label)")
            .chars()
            .map(MarkdownInlineAtom::Char)
            .collect();

        assert!(
            super::parsing::try_parse_link_at_atoms(&atoms, 0).is_none(),
            "separator {separator:?} must not associate a label"
        );
    }
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
    assert_eq!(heading, "<h1><a href=\"{site-root}docs\">Docs</a></h1>");

    let emphasis = to_markdown("\n*@/docs (Docs)*\n", "p");
    assert_eq!(
        emphasis,
        "<p><em><a href=\"{site-root}docs\">Docs</a></em></p>"
    );
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
fn opaque_anchor_inside_wrapped_link_separator_falls_back_to_literal_text() {
    for (anchor, anchor_text) in [
        (child_anchor(7), "{child:7}"),
        (dynamic_anchor(9), "{dynamic:9}"),
    ] {
        let rendered = markdown_formatter_output_from_text_and_anchors(&[
            (Some("@./#target "), None),
            (None, Some(anchor)),
            (Some("\n(Label)"), None),
        ]);

        assert_eq!(rendered, format!("<p>@./#target {anchor_text} (Label)</p>"));
    }
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
    assert!(rendered.contains("<a href=\"{site-root}docs\">Docs</a>"));
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
fn soft_line_boundaries_before_normal_text_keep_paragraph_open() {
    for separator in ["\n", "\r\n", "\r"] {
        let rendered = to_markdown(&format!("hello{separator}world"), "p");
        assert_eq!(rendered, "<p>hello world</p>", "separator {separator:?}");
    }
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

#[test]
fn headings_without_markers_have_no_fragment_ids() {
    for (input, expected) in [
        ("# Hello", "<h1>Hello</h1>"),
        ("## Hello", "<h2>Hello</h2>"),
        ("### Hello", "<h3>Hello</h3>"),
        ("#### Hello", "<h4>Hello</h4>"),
        ("##### Hello", "<h5>Hello</h5>"),
        ("###### Hello", "<h6>Hello</h6>"),
    ] {
        assert_eq!(to_markdown(input, "p"), expected, "input: {input:?}");
    }
}
#[test]
fn heading_runs_longer_than_six_hashes_are_plain_text() {
    assert_eq!(to_markdown("####### Seven", "p"), "<p>####### Seven</p>");
    assert_eq!(to_markdown("######## Eight", "p"), "<p>######## Eight</p>");
}

#[test]
fn heading_markers_assign_the_heading_id_without_changing_the_label() {
    assert_eq!(
        to_markdown("# Title @#intro", "p"),
        "<h1 id=\"intro\">Title</h1>"
    );
    assert_eq!(
        to_markdown("# @#intro Title", "p"),
        "<h1 id=\"intro\">Title</h1>"
    );
    assert_eq!(
        to_markdown("# Title @#intro More", "p"),
        "<h1 id=\"intro\">Title More</h1>"
    );
}

#[test]
fn explicit_section_ids_preserve_authored_case_and_separators() {
    assert_eq!(
        to_markdown("# Title @#Hello-World_42", "p"),
        "<h1 id=\"Hello-World_42\">Title</h1>"
    );
    assert_eq!(
        to_markdown("# Title @#Café42", "p"),
        "<h1 id=\"Café42\">Title</h1>"
    );
}

#[test]
fn heading_markers_work_inside_formatted_spans() {
    assert_eq!(
        to_markdown("# **Bold** title @#intro", "p"),
        "<h1 id=\"intro\"><strong>Bold</strong> title</h1>"
    );
    assert_eq!(
        to_markdown("# `String values` @#code", "p"),
        "<h1 id=\"code\"><code>String values</code></h1>"
    );
    assert_eq!(
        to_markdown("# @/docs/other-words (Templates) @#templates", "p"),
        "<h1 id=\"templates\"><a href=\"{site-root}docs/other-words\">Templates</a></h1>"
    );
}
#[test]
fn opaque_anchors_render_inside_headings_without_an_automatic_id() {
    // A line-leading anchor is a paragraph boundary, not heading content, so every
    // probe below keeps the anchor inside one `#` heading line.
    for (anchor, placeholder) in [
        (dynamic_anchor(1), "{dynamic:1}"),
        (child_anchor(7), "{child:7}"),
    ] {
        for pieces in [
            vec![
                (Some("# "), None),
                (None, Some(anchor)),
                (Some("Hello"), None),
            ],
            vec![
                (Some("# Hello "), None),
                (None, Some(anchor)),
                (Some(" world"), None),
            ],
            vec![(Some("# Hello "), None), (None, Some(anchor))],
        ] {
            let rendered = markdown_formatter_output_from_text_and_anchors(&pieces);
            assert!(
                rendered.starts_with("<h1>") && !rendered.contains("id="),
                "opaque anchor {placeholder} must not create an id: {rendered:?}"
            );
            assert!(
                rendered.contains(placeholder),
                "anchor body must stay unchanged: {rendered:?}"
            );
        }
    }
}
#[test]
fn dynamic_anchor_inside_heading_code_span_renders_opaque_placeholder() {
    let rendered = markdown_formatter_output_from_text_and_anchors(&[
        (Some("# `foo "), None),
        (None, Some(dynamic_anchor(1))),
        (Some(" bar`"), None),
    ]);

    assert_eq!(rendered, "<h1><code>foo {dynamic:1} bar</code></h1>");
}
#[test]
fn explicit_section_ids_are_not_suffixed_for_repeated_headings() {
    assert_eq!(
        to_markdown("# Hello @#same\n\n# Hello @#same", "p"),
        "<h1 id=\"same\">Hello</h1><h1 id=\"same\">Hello</h1>"
    );
}
#[test]
fn plain_paragraphs_and_list_items_without_markers_have_no_ids() {
    assert_eq!(to_markdown("Hello World", "p"), "<p>Hello World</p>");
    assert_eq!(
        to_markdown("- Hello World", "p"),
        "<ul><li>Hello World</li></ul>"
    );
}
fn markdown_error(content: &str) -> TemplateError {
    render_markdown_stream(&split_text_into_lines(content), "p")
        .expect_err("markdown content should produce a source error")
}

#[test]
fn section_marker_is_zero_width_and_removes_an_adjacent_separator() {
    assert_eq!(to_markdown("A @#intro B", "p"), "<p id=\"intro\">A B</p>");
    assert_eq!(
        to_markdown("@#intro\nHello", "p"),
        "<p id=\"intro\">Hello</p>"
    );
    assert_eq!(
        to_markdown("Hello @#intro", "p"),
        "<p id=\"intro\">Hello</p>"
    );
}

#[test]
fn marker_consumes_lf_crlf_and_cr_as_one_separator() {
    for separator in ["\n", "\r\n", "\r"] {
        assert_eq!(
            to_markdown(&format!("@#intro{separator}Hello"), "p"),
            "<p id=\"intro\">Hello</p>",
            "separator at block start {separator:?}"
        );
        assert_eq!(
            to_markdown(&format!("Hello @#intro{separator}world"), "p"),
            "<p id=\"intro\">Hello world</p>",
            "separator after text {separator:?}"
        );
        assert_eq!(
            to_markdown(&format!("Hello{separator}@#intro world"), "p"),
            "<p id=\"intro\">Hello world</p>",
            "separator before marker {separator:?}"
        );
    }
}

#[test]
fn section_marker_requires_whitespace_or_block_start_before_the_at_sign() {
    assert_eq!(to_markdown("email@#intro", "p"), "<p>email@#intro</p>");
}

#[test]
fn marker_does_not_leave_empty_emphasis_markup() {
    assert_eq!(to_markdown("*@#intro*", "p"), "<span id=\"intro\"></span>");
    assert_eq!(to_markdown("A *@#intro*", "p"), "<p id=\"intro\">A</p>");
    assert_eq!(to_markdown("A **@#intro**", "p"), "<p id=\"intro\">A</p>");
    assert_eq!(to_markdown("A *@#intro* B", "p"), "<p id=\"intro\">A B</p>");
    assert_eq!(
        to_markdown("*A @#intro*", "p"),
        "<p id=\"intro\"><em>A</em></p>"
    );
    assert_eq!(
        to_markdown("**A @#intro**", "p"),
        "<p id=\"intro\"><strong>A</strong></p>"
    );
    assert_eq!(
        to_markdown("***A @#intro***", "p"),
        "<p id=\"intro\"><em><strong>A</strong></em></p>"
    );
}

#[test]
fn marker_candidate_inside_a_link_label_stays_literal() {
    assert_eq!(
        to_markdown("@https://example.com (#intro)", "p"),
        "<p><a href=\"https://example.com\">#intro</a></p>"
    );
}

#[test]
fn invalid_section_marker_reports_a_source_error() {
    assert!(matches!(
        markdown_error("A @#intro/name"),
        TemplateError::Diagnostic(_)
    ));
}

#[test]
fn duplicate_markers_in_one_paragraph_report_a_source_error() {
    assert!(matches!(
        markdown_error("A @#one B @#two"),
        TemplateError::Diagnostic(_)
    ));
}

#[test]
fn duplicate_markers_in_separate_paragraphs_are_allowed() {
    assert_eq!(
        to_markdown("A @#intro\n\nB @#intro", "p"),
        "<p id=\"intro\">A</p><p id=\"intro\">B</p>"
    );
}

#[test]
fn empty_marker_id_before_emphasis_close_reports_a_source_error() {
    assert!(matches!(
        markdown_error("*@#*"),
        TemplateError::Diagnostic(_)
    ));
}

#[test]
fn list_items_use_one_marker_for_the_whole_item() {
    assert_eq!(
        to_markdown("- Item @#intro", "p"),
        "<ul><li id=\"intro\">Item</li></ul>"
    );

    assert!(matches!(
        markdown_error("- First @#one\n  continued @#two"),
        TemplateError::Diagnostic(_)
    ));
}

#[test]
fn headings_reject_duplicate_markers() {
    assert!(matches!(
        markdown_error("# Title @#one @#two"),
        TemplateError::Diagnostic(_)
    ));
}

#[test]
fn empty_paragraph_marker_emits_a_standalone_span_when_authored() {
    assert_eq!(
        markdown_output_from_spanned_pieces(&["@#release"]),
        "<span id=\"release\"></span>"
    );
    assert_eq!(
        markdown_output_from_spanned_pieces(&["@#release\n\nTail"]),
        "<span id=\"release\"></span><p>Tail</p>"
    );
}

#[test]
fn spanned_marker_on_text_or_headings_prefers_the_owning_element() {
    assert_eq!(
        markdown_output_from_spanned_pieces(&["A @#release B"]),
        "<p id=\"release\">A B</p>"
    );
    assert_eq!(
        markdown_output_from_spanned_pieces(&["# @#release"]),
        "<h1 id=\"release\"></h1>"
    );
}

#[test]
fn list_item_marker_uses_the_li_even_when_the_item_is_empty() {
    assert_eq!(
        markdown_output_from_spanned_pieces(&["- @#release"]),
        "<ul><li id=\"release\"></li></ul>"
    );
}

#[test]
fn marker_span_falling_back_to_its_piece_still_emits_a_standalone_span() {
    assert_eq!(
        markdown_output_from_spanned_pieces(&["@", "#release"]),
        "<span id=\"release\"></span>"
    );
}

#[test]
fn synthetic_marker_without_a_source_span_still_emits_a_standalone_span() {
    assert_eq!(
        to_markdown("@#release", "p"),
        "<span id=\"release\"></span>"
    );
}

#[test]
fn section_marker_syntax_inside_code_spans_stays_literal() {
    assert_eq!(
        to_markdown("# `@#release`", "p"),
        "<h1><code>@#release</code></h1>"
    );
}

fn marker_error_byte_range(error: TemplateError) -> (u32, u32) {
    let TemplateError::Diagnostic(diagnostic) = error else {
        panic!("marker errors should be source diagnostics");
    };
    let span = diagnostic
        .primary_span()
        .expect("marker error should carry a primary span");
    let range = span
        .local()
        .inline_range()
        .expect("test marker spans should use inline encoding");
    (range.start(), range.end())
}

#[test]
fn invalid_marker_primary_span_covers_the_offending_syntax() {
    assert_eq!(
        marker_error_byte_range(markdown_error_from_spanned_pieces(&["A @#bad/name"])),
        (2, 8)
    );
    assert_eq!(
        marker_error_byte_range(markdown_error_from_spanned_pieces(&["A @#1x"])),
        (2, 5)
    );
    assert_eq!(
        marker_error_byte_range(markdown_error_from_spanned_pieces(&["A @#"])),
        (2, 4)
    );
}

#[test]
fn duplicate_marker_primary_span_is_the_second_marker() {
    let error = markdown_error_from_spanned_pieces(&["A @#one B @#two"]);
    let TemplateError::Diagnostic(diagnostic) = &error else {
        panic!("duplicate marker should be a source diagnostic");
    };
    let earlier = diagnostic
        .label_span(0)
        .expect("duplicate marker should label the earlier marker");
    let earlier_range = earlier
        .local()
        .inline_range()
        .expect("test marker spans should use inline encoding");

    assert_eq!(marker_error_byte_range(error), (10, 15));
    assert_eq!((earlier_range.start(), earlier_range.end()), (2, 7));
}

#[test]
fn marker_span_crossing_pieces_falls_back_to_the_containing_piece() {
    let error = markdown_error_from_spanned_pieces(&["@", "#bad/"]);
    assert_eq!(marker_error_byte_range(error), (0, 1));
}

#[test]
fn section_marker_inside_emphasis_decorates_the_paragraph() {
    assert_eq!(
        to_markdown("A **bold @#inside text**", "p"),
        "<p id=\"inside\">A <strong>bold text</strong></p>"
    );
}

#[test]
fn section_marker_adjacent_to_an_opaque_child_decorates_the_paragraph() {
    let rendered = markdown_formatter_output_from_text_and_anchors(&[
        (Some("A "), None),
        (None, Some(child_anchor(1))),
        (Some(" @#inside B"), None),
    ]);

    assert_eq!(rendered, "<p id=\"inside\">A {child:1} B</p>");
}

#[test]
fn ordered_nested_and_continuation_list_items_keep_their_own_ids() {
    assert_eq!(
        to_markdown("1. Item @#ordered", "p"),
        "<ol><li id=\"ordered\">Item</li></ol>"
    );
    assert_eq!(
        to_markdown("- Parent @#parent\n  - Child @#child", "p"),
        "<ul><li id=\"parent\">Parent<ul><li id=\"child\">Child</li></ul></li></ul>"
    );
    assert_eq!(
        to_markdown("- Item @#item\ncontinuation", "p"),
        "<ul><li id=\"item\">Item continuation</li></ul>"
    );
}

#[test]
fn every_heading_level_accepts_a_marker_at_start_middle_and_end() {
    for level in 1..=6usize {
        let hashes = "#".repeat(level);
        for (input, expected_label) in [
            (format!("{hashes} @#sample Title"), "Title"),
            (format!("{hashes} Start @#sample Middle"), "Start Middle"),
            (format!("{hashes} Title @#sample"), "Title"),
        ] {
            assert_eq!(
                to_markdown(&input, "p"),
                format!("<h{level} id=\"sample\">{expected_label}</h{level}>"),
                "input: {input:?}"
            );
        }
    }
}

#[test]
fn marker_only_blocks_stay_separate_before_after_and_between_other_blocks() {
    assert_eq!(
        markdown_output_from_spanned_pieces(&["@#before\n# Heading\n@#after"]),
        "<span id=\"before\"></span><h1>Heading</h1><span id=\"after\"></span>"
    );
    assert_eq!(
        markdown_output_from_spanned_pieces(&["Text\n\n@#middle\n\nMore"]),
        "<p>Text</p><span id=\"middle\"></span><p>More</p>"
    );
}

#[test]
fn marker_without_a_valid_terminator_reports_a_source_error() {
    assert!(matches!(
        markdown_error("A @#intro* B"),
        TemplateError::Diagnostic(_)
    ));
}

#[test]
fn marker_diagnostic_byte_mapping_keeps_non_ascii_prefixes() {
    assert_eq!(
        marker_error_byte_range(markdown_error_from_spanned_pieces(&["Café @#bad/name"])),
        (6, 12)
    );
}

#[test]
fn marker_diagnostic_in_a_long_piece_falls_back_to_the_authored_piece() {
    let mut string_table = StringTable::new();
    let mut span_builder = ExtendedSpanBuilder::new();
    let text = format!("{} @#bad/name", "é".repeat(600));
    let input = FormatterInput {
        pieces: vec![formatter_text_piece_with_extended_source_span(
            &text,
            0,
            &mut string_table,
            &mut span_builder,
        )],
    };
    let mut diagnostic_strings = DiagnosticStringBuilder::new(&mut string_table);

    let error = MarkdownTemplateFormatter
        .format(input, &mut diagnostic_strings)
        .expect_err("the invalid marker should produce a source error");
    let TemplateError::Diagnostic(diagnostic) = error else {
        panic!("marker errors should be source diagnostics");
    };
    let span = diagnostic
        .primary_span()
        .expect("marker error should carry a primary span");
    let range = span.local().resolve_with(span_builder.resolver());

    assert_eq!(
        (range.start(), range.end()),
        (
            0,
            u32::try_from(text.len()).expect("test text length should fit u32")
        )
    );
}

#[test]
fn list_item_marker_never_migrates_to_a_following_paragraph() {
    assert_eq!(
        to_markdown("- First @#item\n\n  Second @#second", "p"),
        "<ul><li id=\"item\">First</li></ul><p id=\"second\">  Second</p>"
    );
}

#[test]
fn list_item_marker_decorates_the_li_not_an_inner_paragraph() {
    let rendered = markdown_formatter_output_from_text_and_anchors(&[
        (Some("- "), None),
        (None, Some(child_anchor(1))),
        (Some("\n  text @#item"), None),
    ]);

    assert_eq!(
        rendered,
        "<ul><li id=\"item\">{child:1}<p>text</p></li></ul>"
    );
}

#[test]
fn labelled_link_accepts_one_soft_newline_separator() {
    for separator in ["\n", "\r\n", "\r"] {
        let input = format!("See @#intro \t{separator} (Introduction)");
        assert_eq!(
            to_markdown(&input, "p"),
            "<p>See <a href=\"#intro\">Introduction</a></p>",
            "separator {separator:?}"
        );
    }
}

#[test]
fn marker_like_target_with_empty_or_unclosed_label_stays_literal() {
    assert_eq!(to_markdown("@#intro ()", "p"), "<p>@#intro ()</p>");
    assert_eq!(to_markdown("@#intro (Intro", "p"), "<p>@#intro (Intro</p>");
}

#[test]
fn marker_rejects_empty_or_nonletter_starting_ids() {
    for input in ["A @# bad", "A @#_id", "A @#-id", "A @#1id"] {
        assert!(
            matches!(markdown_error(input), TemplateError::Diagnostic(_)),
            "input should be rejected: {input:?}"
        );
    }
}

#[test]
fn bare_non_fragment_url_without_a_label_stays_literal() {
    assert_eq!(
        to_markdown("Visit @https://example.com", "p"),
        "<p>Visit @https://example.com</p>"
    );
}
