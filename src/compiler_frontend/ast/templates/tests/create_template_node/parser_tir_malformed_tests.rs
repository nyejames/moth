//! Focused malformed-template tests for parser-to-TIR surfaces.
//!
//! WHAT: adds diagnostic parity coverage around body text, nested child
//! templates, slot/insert helpers, and suppressed child-template brackets
//! since the parser emits TIR nodes directly.
//!
//! WHY: body and head parsing record TIR nodes incrementally. If an
//! error is raised after some nodes have been recorded, the diagnostic reason
//! and source span must remain stable. These tests pin the
//! expected malformed-surface behavior without relying on internal TIR IDs.

use super::*;
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::expression_store::AstExpressionBuilder;
use crate::compiler_frontend::compiler_messages::DiagnosticToken;
use crate::compiler_frontend::compiler_messages::InvalidTemplateStructureReason;
use crate::compiler_frontend::compiler_messages::diagnostics::families::token_lexical::{
    ExpectedTokenView, UnexpectedEndOfFileView, UnexpectedTokenShape, UnexpectedTokenView,
};
use crate::compiler_frontend::compiler_messages::diagnostics::foreign::SiteRef;
use crate::compiler_frontend::compiler_messages::diagnostics::test_support::CompactDiagnosticBagForTest;
use crate::compiler_frontend::source::SourceId;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::TokenTag;
use std::sync::Arc;

fn parse_template_diagnostic(source: &str) -> CompactDiagnosticBagForTest {
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let mut string_table = StringTable::new();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut path_fork = PathInternerFork::empty();
    let file_tokens =
        template_tokens_from_source(source, &mut string_table, &mut span_builder, &mut path_fork);
    let source_path = file_tokens.source_path;
    let canonical_owner = file_tokens.canonical_owner();
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
            &mut expression_builder,
            &mut const_values,
            vec![],
            &mut string_table,
            &mut path_fork,
        )
        .expect_err("template source should fail to parse"),
    )
}

fn parse_template_diagnostic_with_replaced_body_token(
    source: &str,
) -> (CompactDiagnosticBagForTest, ExtendedSpanBuilder) {
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let mut string_table = StringTable::new();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut path_fork = PathInternerFork::empty();
    let file_tokens =
        template_tokens_from_source(source, &mut string_table, &mut span_builder, &mut path_fork);
    // Normal template-body lexing emits text, newline, nested-template, and close tokens only.
    // Corrupt the canonical payload in place so the parser exercises its defensive
    // unexpected-token lane without materialising a separate token vector.
    let opener_index = file_tokens.opener_index;
    let source_path = file_tokens.source_path;
    let mut canonical_owner = file_tokens.canonical_owner();
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
            &mut expression_builder,
            &mut const_values,
            vec![],
            &mut string_table,
            &mut path_fork,
        )
        .expect_err("template source should fail to parse"),
    );
    (diagnostic, span_builder)
}

/// Asserts that the diagnostic carries a meaningful source span rather
/// than the absent span used for synthetic errors.
fn assert_span_is_meaningful(diagnostic: &CompactDiagnosticBagForTest) {
    let site = diagnostic
        .first()
        .expect("template diagnostic should compact to one record")
        .primary_site();
    let SiteRef::Local(span) = site else {
        panic!("template diagnostic should retain a local source span, got {site:?}");
    };
    assert!(
        !is_default_error_span(Some(span)),
        "diagnostic should carry a meaningful source span, got {span:?}"
    );
}

#[test]
fn unexpected_template_body_token_retains_exact_primary_span() {
    let source = "[:π]";
    let (diagnostic, span_builder) = parse_template_diagnostic_with_replaced_body_token(source);

    let view = diagnostic
        .first()
        .expect("unexpected body token should compact to one record");
    let facts = view
        .family_view::<UnexpectedTokenView>()
        .expect("unexpected body token should retain typed token facts");
    assert!(matches!(facts.shape, UnexpectedTokenShape::Token(_)));
    let SiteRef::Local(primary_span) = view.primary_site() else {
        panic!("unexpected body token should retain a local source span");
    };
    assert_eq!(primary_span.source(), SourceId::COMPILATION_ROOT);
    let range = primary_span.resolve_with(span_builder.resolver_for(SourceId::COMPILATION_ROOT));
    assert_eq!(range.start(), 2);
    assert_eq!(range.end(), 4);
    assert_eq!(&source[range.start() as usize..range.end() as usize], "π");
    assert_eq!(view.durable_labels().count(), 0);
}

#[test]
fn truncated_nested_template_body_reports_eof_with_meaningful_span() {
    let diagnostic = parse_template_diagnostic("[: outer [: inner]");

    let unexpected_eof = diagnostic
        .first()
        .expect("truncated nested body should compact to one record")
        .family_view::<UnexpectedEndOfFileView>()
        .is_some();
    assert!(
        unexpected_eof,
        "expected unexpected-end-of-file for truncated nested body"
    );
    assert_span_is_meaningful(&diagnostic);
}

#[test]
fn children_directive_truncated_argument_template_reports_eof_with_meaningful_span() {
    let diagnostic = parse_template_diagnostic("[$children([: unclosed): body]");

    let view = diagnostic
        .first()
        .expect("truncated children argument should compact to one record");
    let has_expected_payload = view.family_view::<UnexpectedEndOfFileView>().is_some()
        || view
            .family_view::<ExpectedTokenView>()
            .is_some_and(|facts| {
                facts.expected == DiagnosticToken::from_static_tag(TokenTag::CLOSE_PARENTHESIS)
            });
    assert!(
        has_expected_payload,
        "expected unexpected-end-of-file or missing-close-paren for truncated $children argument"
    );
    assert_span_is_meaningful(&diagnostic);
}

#[test]
fn doc_suppressed_child_template_unclosed_bracket_reports_eof() {
    let diagnostic = parse_template_diagnostic("[$doc: [: unclosed");

    let unexpected_eof = diagnostic
        .first()
        .expect("unclosed doc body should compact to one record")
        .family_view::<UnexpectedEndOfFileView>()
        .is_some();
    assert!(
        unexpected_eof,
        "expected unexpected-end-of-file for unclosed bracket in $doc body"
    );
    assert_span_is_meaningful(&diagnostic);
}

#[test]
fn insert_with_truncated_body_reports_eof_with_meaningful_span() {
    let diagnostic = parse_template_diagnostic("[$insert(\"name\"): body");

    let unexpected_eof = diagnostic
        .first()
        .expect("truncated insert body should compact to one record")
        .family_view::<UnexpectedEndOfFileView>()
        .is_some();
    assert!(
        unexpected_eof,
        "expected unexpected-end-of-file for truncated $insert body"
    );
    assert_span_is_meaningful(&diagnostic);
}

#[test]
fn insert_with_truncated_nested_body_in_helper_reports_eof() {
    let diagnostic = parse_template_diagnostic("[$insert(\"name\"): [: inner]");

    let unexpected_eof = diagnostic
        .first()
        .expect("$insert helper with truncated nested body should compact to one record")
        .family_view::<UnexpectedEndOfFileView>()
        .is_some();
    assert!(
        unexpected_eof,
        "expected unexpected-end-of-file for $insert helper with truncated nested body"
    );
    assert_span_is_meaningful(&diagnostic);
}

#[test]
fn slot_definition_with_body_is_rejected() {
    let diagnostic = parse_template_diagnostic("[$slot: body]");

    assert_invalid_template_structure(&diagnostic, InvalidTemplateStructureReason::SlotInHead);
    assert_span_is_meaningful(&diagnostic);
}
