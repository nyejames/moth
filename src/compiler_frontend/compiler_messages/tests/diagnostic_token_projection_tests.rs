use super::{
    DiagnosticPayload, DiagnosticToken, InvalidFunctionSignatureReason,
    InvalidGenericParameterReason, InvalidTypeAnnotationReason, TokenDescriptorPayload, TokenTag,
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
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, Token, TokenIndex, TokenKind};

fn projection_fixture() -> (FileTokens, StringTable) {
    let source = SourceId::COMPILATION_ROOT;
    let mut string_table = StringTable::new();
    let name = string_table.intern("value");
    let style = string_table.intern("md");
    let string = string_table.intern("hello");
    let raw_string = string_table.intern("raw");
    let integer = NumericLiteralToken::test_new("42", &mut string_table);
    let decimal = NumericLiteralToken::test_new("3.5", &mut string_table);
    let exponent = NumericLiteralToken::test_new("1e6", &mut string_table);

    let mut path_syntax = PathSyntaxTable::with_source(source);
    let path = path_syntax.push(PathId::ROOT, LocalSpan::source_start());
    let kinds = [
        TokenKind::Eof,
        TokenKind::Path(path),
        TokenKind::Symbol(name),
        TokenKind::StyleDirective(style),
        TokenKind::StringSliceLiteral(string),
        TokenKind::NumericLiteral(integer),
        TokenKind::NumericLiteral(decimal),
        TokenKind::NumericLiteral(exponent),
        TokenKind::CharLiteral('λ'),
        TokenKind::RawStringLiteral(raw_string),
        TokenKind::BoolLiteral(true),
        TokenKind::BoolLiteral(false),
    ];
    let tokens = kinds
        .into_iter()
        .map(|kind| Token::new(kind, LocalSpan::source_start()))
        .collect();

    let mut file_tokens =
        FileTokens::new_with_identity(PathId::ROOT, source, None, tokens, path_syntax);
    file_tokens.freeze_path_syntax_for_test();
    (file_tokens, string_table)
}

fn render_payload_message(payload: &DiagnosticPayload, string_table: &StringTable) -> String {
    render_payload(payload, DiagnosticRenderContext::new(string_table)).message
}

