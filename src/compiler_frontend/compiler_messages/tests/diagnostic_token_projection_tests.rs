use super::{
    CompilerDiagnostic, DiagnosticBag, DiagnosticToken, InvalidFunctionSignatureReason,
    InvalidGenericParameterReason, TokenDescriptorPayload,
};
use crate::compiler_frontend::compiler_messages::diagnostics::{
    families::generics_traits_api, test_support::compact_bag_for_test,
};
use crate::compiler_frontend::compiler_messages::render::DiagnosticRenderContext;
use crate::compiler_frontend::compiler_messages::render::terminal::format_diagnostic_guidance;
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
    let path = path_syntax.push(PathId::ROOT, span);
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

fn render_diagnostic_message(
    diagnostic: &CompilerDiagnostic,
    string_table: &StringTable,
) -> String {
    format_diagnostic_guidance(diagnostic, DiagnosticRenderContext::new(string_table)).join("\n")
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
        .map(|view| {
            crate::compiler_frontend::compiler_messages::render::terse::format_terse_diagnostic_view(
                view,
                DiagnosticRenderContext::new(&string_table),
            )
        })
        .collect::<Vec<_>>();
    drop(source_tokens);
    let rendered_after_drop = compact
        .views()
        .map(|view| {
            crate::compiler_frontend::compiler_messages::render::terse::format_terse_diagnostic_view(
                view,
                DiagnosticRenderContext::new(&string_table),
            )
        })
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
    let subject =
        crate::compiler_frontend::compiler_messages::render::subject::DiagnosticSubject::new(view);
    let message = crate::compiler_frontend::compiler_messages::render::terse_diagnostic_message(
        &subject, context,
    );
    assert_eq!(
        message,
        "Invalid generic parameter token name `generic_name`."
    );
}

#[test]
fn projected_unexpected_and_expected_rendering_survives_store_drop() {
    let (source_tokens, string_table) = projection_fixture();
    let (projected_diagnostics, projected_rendered) = {
        let mut projected_diagnostics = Vec::with_capacity(source_tokens.len() * 2);
        let mut projected_rendered = Vec::with_capacity(source_tokens.len() * 2);

        for index in 0..source_tokens.len() {
            let index = TokenIndex::try_from_index(index).expect("fixture index should fit");
            let token_ref = source_tokens
                .token(index)
                .expect("fixture token index should resolve");
            let projected = DiagnosticToken::try_from_token_ref(token_ref)
                .expect("fixture token should project into a diagnostic token");
            let projected_unexpected =
                super::CompilerDiagnostic::unexpected_token_from_tag(projected, None);
            let projected_expected = super::CompilerDiagnostic::expected_token_from_ref(
                TokenTag::ARROW,
                Some(token_ref),
                None,
            );

            projected_rendered.push(render_diagnostic_message(
                &projected_unexpected,
                &string_table,
            ));
            projected_rendered.push(render_diagnostic_message(
                &projected_expected,
                &string_table,
            ));
            projected_diagnostics.push(projected_unexpected);
            projected_diagnostics.push(projected_expected);
        }

        (projected_diagnostics, projected_rendered)
    };

    drop(source_tokens);
    let after_drop = projected_diagnostics
        .iter()
        .map(|diagnostic| render_diagnostic_message(diagnostic, &string_table))
        .collect::<Vec<_>>();
    assert_eq!(projected_rendered, after_drop);
}

#[test]
fn projection_uses_expected_tag_and_payload_for_every_fixture_token() {
    let (source_tokens, _string_table) = projection_fixture();
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

    for (index, expected_tag) in expected_tags.into_iter().enumerate() {
        let index = TokenIndex::try_from_index(index).expect("fixture index should fit");
        let token_ref = source_tokens
            .token(index)
            .expect("fixture token index should resolve");
        assert_eq!(
            DiagnosticToken::from_token_ref(token_ref).tag(),
            expected_tag,
            "canonical projection must preserve the schema tag",
        );
    }
}
