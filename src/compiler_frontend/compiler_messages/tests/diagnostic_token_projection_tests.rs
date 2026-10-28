use super::{
    DiagnosticBag, DiagnosticToken, InvalidFunctionSignatureReason, InvalidGenericParameterReason,
    TokenDescriptorPayload,
};
use crate::compiler_frontend::compiler_messages::diagnostics::{
    families::{
        generics_traits_api,
        token_lexical::{self, ExpectedTokenView, UnexpectedTokenShape, UnexpectedTokenView},
    },
    test_support::compact_bag_for_test,
    view::DiagnosticView,
};
use crate::compiler_frontend::compiler_messages::render::terse::format_terse_diagnostic_view;
use crate::compiler_frontend::compiler_messages::render::{
    DiagnosticRenderContext, subject::DiagnosticSubject, terminal, terse, terse_diagnostic_message,
};
use crate::compiler_frontend::numeric_text::token::NumericLiteralToken;
use crate::compiler_frontend::paths::path_syntax::PathSyntaxTable;
use crate::compiler_frontend::source::{LocalSpan, SourceId};
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{
    SourceTokens, TestSourceTokensBuilder, TokenIndex, TokenTag,
};
use std::sync::Arc;

fn projection_fixture() -> (Arc<SourceTokens>, StringTable) {
    let source = SourceId::COMPILATION_ROOT;
    let mut string_table = StringTable::new();
    let name = string_table.intern("value");
    let style = string_table.intern("md");
    let string = string_table.intern("hello");
    let raw_string = string_table.intern("raw");
    let integer = NumericLiteralToken::test_new("42", &mut string_table);
    let decimal = NumericLiteralToken::test_new("3.5", &mut string_table);
    let exponent = NumericLiteralToken::test_new("1e6", &mut string_table);
    let span = LocalSpan::source_start();

    let mut path_syntax = PathSyntaxTable::with_source(source);
    let path = path_syntax
        .try_push_local(PathId::ROOT, span)
        .expect("test path syntax row should fit");
    let mut builder = TestSourceTokensBuilder::with_path_syntax(source, path_syntax);
    builder.push_static(TokenTag::EOF, span).unwrap();
    builder.push_path(TokenTag::PATH, path, span).unwrap();
    builder.push_symbol(TokenTag::SYMBOL, name, span).unwrap();
    builder
        .push_symbol(TokenTag::STYLE_DIRECTIVE, style, span)
        .unwrap();
    builder
        .push_symbol(TokenTag::STRING_SLICE_LITERAL, string, span)
        .unwrap();
    builder.push_numeric(integer, span).unwrap();
    builder.push_numeric(decimal, span).unwrap();
    builder.push_numeric(exponent, span).unwrap();
    builder
        .push_char(TokenTag::CHAR_LITERAL, 'λ', span)
        .unwrap();
    builder
        .push_symbol(TokenTag::RAW_STRING_LITERAL, raw_string, span)
        .unwrap();
    builder
        .push_bool(TokenTag::BOOL_LITERAL, true, span)
        .unwrap();
    builder
        .push_bool(TokenTag::BOOL_LITERAL, false, span)
        .unwrap();
    (
        builder.finish().expect("projection fixture should build"),
        string_table,
    )
}

fn render_diagnostic_message(view: DiagnosticView<'_>, string_table: &StringTable) -> String {
    format_terse_diagnostic_view(view, DiagnosticRenderContext::new(string_table))
}

#[test]
fn diagnostic_token_has_the_locked_eight_byte_layout() {
    assert_eq!(std::mem::size_of::<DiagnosticToken>(), 8);
    assert_eq!(std::mem::align_of::<DiagnosticToken>(), 4);
}

#[test]
fn static_and_path_descriptors_project_without_payload_data() {
    for tag in TokenTag::all() {
        if matches!(
            tag.descriptor().payload(),
            TokenDescriptorPayload::Static | TokenDescriptorPayload::Path
        ) {
            let token = DiagnosticToken::from_static_tag(*tag);
            assert_eq!(token.tag(), *tag);
            assert_eq!(token.flags(), 0);
            assert_eq!(token.data(), 0);
        }
    }
}

