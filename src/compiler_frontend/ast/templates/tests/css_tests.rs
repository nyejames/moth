use super::*;
use crate::compiler_frontend::ast::templates::formatter_contract::{
    FormatterAnchorId, FormatterInput, FormatterInputPiece, FormatterOpaqueKind,
    FormatterOpaquePiece, FormatterOutputPiece, FormatterTextPiece,
};
use crate::compiler_frontend::compiler_messages::{CssTemplateWarning, MalformedTemplateReason};
use crate::compiler_frontend::source::{ExtendedSpanBuilder, LocalSpan, SourceId, SourceSpan};

fn located_text_piece(text: &str, string_table: &mut StringTable) -> FormatterInputPiece {
    FormatterInputPiece::Text(FormatterTextPiece {
        text: string_table.intern(text),
        span: None,
    })
}

fn spanned_text_piece(
    text: &str,
    span: SourceSpan,
    string_table: &mut StringTable,
) -> FormatterInputPiece {
    FormatterInputPiece::Text(FormatterTextPiece {
        text: string_table.intern(text),
        span: Some(span),
    })
}

fn validate_css_for_test(source: &str, mode: CssFormatterMode) -> Vec<SourceWarning> {
    let mut string_table = StringTable::new();
    validate_css_source(source, mode, &mut string_table)
}

#[test]
fn valid_block_css_emits_no_warnings() {
    let warnings = validate_css_for_test(
        ".button { color: red; }\n@media (width > 600px) { .button { padding: 1rem; } }",
        CssFormatterMode::Block,
    );
    assert!(warnings.is_empty());
}

#[test]
fn valid_block_css_ignores_comments_inside_statements() {
    let warnings = validate_css_for_test(
        ":root { /* Default Background Colours */ --moth-bg-lightmode: #fff; /* Code block colours */ --comment-dark: #838c86; }",
        CssFormatterMode::Block,
    );
    assert!(warnings.is_empty());
}

#[test]
fn inline_css_rejects_selector_blocks() {
    let warnings = validate_css_for_test(".button { color: red; }", CssFormatterMode::Inline);
    assert!(warnings.iter().any(|warning| {
        warning.reason == MalformedTemplateReason::Css(CssTemplateWarning::InlineSelectorBlock)
    }));
}

#[test]
fn malformed_css_reports_balancing_and_declaration_shape() {
    let warnings = validate_css_for_test(".button { color red; ", CssFormatterMode::Block);
    assert!(warnings.iter().any(|warning| {
        warning.reason
            == MalformedTemplateReason::Css(CssTemplateWarning::UnclosedDelimiter { opening: '{' })
    }));
    assert!(warnings.iter().any(|warning| {
        warning.reason == MalformedTemplateReason::Css(CssTemplateWarning::MalformedDeclaration)
    }));
}

#[test]
fn css_formatter_preserves_structural_anchors_and_maps_warnings_to_authored_text() {
    let mut string_table = StringTable::new();

    // A structural ConstStringPiece::Resource reaches the formatter view as a
    // DynamicExpression node. The existing DynamicExpression opaque kind is
    // therefore the resource-anchor representation; CSS must preserve it
    // rather than require a FormatterOpaqueKind::Resource.
    let input = FormatterInput {
        pieces: vec![
            located_text_piece(".button { background: url(\"", &mut string_table),
            FormatterInputPiece::Opaque(FormatterOpaquePiece {
                id: FormatterAnchorId(0),
                kind: FormatterOpaqueKind::DynamicExpression,
            }),
            located_text_piece("\"); color: red; ", &mut string_table),
            FormatterInputPiece::Opaque(FormatterOpaquePiece {
                id: FormatterAnchorId(1),
                kind: FormatterOpaqueKind::SiteRoot,
            }),
            located_text_piece("bad; }", &mut string_table),
        ],
    };

    let result = css_validation_formatter(CssFormatterMode::Block)
        .formatter
        .format(input, &mut string_table)
        .expect("CSS formatter should preserve structural anchors");

    assert_eq!(
        result.warnings.len(),
        1,
        "the malformed declaration after the anchors should be diagnosed"
    );

    let output = &result.output.pieces;
    assert_eq!(
        output.len(),
        5,
        "formatter output must retain both text and opaque pieces"
    );
    assert!(matches!(
        &output[0],
        FormatterOutputPiece::Text(text) if text == ".button { background: url(\""
    ));
    assert!(matches!(
        &output[1],
        FormatterOutputPiece::Opaque(anchor)
            if anchor.id == FormatterAnchorId(0)
                && anchor.kind == FormatterOpaqueKind::DynamicExpression
    ));
    assert!(matches!(
        &output[2],
        FormatterOutputPiece::Text(text) if text == "\"); color: red; "
    ));
    assert!(matches!(
        &output[3],
        FormatterOutputPiece::Opaque(anchor)
            if anchor.id == FormatterAnchorId(1)
                && anchor.kind == FormatterOpaqueKind::SiteRoot
    ));
    assert!(matches!(
        &output[4],
        FormatterOutputPiece::Text(text) if text == "bad; }"
    ));
}

