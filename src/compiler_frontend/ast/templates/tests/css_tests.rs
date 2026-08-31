use super::*;
use crate::compiler_frontend::ast::templates::formatter_contract::{
    FormatterAnchorId, FormatterInput, FormatterInputPiece, FormatterOpaqueKind,
    FormatterOpaquePiece, FormatterOutputPiece, FormatterTextPiece,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{CharPosition, SourceLocation};

fn located_text_piece(
    text: &str,
    line_number: i32,
    char_column: i32,
    string_table: &mut StringTable,
) -> FormatterInputPiece {
    let text_len = text.chars().count() as i32;
    FormatterInputPiece::Text(FormatterTextPiece {
        text: string_table.intern(text),
        location: SourceLocation::new(
            Default::default(),
            CharPosition {
                line_number,
                char_column,
            },
            CharPosition {
                line_number,
                char_column: char_column + text_len,
            },
        ),
    })
}

#[test]
fn valid_block_css_emits_no_warnings() {
    let warnings = validate_css_source(
        ".button { color: red; }\n@media (width > 600px) { .button { padding: 1rem; } }",
        CssFormatterMode::Block,
    );

    assert!(warnings.is_empty());
}

#[test]
fn valid_block_css_ignores_comments_inside_statements() {
    let warnings = validate_css_source(
        ":root { /* Default Background Colours */ --moth-bg-lightmode: #fff; /* Code block colours */ --comment-dark: #838c86; }",
        CssFormatterMode::Block,
    );

    assert!(warnings.is_empty());
}

#[test]
fn inline_css_rejects_selector_blocks() {
    let warnings = validate_css_source(".button { color: red; }", CssFormatterMode::Inline);
    assert!(
        warnings
            .iter()
            .any(|warning| warning.message.contains("only allow declarations"))
    );
}

#[test]
fn malformed_css_reports_balancing_and_declaration_shape() {
    let warnings = validate_css_source(".button { color red; ", CssFormatterMode::Block);
    assert!(
        warnings
            .iter()
            .any(|warning| warning.message.contains("Unclosed '{'"))
    );
    assert!(
        warnings
            .iter()
            .any(|warning| warning.message.contains("Expected 'property: value'"))
    );
}

#[test]
fn css_formatter_preserves_structural_anchors_and_maps_warnings_to_authored_text() {
    let mut string_table = StringTable::new();
    let tail_location = SourceLocation::new(
        Default::default(),
        CharPosition {
            line_number: 12,
            char_column: 4,
        },
        CharPosition {
            line_number: 12,
            char_column: 10,
        },
    );

    // A structural ConstStringPiece::Resource reaches the formatter view as a
    // DynamicExpression node. The existing DynamicExpression opaque kind is
    // therefore the resource-anchor representation; CSS must preserve it
    // rather than require a FormatterOpaqueKind::Resource.
    let input = FormatterInput {
        pieces: vec![
            located_text_piece(".button { background: url(\"", 2, 3, &mut string_table),
            FormatterInputPiece::Opaque(FormatterOpaquePiece {
                id: FormatterAnchorId(0),
                kind: FormatterOpaqueKind::DynamicExpression,
            }),
            located_text_piece("\"); color: red; ", 4, 5, &mut string_table),
            FormatterInputPiece::Opaque(FormatterOpaquePiece {
                id: FormatterAnchorId(1),
                kind: FormatterOpaqueKind::SiteRoot,
            }),
            located_text_piece("bad; }", 12, 4, &mut string_table),
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
    let warning = &result.warnings[0];
    assert_eq!(warning.primary_location.scope, tail_location.scope);
    assert_eq!(warning.primary_location.start_pos, tail_location.start_pos);
    assert_eq!(
        warning.primary_location.end_pos,
        CharPosition {
            line_number: 12,
            char_column: 6,
        },
        "warning span should map to the authored `bad` text, not flattened offsets"
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
