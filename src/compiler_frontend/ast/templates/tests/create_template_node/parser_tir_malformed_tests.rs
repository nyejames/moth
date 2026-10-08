//! Focused malformed-template tests for parser-to-TIR surfaces.
//!
//! WHAT: adds diagnostic parity coverage around body text, nested child
//! templates, slot/insert helpers, and suppressed child-template brackets
//! now that the parser emits TIR nodes directly.
//!
//! WHY: body and head parsing record TIR nodes incrementally. If an
//! error is raised after some nodes have been recorded, the diagnostic reason
//! and source span must remain stable. These tests pin the
//! expected malformed-surface behavior without relying on internal TIR IDs.

use super::*;
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticPayload, DiagnosticToken, InvalidTemplateStructureReason,
};
use crate::compiler_frontend::source::SourceId;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::TokenTag;
use std::sync::Arc;

fn parse_template_diagnostic(source: &str) -> CompilerDiagnostic {
    let mut string_table = StringTable::new();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut path_fork = PathInternerFork::empty();
    let file_tokens =
        template_tokens_from_source(source, &mut string_table, &mut span_builder, &mut path_fork);
    let source_path = file_tokens.source_path;
    let canonical_owner = file_tokens
        .canonical_owner()
        .expect("test token stream must expose canonical source tokens");
    let canonical_range = canonical_owner
        .full_range()
        .expect("test token stream must expose canonical source range");
    let mut token_stream = AstCursor::from_source_tokens(&canonical_owner, canonical_range)
        .expect("test token stream must expose an AST cursor");
    token_stream
        .set_position(file_tokens.opener_index)
        .expect("template opener position must remain in the canonical range");
    let context = new_constant_context(source_path, &path_fork);

    expect_template_diagnostic(
        Template::new(
            &mut token_stream,
            source_path,
            &context,
            vec![],
            &mut string_table,
            &mut path_fork,
        )
        .expect_err("template source should fail to parse"),
    )
}

fn parse_template_diagnostic_with_replaced_body_token(
    source: &str,
) -> (CompilerDiagnostic, ExtendedSpanBuilder) {
    let mut string_table = StringTable::new();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut path_fork = PathInternerFork::empty();
    let file_tokens =
        template_tokens_from_source(source, &mut string_table, &mut span_builder, &mut path_fork);
    // Normal template-body lexing emits text, newline, nested-template, and close tokens only.
    // Corrupt the canonical payload in place so the parser exercises its defensive
    // unexpected-token lane without falling back to a compatibility token vector.
    let opener_index = file_tokens.opener_index;
    let source_path = file_tokens.source_path;
    let mut canonical_owner = file_tokens
        .canonical_owner()
        .expect("test token stream must expose canonical source tokens");
    drop(file_tokens);
    let body_index = canonical_owner
        .shapes()
        .iter()
        .position(|shape| shape.tag() == TokenTag::STRING_SLICE_LITERAL)
        .expect("template source should contain a body token");
    Arc::get_mut(&mut canonical_owner)
        .expect("canonical owner should be uniquely mutable before cursor construction")
        .corrupt_payload_for_test(body_index, TokenTag::COMMA);
    let canonical_range = canonical_owner
        .full_range()
        .expect("test token stream must expose canonical source range");
    let mut token_stream = AstCursor::from_source_tokens(&canonical_owner, canonical_range)
        .expect("test token stream must expose an AST cursor");
    token_stream
        .set_position(opener_index)
        .expect("template opener position must remain in the canonical range");
    let context = new_constant_context(source_path, &path_fork);

    let diagnostic = expect_template_diagnostic(
        Template::new(
            &mut token_stream,
            source_path,
            &context,
            vec![],
            &mut string_table,
            &mut path_fork,
        )
        .expect_err("template source should fail to parse"),
    );
    (diagnostic, span_builder)
}

/// Asserts that a diagnostic is an `InvalidTemplateStructure` with the given reason.
fn assert_invalid_template_structure(
    diagnostic: &CompilerDiagnostic,
    expected_reason: InvalidTemplateStructureReason,
) {
    match &diagnostic.payload {
        DiagnosticPayload::InvalidTemplateStructure { reason } => {
            assert_eq!(*reason, expected_reason);
        }
        payload => panic!("expected invalid template structure payload, found {payload:?}"),
    }
}