#[test]
fn strip_css_comments_preserves_comment_markers_inside_quotes() {
    assert_eq!(strip_css_comments(r#"content: "/*";"#), r#"content: "/*";"#);
    assert_eq!(strip_css_comments("content: '/*';"), "content: '/*';");
    assert_eq!(
        strip_css_comments(r#"content: "*/"; content: '*/';"#),
        r#"content: "*/"; content: '*/';"#,
    );
    // `//` is not a CSS comment marker.
    assert_eq!(strip_css_comments("a // b"), "a // b");
}

#[test]
fn strip_css_comments_handles_escapes_adjacent_and_unterminated_comments() {
    // An escaped quote does not close the string, so the marker stays quoted.
    assert_eq!(strip_css_comments(r#""a\"/*""#), r#""a\"/*""#);
    assert_eq!(strip_css_comments(r"'a\'/*'"), r"'a\'/*'");
    // An escaped backslash does not escape the quote: the string closes and
    // the following comment is stripped.
    assert_eq!(strip_css_comments(r#""a\\"/*x*/b"#), r#""a\\"b"#);
    // Adjacent comments collapse completely.
    assert_eq!(strip_css_comments("a/*x*//*y*/b"), "ab");
    assert_eq!(strip_css_comments("a/**/b"), "ab");
    // Unterminated comments run to the end of the input; an unterminated
    // string is preserved as authored.
    assert_eq!(strip_css_comments("a/*b"), "a");
    assert_eq!(strip_css_comments("/*unterminated"), "");
    assert_eq!(strip_css_comments("\"abc"), "\"abc");
}

#[test]
fn strip_css_comments_preserves_unicode_text() {
    assert_eq!(strip_css_comments("café /* x */: red"), "café : red");
    assert_eq!(strip_css_comments("a /* café */ b"), "a  b");
}

#[test]
fn missing_colon_reports_malformed_declaration_with_scalar_offsets() {
    let warnings = validate_css_for_test(".a { color red; }", CssFormatterMode::Block);
    assert_eq!(warnings.len(), 1);
    assert_eq!(
        warnings[0].reason,
        MalformedTemplateReason::Css(CssTemplateWarning::MalformedDeclaration)
    );
    assert_eq!((warnings[0].start_offset, warnings[0].end_offset), (5, 14));
}

#[test]
fn empty_declaration_value_pins_first_colon_offset() {
    let mut string_table = StringTable::new();
    let warnings = validate_css_source(
        ".a { color: ; }",
        CssFormatterMode::Block,
        &mut string_table,
    );
    assert_eq!(warnings.len(), 1);
    assert_eq!(
        warnings[0].reason,
        MalformedTemplateReason::Css(CssTemplateWarning::MissingDeclarationValue)
    );
    assert_eq!((warnings[0].start_offset, warnings[0].end_offset), (10, 11));
}

#[test]
fn multiple_colons_split_at_first_colon() {
    let warnings = validate_css_for_test(".a { filter: a: b; }", CssFormatterMode::Block);
    assert!(warnings.is_empty());
}

#[test]
fn combined_invalid_name_and_missing_value_keep_warning_order() {
    let mut string_table = StringTable::new();
    let warnings = validate_css_source(".a { 123:; }", CssFormatterMode::Block, &mut string_table);
    assert_eq!(warnings.len(), 2);
    assert_eq!(
        warnings[0].reason,
        MalformedTemplateReason::Css(CssTemplateWarning::InvalidPropertyName {
            name: string_table.intern("123"),
        })
    );
    assert_eq!((warnings[0].start_offset, warnings[0].end_offset), (5, 8));
    assert_eq!(
        warnings[1].reason,
        MalformedTemplateReason::Css(CssTemplateWarning::MissingDeclarationValue)
    );
    assert_eq!((warnings[1].start_offset, warnings[1].end_offset), (8, 9));

    // Empty property and empty value warn in the same order on the same span.
    let warnings = validate_css_source(".a { :; }", CssFormatterMode::Block, &mut string_table);
    assert_eq!(warnings.len(), 2);
    assert_eq!(
        warnings[0].reason,
        MalformedTemplateReason::Css(CssTemplateWarning::InvalidPropertyName {
            name: string_table.intern(""),
        })
    );
    assert_eq!((warnings[0].start_offset, warnings[0].end_offset), (5, 6));
    assert_eq!(
        warnings[1].reason,
        MalformedTemplateReason::Css(CssTemplateWarning::MissingDeclarationValue)
    );
    assert_eq!((warnings[1].start_offset, warnings[1].end_offset), (5, 6));
}

#[test]
fn declaration_offsets_count_unicode_scalars_not_bytes() {
    let mut string_table = StringTable::new();
    let warnings = validate_css_source(".a { café:; }", CssFormatterMode::Block, &mut string_table);
    assert_eq!(warnings.len(), 2);
    assert_eq!(
        warnings[0].reason,
        MalformedTemplateReason::Css(CssTemplateWarning::InvalidPropertyName {
            name: string_table.intern("café"),
        })
    );
    assert_eq!((warnings[0].start_offset, warnings[0].end_offset), (5, 9));
    assert_eq!(
        warnings[1].reason,
        MalformedTemplateReason::Css(CssTemplateWarning::MissingDeclarationValue)
    );
    assert_eq!((warnings[1].start_offset, warnings[1].end_offset), (9, 10));
}

#[test]
fn css_formatter_passes_through_comments_and_maps_warning_to_source_piece() {
    let mut string_table = StringTable::new();
    let mut span_builder = ExtendedSpanBuilder::new();
    let first_span = SourceSpan::new(
        SourceId::from_index(1),
        LocalSpan::exact(0, 5, &mut span_builder).expect("first piece span should fit"),
    );
    let second_span = SourceSpan::new(
        SourceId::from_index(1),
        LocalSpan::exact(5, 17, &mut span_builder).expect("second piece span should fit"),
    );
    let input = FormatterInput {
        pieces: vec![
            spanned_text_piece(".a { ", first_span, &mut string_table),
            spanned_text_piece("/* keep */ bad; }", second_span, &mut string_table),
        ],
    };

    let result = css_validation_formatter(CssFormatterMode::Block)
        .formatter
        .format(input, &mut string_table)
        .expect("CSS formatter should pass comments through");

    // Comments are analysis-only: output text is identical to the input text.
    let output = &result.output.pieces;
    assert_eq!(output.len(), 2);
    assert!(matches!(
        &output[0],
        FormatterOutputPiece::Text(text) if text == ".a { "
    ));
    assert!(matches!(
        &output[1],
        FormatterOutputPiece::Text(text) if text == "/* keep */ bad; }"
    ));

    // The `bad` declaration warns once, attributed to the piece that authored it.
    assert_eq!(result.warnings.len(), 1);
    assert_eq!(result.warnings[0].primary_span, Some(second_span));
}
