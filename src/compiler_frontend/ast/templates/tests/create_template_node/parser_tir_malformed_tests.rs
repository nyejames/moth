//! Focused malformed-template tests for parser-to-TIR surfaces.
//!
//! WHAT: adds diagnostic parity coverage around body text, nested child
//! templates, control-flow sentinels, slot/insert helpers, and suppressed
//! child-template brackets now that the parser emits TIR nodes directly.
//!
//! WHY: body and head parsing record TIR nodes incrementally. If an
//! error is raised after some nodes have been recorded, the diagnostic reason
//! and source span must remain stable. These tests pin the
//! expected malformed-surface behavior without relying on internal TIR IDs.

use super::*;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticPayload, DiagnosticToken, InvalidTemplateStructureReason,
};
use crate::compiler_frontend::source::SourceId;
use crate::compiler_frontend::symbols::string_interning::StringTable;

/// Parses a template that is expected to fail and returns the diagnostic.
fn parse_template_diagnostic(source: &str) -> CompilerDiagnostic {
    let mut string_table = StringTable::new();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut token_stream =
        template_tokens_from_source(source, &mut string_table, &mut span_builder);
    let context = new_constant_context(token_stream.src_path.clone());

    expect_template_diagnostic(
        Template::new(&mut token_stream, &context, vec![], &mut string_table, &mut PathInternerFork::empty())
            .expect_err("template source should fail to parse"),
    )
}

fn parse_template_diagnostic_with_replaced_body_token(
    source: &str,
) -> (CompilerDiagnostic, ExtendedSpanBuilder) {
    let mut string_table = StringTable::new();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut token_stream =
        template_tokens_from_source(source, &mut string_table, &mut span_builder);
    // Normal template-body lexing emits text, newline, nested-template, and close tokens only.
    // Replace the retained text token to exercise the parser's defensive unexpected-token lane
    // while keeping its authored UTF-8 span and source identity intact.
    let body_token = token_stream
        .tokens
        .iter_mut()
        .find(|token| matches!(token.kind, TokenKind::StringSliceLiteral(_)))
        .expect("template source should contain a body token");
    body_token.kind = TokenKind::Comma;
    let context = new_constant_context(token_stream.src_path.clone());

    let diagnostic = expect_template_diagnostic(
        Template::new(&mut token_stream, &context, vec![], &mut string_table, &mut PathInternerFork::empty())
            .expect_err("template source should fail to parse"),
    );
    (diagnostic, span_builder)
}