fn token_payload_pairs(
    legacy: DiagnosticToken,
    projected: DiagnosticToken,
) -> [(DiagnosticPayload, DiagnosticPayload); 7] {
    [
        (
            DiagnosticPayload::ExpectedToken {
                expected: DiagnosticToken::from_static_tag(TokenTag::ARROW),
                found: Some(legacy),
            },
            DiagnosticPayload::ExpectedToken {
                expected: DiagnosticToken::from_static_tag(TokenTag::ARROW),
                found: Some(projected),
            },
        ),
        (
            DiagnosticPayload::UnexpectedToken { found: legacy },
            DiagnosticPayload::UnexpectedToken { found: projected },
        ),
        (
            DiagnosticPayload::InvalidTypeAnnotation {
                context: TypeAnnotationContext::DeclarationTarget,
                reason: InvalidTypeAnnotationReason::InvalidTokenAfterName { token: legacy },
            },
            DiagnosticPayload::InvalidTypeAnnotation {
                context: TypeAnnotationContext::DeclarationTarget,
                reason: InvalidTypeAnnotationReason::InvalidTokenAfterName { token: projected },
            },
        ),
        (
            DiagnosticPayload::InvalidTypeAnnotation {
                context: TypeAnnotationContext::DeclarationTarget,
                reason: InvalidTypeAnnotationReason::ExpectedTypeAnnotation { found: legacy },
            },
            DiagnosticPayload::InvalidTypeAnnotation {
                context: TypeAnnotationContext::DeclarationTarget,
                reason: InvalidTypeAnnotationReason::ExpectedTypeAnnotation { found: projected },
            },
        ),
        (
            DiagnosticPayload::InvalidGenericParameter {
                reason: InvalidGenericParameterReason::InvalidToken { found: legacy },
            },
            DiagnosticPayload::InvalidGenericParameter {
                reason: InvalidGenericParameterReason::InvalidToken { found: projected },
            },
        ),
        (
            DiagnosticPayload::InvalidFunctionSignature {
                reason: InvalidFunctionSignatureReason::MissingArrowOrColon { found: legacy },
            },
            DiagnosticPayload::InvalidFunctionSignature {
                reason: InvalidFunctionSignatureReason::MissingArrowOrColon { found: projected },
            },
        ),
        (
            DiagnosticPayload::InvalidFunctionSignature {
                reason: InvalidFunctionSignatureReason::MissingCommaOrColon { found: legacy },
            },
            DiagnosticPayload::InvalidFunctionSignature {
                reason: InvalidFunctionSignatureReason::MissingCommaOrColon { found: projected },
            },
        ),
    ]
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
fn every_token_payload_render_matches_legacy_and_survives_store_drop() {
    let (file_tokens, string_table) = projection_fixture();
    let (legacy_rendered, projected_payloads, projected_rendered) = {
        let source_tokens = file_tokens
            .source_tokens()
            .expect("fixture should retain its canonical source token store");
        let mut legacy_rendered = Vec::new();
        let mut projected_payloads = Vec::new();
        let mut projected_rendered = Vec::new();

        for (index, legacy_token) in file_tokens.tokens.iter().enumerate() {
            let index = TokenIndex::try_from_index(index).expect("fixture index should fit");
            let token_ref = source_tokens
                .token(index)
                .expect("fixture token index should resolve");
            let legacy = DiagnosticToken::from(&legacy_token.kind);
            let projected = DiagnosticToken::from_token_ref(token_ref);

            for (legacy_payload, projected_payload) in token_payload_pairs(legacy, projected) {
                legacy_rendered.push(render_payload_message(&legacy_payload, &string_table));
                projected_rendered.push(render_payload_message(&projected_payload, &string_table));
                projected_payloads.push(projected_payload);
            }
        }

        (legacy_rendered, projected_payloads, projected_rendered)
    };

    assert_eq!(legacy_rendered, projected_rendered);
    drop(file_tokens);

    let after_drop = projected_payloads
        .iter()
        .map(|payload| render_payload_message(payload, &string_table))
        .collect::<Vec<_>>();
    assert_eq!(projected_rendered, after_drop);
}

#[test]
fn projected_unexpected_and_expected_rendering_matches_legacy_before_and_after_drop() {
    let (file_tokens, string_table) = projection_fixture();
    let (legacy_rendered, projected_diagnostics, projected_rendered) = {
        let source_tokens = file_tokens
            .source_tokens()
            .expect("fixture should retain its canonical source token store");
        let mut legacy_rendered = Vec::with_capacity(file_tokens.tokens.len() * 2);
        let mut projected_diagnostics = Vec::with_capacity(file_tokens.tokens.len() * 2);
        let mut projected_rendered = Vec::with_capacity(file_tokens.tokens.len() * 2);

        for (index, legacy_token) in file_tokens.tokens.iter().enumerate() {
            let index = TokenIndex::try_from_index(index).expect("fixture index should fit");
            let token_ref = source_tokens
                .token(index)
                .expect("fixture token index should resolve");
            let legacy = DiagnosticToken::from(&legacy_token.kind);
            let legacy_unexpected =
                super::CompilerDiagnostic::unexpected_token_from_tag(legacy, None);
            let legacy_expected = super::CompilerDiagnostic::expected_token_from_tags(
                TokenTag::ARROW,
                Some(legacy),
                None,
            );
            let projected_unexpected =
                super::CompilerDiagnostic::unexpected_token_from_ref(token_ref, None);
            let projected_expected = super::CompilerDiagnostic::expected_token_from_ref(
                TokenTag::ARROW,
                Some(token_ref),
                None,
            );

            legacy_rendered.push(render_payload_message(
                &legacy_unexpected.payload,
                &string_table,
            ));
            legacy_rendered.push(render_payload_message(
                &legacy_expected.payload,
                &string_table,
            ));
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

        (legacy_rendered, projected_diagnostics, projected_rendered)
    };

    assert_eq!(legacy_rendered, projected_rendered);
    drop(file_tokens);
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
            found: DiagnosticToken::from(TokenKind::Symbol(name)),
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
fn projection_uses_the_shared_tag_for_every_dynamic_descriptor() {
    let (file_tokens, _string_table) = projection_fixture();
    let source_tokens = file_tokens
        .source_tokens()
        .expect("fixture should retain its canonical source token store");

    for (index, legacy_token) in file_tokens.tokens.iter().enumerate() {
        let index = TokenIndex::try_from_index(index).expect("fixture index should fit");
        let token_ref = source_tokens
            .token(index)
            .expect("fixture token index should resolve");
        assert_eq!(
            DiagnosticToken::from(&legacy_token.kind).tag(),
            DiagnosticToken::from_token_ref(token_ref).tag(),
            "legacy and canonical projections must share the TokenTag descriptor"
        );
    }
}
