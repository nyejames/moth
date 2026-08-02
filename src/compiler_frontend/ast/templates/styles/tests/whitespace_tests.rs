use super::super::whitespace::{
    TemplateBodyRunPosition, TemplateWhitespacePassProfile, apply_whitespace_passes_to_input,
};
use crate::compiler_frontend::ast::templates::formatter_contract::{
    FormatterInput, FormatterInputPiece, FormatterOutputPiece, FormatterTextPiece,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;

fn apply_default_template_body_whitespace(input: &str) -> String {
    let mut string_table = StringTable::new();
    let input_id = string_table.intern(input);
    let output = apply_whitespace_passes_to_input(
        FormatterInput {
            pieces: vec![FormatterInputPiece::Text(FormatterTextPiece {
                text: input_id,
                location: SourceLocation::default(),
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