fn parse_template_diagnostic_with_span_builder(
    source: &str,
) -> (CompilerDiagnostic, ExtendedSpanBuilder) {
    let mut string_table = StringTable::new();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut token_stream =
        template_tokens_from_source(source, &mut string_table, &mut span_builder);
    let context = new_constant_context(token_stream.src_path.clone());

    let diagnostic = expect_template_diagnostic(
        Template::new(&mut token_stream, &context, vec![], &mut string_table, &mut PathInternerFork::empty())
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

fn assert_exact_marker_span(
    diagnostic: &CompilerDiagnostic,
    source: &str,
    span_builder: &ExtendedSpanBuilder,
) {
    let marker_start = source
        .find("else")
        .expect("test source should contain an else marker");
    let marker_end = marker_start + "else".len();
    let primary_span = diagnostic
        .primary_span
        .expect("else boundary diagnostic should retain its exact marker span");
    assert_eq!(primary_span.source(), SourceId::COMPILATION_ROOT);
    let range = primary_span.resolve_with(span_builder.resolver_for(SourceId::COMPILATION_ROOT));
    assert_eq!(
        (range.start(), range.end()),
        (marker_start as u32, marker_end as u32)
    );
    assert_eq!(&source[marker_start..marker_end], "else");
    assert!(diagnostic.labels.is_empty());
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
fn orphan_template_break_retains_exact_marker_span() {
    let source = "[: before [break] after]";
    let (diagnostic, span_builder) = parse_template_diagnostic_with_span_builder(source);

    assert_invalid_template_structure(
        &diagnostic,
        InvalidTemplateStructureReason::OrphanTemplateBreak,
    );
    let primary_span = diagnostic
        .primary_span
        .expect("orphan loop control should retain its exact marker span");
    assert_eq!(primary_span.source(), SourceId::COMPILATION_ROOT);
    let range = primary_span.resolve_with(span_builder.resolver_for(SourceId::COMPILATION_ROOT));
    assert_eq!((range.start(), range.end()), (11, 16));
    assert_eq!(
        &source[range.start() as usize..range.end() as usize],
        "break"
    );
    assert!(diagnostic.labels.is_empty());
}

#[test]
fn orphan_template_else_retains_exact_multibyte_marker_span() {
    let source = "[: π\n[else] after]";
    let (diagnostic, span_builder) = parse_template_diagnostic_with_span_builder(source);

    assert_invalid_template_structure(
        &diagnostic,
        InvalidTemplateStructureReason::OrphanTemplateElse,
    );
    let primary_span = diagnostic
        .primary_span
        .expect("orphan template else should retain its exact marker span");
    assert_eq!(primary_span.source(), SourceId::COMPILATION_ROOT);
    let range = primary_span.resolve_with(span_builder.resolver_for(SourceId::COMPILATION_ROOT));
    assert_eq!((range.start(), range.end()), (7, 11));
    assert_eq!(
        &source[range.start() as usize..range.end() as usize],
        "else"
    );
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
fn malformed_loop_control_missing_close_reports_malformed_break() {
    let diagnostic = parse_template_diagnostic("[loop true: body [break");

    assert_invalid_template_structure(
        &diagnostic,
        InvalidTemplateStructureReason::MalformedTemplateBreak,
    );
    assert_span_is_meaningful(&diagnostic);
}

#[test]
fn malformed_loop_control_missing_close_reports_malformed_continue() {
    let diagnostic = parse_template_diagnostic("[loop true: body [continue");

    assert_invalid_template_structure(
        &diagnostic,
        InvalidTemplateStructureReason::MalformedTemplateContinue,
    );
    assert_span_is_meaningful(&diagnostic);
}

#[test]
fn orphan_break_in_normal_body_reports_orphan_break() {
    let diagnostic = parse_template_diagnostic("[: before [break] after]");

    assert_invalid_template_structure(
        &diagnostic,
        InvalidTemplateStructureReason::OrphanTemplateBreak,
    );
    assert_span_is_meaningful(&diagnostic);
}

#[test]
fn orphan_continue_in_normal_body_reports_orphan_continue() {
    let diagnostic = parse_template_diagnostic("[: before [continue] after]");

    assert_invalid_template_structure(
        &diagnostic,
        InvalidTemplateStructureReason::OrphanTemplateContinue,
    );
    assert_span_is_meaningful(&diagnostic);
}

#[test]
fn children_directive_truncated_argument_template_reports_eof_with_meaningful_span() {
    let diagnostic = parse_template_diagnostic("[$children([: unclosed): body]");

    let has_expected_payload = match &diagnostic.payload {
        DiagnosticPayload::UnexpectedEndOfFile { .. } => true,
        DiagnosticPayload::ExpectedToken { expected, .. } => {
            *expected == DiagnosticToken::from(TokenKind::CloseParenthesis)
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

#[test]
fn malformed_else_if_missing_condition_keeps_non_default_span() {
    let diagnostic = parse_template_diagnostic("[if true:\n    Then\n[else if]\n    Hidden\n]");

    assert_invalid_template_structure(
        &diagnostic,
        InvalidTemplateStructureReason::MissingTemplateElseIfCondition,
    );
    assert_span_is_meaningful(&diagnostic);
}

#[test]
fn malformed_else_if_missing_condition_retains_exact_multibyte_marker_span() {
    let source = "[if true:\n    π\n[else if]\n    Hidden\n]";
    let (diagnostic, span_builder) = parse_template_diagnostic_with_span_builder(source);

    assert_invalid_template_structure(
        &diagnostic,
        InvalidTemplateStructureReason::MissingTemplateElseIfCondition,
    );
    assert_exact_marker_span(&diagnostic, source, &span_builder);
}

#[test]
fn malformed_else_if_sentinel_retains_exact_multibyte_marker_span() {
    let source = "[if true:\n    π\n[else if false, nope]\n    Hidden\n]";
    let (diagnostic, span_builder) = parse_template_diagnostic_with_span_builder(source);

    assert_invalid_template_structure(
        &diagnostic,
        InvalidTemplateStructureReason::MalformedTemplateElseIf,
    );
    assert_exact_marker_span(&diagnostic, source, &span_builder);
}

#[test]
fn inline_else_if_boundary_retains_exact_multibyte_marker_span() {
    let source = "[if true:\n    π\n[else if false] inline\n]";
    let (diagnostic, span_builder) = parse_template_diagnostic_with_span_builder(source);

    assert_invalid_template_structure(
        &diagnostic,
        InvalidTemplateStructureReason::InlineTemplateElseIf,
    );
    assert_exact_marker_span(&diagnostic, source, &span_builder);
}

#[test]
fn inline_else_fallback_boundary_retains_exact_multibyte_marker_span() {
    let source = "[if true:\n    π\n[else] inline\n]";
    let (diagnostic, span_builder) = parse_template_diagnostic_with_span_builder(source);

    assert_invalid_template_structure(
        &diagnostic,
        InvalidTemplateStructureReason::InlineTemplateElse,
    );
    assert_exact_marker_span(&diagnostic, source, &span_builder);
}

#[test]
fn malformed_else_sentinel_keeps_non_default_span() {
    let diagnostic = parse_template_diagnostic("[if true:\nThen\n[else: nope]\n]");

    assert_invalid_template_structure(
        &diagnostic,
        InvalidTemplateStructureReason::MalformedTemplateElse,
    );
    assert_span_is_meaningful(&diagnostic);
}
