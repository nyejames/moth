use super::{
    DiagnosticPayload, DiagnosticToken, InvalidFunctionSignatureReason,
    InvalidGenericParameterReason, InvalidTypeAnnotationReason, TokenDescriptorPayload,
    TypeAnnotationContext,
};
use crate::compiler_frontend::compiler_messages::render::{
    DiagnosticRenderContext, render_payload,
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

fn render_payload_message(payload: &DiagnosticPayload, string_table: &StringTable) -> String {
    render_payload(payload, DiagnosticRenderContext::new(string_table)).message
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
fn every_token_payload_render_survives_store_drop() {
    let (source_tokens, string_table) = projection_fixture();
    let (projected_payloads, projected_rendered) = {
        let mut projected_payloads = Vec::new();
        let mut projected_rendered = Vec::new();

        for index in 0..source_tokens.len() {
            let index = TokenIndex::try_from_index(index).expect("fixture index should fit");
            let token_ref = source_tokens
                .token(index)
                .expect("fixture token index should resolve");
            let projected = DiagnosticToken::from_token_ref(token_ref);
            let payloads = [
                DiagnosticPayload::ExpectedToken {
                    expected: DiagnosticToken::from_static_tag(TokenTag::ARROW),
                    found: Some(projected),
                },
                DiagnosticPayload::UnexpectedToken { found: projected },
                DiagnosticPayload::InvalidTypeAnnotation {
                    context: TypeAnnotationContext::DeclarationTarget,
                    reason: InvalidTypeAnnotationReason::InvalidTokenAfterName { token: projected },
                },
                DiagnosticPayload::InvalidTypeAnnotation {
                    context: TypeAnnotationContext::DeclarationTarget,
                    reason: InvalidTypeAnnotationReason::ExpectedTypeAnnotation {
                        found: projected,
                    },
                },
                DiagnosticPayload::InvalidGenericParameter {
                    reason: InvalidGenericParameterReason::InvalidToken { found: projected },
                },
                DiagnosticPayload::InvalidFunctionSignature {
                    reason: InvalidFunctionSignatureReason::MissingArrowOrColon {
                        found: projected,
                    },
                },
                DiagnosticPayload::InvalidFunctionSignature {
                    reason: InvalidFunctionSignatureReason::MissingCommaOrColon {
                        found: projected,
                    },
                },
            ];
            for payload in payloads {
                projected_rendered.push(render_payload_message(&payload, &string_table));
                projected_payloads.push(payload);
            }
        }

        (projected_payloads, projected_rendered)
    };
    drop(source_tokens);

    let after_drop = projected_payloads
        .iter()
        .map(|payload| render_payload_message(payload, &string_table))
        .collect::<Vec<_>>();
    assert_eq!(projected_rendered, after_drop);
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
            let projected_unexpected =
                super::CompilerDiagnostic::unexpected_token_from_ref(token_ref, None);
            let projected_expected = super::CompilerDiagnostic::expected_token_from_ref(
                TokenTag::ARROW,
                Some(token_ref),
                None,
            );

            projected_rendered.push(render_payload_message(
                &projected_unexpected.payload,
                &string_table,
            ));
            projected_rendered.push(render_payload_message(
                &projected_expected.payload,
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
        .map(|diagnostic| render_payload_message(&diagnostic.payload, &string_table))
        .collect::<Vec<_>>();
    assert_eq!(projected_rendered, after_drop);
}
#[test]
fn remapping_generic_parameter_projection_updates_symbol_text() {
    let mut local_table = StringTable::new();
    let name = local_table.intern("generic_name");
    let diagnostic = super::CompilerDiagnostic::invalid_generic_parameter(
        InvalidGenericParameterReason::InvalidToken {
            found: DiagnosticToken::from_string_tag(TokenTag::SYMBOL, name),
        },
        None,
    );
    let mut bag = super::DiagnosticBag::from_diagnostics(vec![diagnostic]);
    let mut merged_table = StringTable::new();
    let remap = merged_table.merge_from(&local_table);
    bag.remap_string_ids(&remap);

    let payload = &bag.diagnostics()[0].payload;
    assert_eq!(
        render_payload_message(payload, &merged_table),
        "Invalid generic parameter token name `generic_name`."
    );
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