#[test]
fn every_token_projection_render_survives_source_store_drop() {
    let (source_tokens, string_table) = projection_fixture();
    let mut diagnostics = Vec::with_capacity(source_tokens.len() * 3);
    for index in 0..source_tokens.len() {
        let index = TokenIndex::try_from_index(index).expect("fixture index should fit");
        let token_ref = source_tokens
            .token(index)
            .expect("fixture token index should resolve");
        let found = DiagnosticToken::from_token_ref(token_ref);
        diagnostics.push(generics_traits_api::invalid_generic_parameter(
            InvalidGenericParameterReason::InvalidToken { found },
            None,
        ));
        diagnostics.push(generics_traits_api::invalid_function_signature(
            InvalidFunctionSignatureReason::MissingArrowOrColon { found },
            None,
        ));
        diagnostics.push(generics_traits_api::invalid_function_signature(
            InvalidFunctionSignatureReason::MissingCommaOrColon { found },
            None,
        ));
    }

    let compact = compact_bag_for_test(DiagnosticBag::from_diagnostics(diagnostics), None);
    let rendered_before_drop = compact
        .views()
        .map(|view| render_diagnostic_message(view, &string_table))
        .collect::<Vec<_>>();
    drop(source_tokens);
    let rendered_after_drop = compact
        .views()
        .map(|view| render_diagnostic_message(view, &string_table))
        .collect::<Vec<_>>();
    assert_eq!(rendered_before_drop, rendered_after_drop);
}

#[test]
fn remapping_generic_parameter_projection_updates_symbol_text() {
    let mut local_table = StringTable::new();
    let name = local_table.intern("generic_name");
    let diagnostic = generics_traits_api::invalid_generic_parameter(
        InvalidGenericParameterReason::InvalidToken {
            found: DiagnosticToken::from_string_tag(TokenTag::SYMBOL, name),
        },
        None,
    );
    let mut bag = DiagnosticBag::from_diagnostics(vec![diagnostic]);
    let mut merged_table = StringTable::new();
    let remap = merged_table.merge_from(&local_table);
    bag.remap_string_ids(&remap);

    let compact = compact_bag_for_test(bag, None);
    let view = compact
        .first()
        .expect("generic parameter diagnostic should compact");
    assert!(
        view.family_view::<generics_traits_api::InvalidGenericParameterView>()
            .is_some(),
        "generic parameter projection should remain in its diagnostic family"
    );
    let context = DiagnosticRenderContext::new(&merged_table);
    let subject = DiagnosticSubject::new(view);
    let message = terse_diagnostic_message(&subject, context);
    assert_eq!(
        message,
        "Invalid generic parameter token name `generic_name`."
    );
}

#[test]
fn projected_unexpected_and_expected_rendering_survives_store_drop() {
    let (source_tokens, string_table) = projection_fixture();
    let mut diagnostics = Vec::with_capacity(source_tokens.len() * 2);
    let mut projected_tokens = Vec::with_capacity(source_tokens.len());
    let expected_token = DiagnosticToken::from_static_tag(TokenTag::ARROW);

    for index in 0..source_tokens.len() {
        let index = TokenIndex::try_from_index(index).expect("fixture index should fit");
        let token_ref = source_tokens
            .token(index)
            .expect("fixture token index should resolve");
        let projected = DiagnosticToken::try_from_token_ref(token_ref)
            .expect("fixture token should project into a diagnostic token");
        diagnostics.push(token_lexical::unexpected_token_from_tag(projected, None));
        diagnostics.push(token_lexical::expected_token_from_projections(
            expected_token,
            Some(projected),
            None,
        ));
        projected_tokens.push(projected);
    }

    let compact = compact_bag_for_test(DiagnosticBag::from_diagnostics(diagnostics), None);
    let views = compact.views().collect::<Vec<_>>();
    assert_eq!(views.len(), projected_tokens.len() * 2);
    for (pair, projected) in views.as_chunks::<2>().0.iter().zip(&projected_tokens) {
        let unexpected = pair[0];
        let unexpected_facts = unexpected
            .family_view::<UnexpectedTokenView>()
            .expect("unexpected-token drafts keep their direct family facts");
        assert!(matches!(
            &unexpected_facts.shape,
            UnexpectedTokenShape::Token(found) if found == projected
        ));
        let unexpected_message = render_diagnostic_message(unexpected, &string_table);
        assert!(unexpected_message.contains("Unexpected token "));
        let expected_fragment = match projected.tag().descriptor().payload() {
            TokenDescriptorPayload::NumericLiteral if projected.is_whole_number() => {
                "integer literal"
            }
            TokenDescriptorPayload::NumericLiteral => "float literal",
            _ => projected.tag().descriptor().text(),
        };
        assert!(
            unexpected_message.contains(expected_fragment),
            "expected token description {expected_fragment:?} in: {unexpected_message}"
        );

        let expected = pair[1];
        let expected_facts = expected
            .family_view::<ExpectedTokenView>()
            .expect("expected-token drafts keep their direct family facts");
        assert_eq!(expected_facts.expected, expected_token);
        assert_eq!(expected_facts.found, Some(*projected));
        let expected_message = render_diagnostic_message(expected, &string_table);
        assert!(expected_message.contains("Expected "));
        assert!(expected_message.contains(", but found "));
    }

    let rendered_before_drop = views
        .iter()
        .copied()
        .map(|view| render_diagnostic_message(view, &string_table))
        .collect::<Vec<_>>();
    drop(source_tokens);
    let rendered_after_drop = compact
        .views()
        .map(|view| render_diagnostic_message(view, &string_table))
        .collect::<Vec<_>>();
    assert_eq!(rendered_before_drop, rendered_after_drop);
}

