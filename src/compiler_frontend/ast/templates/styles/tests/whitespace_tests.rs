use super::super::whitespace::{
    TemplateBodyRunPosition, TemplateWhitespacePass, TemplateWhitespacePassProfile,
    apply_whitespace_passes_to_input,
};
use crate::compiler_frontend::ast::templates::formatter_contract::{
    FormatterAnchorId, FormatterInput, FormatterInputPiece, FormatterOpaqueKind,
    FormatterOpaquePiece, FormatterOutputPiece, FormatterTextPiece,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;

fn text_piece(text: &str, string_table: &mut StringTable) -> FormatterInputPiece {
    FormatterInputPiece::Text(FormatterTextPiece {
        text: string_table.intern(text),
        span: None,
    })
}

fn apply_default_template_body_whitespace(input: &str) -> String {
    let mut string_table = StringTable::new();
    let input_id = string_table.intern(input);
    let output = apply_whitespace_passes_to_input(
        FormatterInput {
            pieces: vec![FormatterInputPiece::Text(FormatterTextPiece {
                text: input_id,
                span: None,
            })],
        },
        &[TemplateWhitespacePassProfile::default_template_body()],
        TemplateBodyRunPosition::Only,
        &mut string_table,
    );

    output
        .pieces
        .into_iter()
        .map(|piece| match piece {
            FormatterOutputPiece::Text(text) => text,
            FormatterOutputPiece::Opaque(_) => panic!("whitespace pass should preserve text input"),
        })
        .collect()
}

#[test]
fn leading_dedent_uses_first_content_indentation() {
    let normalized = apply_default_template_body_whitespace(
        "\n        first\n                nested\n    less_indented\n            final\n        ",
    );

    assert_eq!(
        normalized,
        "first\n        nested\nless_indented\n    final"
    );
}

#[test]
fn leading_blank_lines_do_not_change_content_baseline() {
    let normalized = apply_default_template_body_whitespace(
        "\n    \n\n            \n        first\n            nested\n        ",
    );

    assert_eq!(normalized, "\n\n\nfirst\n    nested");
}

#[test]
fn inline_body_without_leading_boundary_is_unchanged() {
    let input = "first\n    nested";

    assert_eq!(apply_default_template_body_whitespace(input), input);
}

#[test]
fn no_pass_preserves_text_and_opaque_pieces() {
    let mut string_table = StringTable::new();
    let anchor = FormatterOpaquePiece {
        id: FormatterAnchorId(7),
        kind: FormatterOpaqueKind::DynamicExpression,
    };
    let output = apply_whitespace_passes_to_input(
        FormatterInput {
            pieces: vec![
                text_piece("  unchanged", &mut string_table),
                FormatterInputPiece::Opaque(anchor),
            ],
        },
        &[],
        TemplateBodyRunPosition::Only,
        &mut string_table,
    );

    assert!(matches!(
        &output.pieces[0],
        FormatterOutputPiece::Text(text) if text == "  unchanged"
    ));
    assert!(matches!(
        &output.pieces[1],
        FormatterOutputPiece::Opaque(actual) if *actual == anchor
    ));

    let empty_output = apply_whitespace_passes_to_input(
        FormatterInput { pieces: vec![] },
        &[TemplateWhitespacePassProfile::default_template_body()],
        TemplateBodyRunPosition::Only,
        &mut string_table,
    );
    assert!(empty_output.pieces.is_empty());
}

#[test]
fn run_position_controls_boundary_trimming() {
    let input = "\n    first\n    ";
    let expected = [
        (TemplateBodyRunPosition::Only, "first"),
        (TemplateBodyRunPosition::First, "first\n"),
        (TemplateBodyRunPosition::Middle, "\nfirst\n"),
        (TemplateBodyRunPosition::Last, "\nfirst"),
    ];

    for (run_position, expected) in expected {
        let mut string_table = StringTable::new();
        let output = apply_whitespace_passes_to_input(
            FormatterInput {
                pieces: vec![text_piece(input, &mut string_table)],
            },
            &[TemplateWhitespacePassProfile::default_template_body()],
            run_position,
            &mut string_table,
        );

        assert!(matches!(
            output.pieces.as_slice(),
            [FormatterOutputPiece::Text(text)] if text == expected
        ));
    }
}

#[test]
fn whitespace_passes_apply_sequentially() {
    let mut string_table = StringTable::new();
    let output = apply_whitespace_passes_to_input(
        FormatterInput {
            pieces: vec![text_piece(
                "\n    first\n        nested\n    ",
                &mut string_table,
            )],
        },
        &[
            TemplateWhitespacePassProfile::new(
                TemplateWhitespacePass::DefaultTemplateBody,
                false,
                false,
            ),
            TemplateWhitespacePassProfile::default_template_body(),
        ],
        TemplateBodyRunPosition::Only,
        &mut string_table,
    );

    assert!(matches!(
        output.pieces.as_slice(),
        [FormatterOutputPiece::Text(text)] if text == "first\n    nested"
    ));
}

#[test]
fn empty_text_around_opaque_anchors_preserves_separator_whitespace() {
    for (anchor_id, kind) in [
        FormatterOpaqueKind::ChildTemplate,
        FormatterOpaqueKind::DynamicExpression,
        FormatterOpaqueKind::SiteRoot,
    ]
    .into_iter()
    .enumerate()
    {
        let mut string_table = StringTable::new();
        let anchor = FormatterOpaquePiece {
            id: FormatterAnchorId(anchor_id),
            kind,
        };
        let output = apply_whitespace_passes_to_input(
            FormatterInput {
                pieces: vec![
                    text_piece("", &mut string_table),
                    FormatterInputPiece::Opaque(anchor),
                    text_piece("\n    after\n    ", &mut string_table),
                    text_piece("", &mut string_table),
                ],
            },
            &[TemplateWhitespacePassProfile::default_template_body()],
            TemplateBodyRunPosition::Only,
            &mut string_table,
        );

        assert_eq!(output.pieces.len(), 4);
        assert!(matches!(
            &output.pieces[0],
            FormatterOutputPiece::Text(text) if text.is_empty()
        ));
        assert!(matches!(
            &output.pieces[1],
            FormatterOutputPiece::Opaque(actual) if *actual == anchor
        ));
        assert!(matches!(
            &output.pieces[2],
            FormatterOutputPiece::Text(text) if text == "\nafter"
        ));
        assert!(matches!(
            &output.pieces[3],
            FormatterOutputPiece::Text(text) if text.is_empty()
        ));
    }
}