/// Asserts that the diagnostic carries a meaningful source span rather
/// than the absent span used for synthetic errors.
fn assert_span_is_meaningful(diagnostic: &CompilerDiagnostic) {
    assert!(
        !is_default_error_span(diagnostic.primary_span),
        "diagnostic should carry a meaningful source span, got {:?}",
        diagnostic.primary_span
    );
}

#[test]
fn unexpected_template_body_token_retains_exact_primary_span() {
    let source = "[:π]";
    let (diagnostic, span_builder) = parse_template_diagnostic_with_replaced_body_token(source);

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::UnexpectedToken { .. }
    ));
    let primary_span = diagnostic
        .primary_span
        .expect("unexpected body token should retain its exact span");
    assert_eq!(primary_span.source(), SourceId::COMPILATION_ROOT);
    let range = primary_span.resolve_with(span_builder.resolver_for(SourceId::COMPILATION_ROOT));
    assert_eq!(range.start(), 2);
    assert_eq!(range.end(), 4);
    assert_eq!(&source[range.start() as usize..range.end() as usize], "π");
    assert!(diagnostic.labels.is_empty());
}

#[test]
fn truncated_nested_template_body_reports_eof_with_meaningful_span() {
    let diagnostic = parse_template_diagnostic("[: outer [: inner]");

    assert!(
        matches!(
            diagnostic.payload,
            DiagnosticPayload::UnexpectedEndOfFile { .. }
        ),
        "expected unexpected-end-of-file for truncated nested body, got {:?}",
        diagnostic.payload
    );
    assert_span_is_meaningful(&diagnostic);
}

#[test]
fn children_directive_truncated_argument_template_reports_eof_with_meaningful_span() {
    let diagnostic = parse_template_diagnostic("[$children([: unclosed): body]");

    let has_expected_payload = match &diagnostic.payload {
        DiagnosticPayload::UnexpectedEndOfFile { .. } => true,
        DiagnosticPayload::ExpectedToken { expected, .. } => {
            *expected == DiagnosticToken::from_static_tag(TokenTag::CLOSE_PARENTHESIS)
        }
        _ => false,
    };
    assert!(
        has_expected_payload,
        "expected unexpected-end-of-file or missing-close-paren for truncated $children argument template, got {:?}",
        diagnostic.payload
    );
    assert_span_is_meaningful(&diagnostic);
}

#[test]
fn doc_suppressed_child_template_unclosed_bracket_reports_eof() {
    let diagnostic = parse_template_diagnostic("[$doc: [: unclosed");

    assert!(
        matches!(
            diagnostic.payload,
            DiagnosticPayload::UnexpectedEndOfFile { .. }
        ),
        "expected unexpected-end-of-file for unclosed bracket in $doc body, got {:?}",
        diagnostic.payload
    );
    assert_span_is_meaningful(&diagnostic);
}

#[test]
fn insert_with_truncated_body_reports_eof_with_meaningful_span() {
    let diagnostic = parse_template_diagnostic("[$insert(\"name\"): body");

    assert!(
        matches!(
            diagnostic.payload,
            DiagnosticPayload::UnexpectedEndOfFile { .. }
        ),
        "expected unexpected-end-of-file for truncated $insert body, got {:?}",
        diagnostic.payload
    );
    assert_span_is_meaningful(&diagnostic);
}

#[test]
fn insert_with_truncated_nested_body_in_helper_reports_eof() {
    let diagnostic = parse_template_diagnostic("[$insert(\"name\"): [: inner]");

    assert!(
        matches!(
            diagnostic.payload,
            DiagnosticPayload::UnexpectedEndOfFile { .. }
        ),
        "expected unexpected-end-of-file for $insert helper with truncated nested body, got {:?}",
        diagnostic.payload
    );
    assert_span_is_meaningful(&diagnostic);
}

#[test]
fn slot_definition_with_body_is_rejected() {
    let diagnostic = parse_template_diagnostic("[$slot: body]");

    assert_invalid_template_structure(&diagnostic, InvalidTemplateStructureReason::SlotInHead);
    assert_span_is_meaningful(&diagnostic);
}