#[test]
fn projection_uses_expected_tag_and_payload_for_every_fixture_token() {
    let (source_tokens, string_table) = projection_fixture();
    let expected_tags = [
        TokenTag::EOF,
        TokenTag::PATH,
        TokenTag::SYMBOL,
        TokenTag::STYLE_DIRECTIVE,
        TokenTag::STRING_SLICE_LITERAL,
        TokenTag::NUMERIC_LITERAL,
        TokenTag::NUMERIC_LITERAL,
        TokenTag::NUMERIC_LITERAL,
        TokenTag::CHAR_LITERAL,
        TokenTag::RAW_STRING_LITERAL,
        TokenTag::BOOL_LITERAL,
        TokenTag::BOOL_LITERAL,
    ];
    let expected_text = [
        None,
        None,
        Some("value"),
        Some("md"),
        Some("hello"),
        Some("42"),
        Some("3.5"),
        Some("1e6"),
        None,
        Some("raw"),
        None,
        None,
    ];

    for (position, (expected_tag, expected_text)) in
        expected_tags.into_iter().zip(expected_text).enumerate()
    {
        let token_index = TokenIndex::try_from_index(position).expect("fixture index should fit");
        let token_ref = source_tokens
            .token(token_index)
            .expect("fixture token index should resolve");
        let projected = DiagnosticToken::from_token_ref(token_ref);
        assert_eq!(
            projected.tag(),
            expected_tag,
            "canonical projection must preserve the schema tag",
        );
        match expected_tag.descriptor().payload() {
            TokenDescriptorPayload::Static | TokenDescriptorPayload::Path => {
                assert_eq!(projected.flags(), 0);
                assert_eq!(projected.data(), 0);
            }
            TokenDescriptorPayload::Symbol
            | TokenDescriptorPayload::StyleDirective
            | TokenDescriptorPayload::StringLiteral
            | TokenDescriptorPayload::NumericLiteral
            | TokenDescriptorPayload::RawStringLiteral => {
                assert_eq!(
                    string_table.resolve(projected.string_id()),
                    expected_text.expect("text-payload token has an expected spelling"),
                );
                if expected_tag == TokenTag::NUMERIC_LITERAL {
                    assert_eq!(projected.is_whole_number(), position == 5);
                } else {
                    assert_eq!(projected.flags(), 0);
                }
            }
            TokenDescriptorPayload::CharLiteral => {
                assert_eq!(projected.char_value(), 'λ');
            }
            TokenDescriptorPayload::BoolLiteral => {
                assert_eq!(projected.bool_value(), position == 10);
            }
        }
    }
}

#[test]
fn token_diagnostics_render_source_spelling_not_token_debug_names() {
    let mut string_table = StringTable::new();
    let name = string_table.intern("item");
    let expected = token_lexical::expected_token_from_projections(
        DiagnosticToken::from_static_tag(TokenTag::OPEN_PARENTHESIS),
        Some(DiagnosticToken::from_string_tag(TokenTag::SYMBOL, name)),
        None,
    );
    let unexpected = token_lexical::unexpected_token_from_tag(
        DiagnosticToken::from_static_tag(TokenTag::OPEN_CURLY),
        None,
    );
    let compact = compact_bag_for_test(
        DiagnosticBag::from_diagnostics(vec![expected, unexpected]),
        None,
    );
    let mut views = compact.views();
    let expected_view = views.next().expect("expected token");
    assert!(expected_view.family_view::<ExpectedTokenView>().is_some());
    let expected_terse = terse::format_terse_diagnostic_view(
        expected_view,
        DiagnosticRenderContext::new(&string_table),
    );
    let unexpected_view = views.next().expect("unexpected token");
    let unexpected_terminal = terminal::format_guidance_view(
        unexpected_view,
        DiagnosticRenderContext::new(&string_table),
    )
    .join("\n");
    assert!(expected_terse.contains("Expected `(`"), "{expected_terse}");
    assert!(expected_terse.contains("name `item`"), "{expected_terse}");
    assert!(
        unexpected_terminal.contains("Unexpected token `{`"),
        "{unexpected_terminal}"
    );
    assert!(
        !expected_terse.contains("OpenParenthesis") && !expected_terse.contains("Symbol"),
        "{expected_terse}"
    );
}
