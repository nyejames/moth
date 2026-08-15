//! Focused malformed-template tests for parser-to-TIR surfaces.
//!
//! WHAT: adds diagnostic parity coverage around body text, nested child
//! templates, control-flow sentinels, slot/insert helpers, and suppressed
//! child-template brackets now that the parser emits TIR nodes directly.
//!
//! WHY: body and head parsing record TIR nodes incrementally. If an
//! error is raised after some nodes have been recorded, the diagnostic reason
//! and source location must remain stable. These tests pin the
//! expected malformed-surface behavior without relying on internal TIR IDs.

use super::*;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticPayload, InvalidTemplateStructureReason,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;

/// Parses a template that is expected to fail and returns the diagnostic.
fn parse_template_diagnostic(source: &str) -> CompilerDiagnostic {
    let mut string_table = StringTable::new();
    let mut token_stream = template_tokens_from_source(source, &mut string_table);
    let context = new_constant_context(token_stream.src_path.clone());

    expect_template_diagnostic(
        Template::new(&mut token_stream, &context, vec![], &mut string_table)
            .expect_err("template source should fail to parse"),
    )
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

/// Asserts that the diagnostic points at a meaningful source location rather
/// than the default zeroed location used for synthetic errors.
fn assert_location_is_meaningful(diagnostic: &CompilerDiagnostic) {
    assert!(
        !is_default_error_location(&diagnostic.primary_location),
        "diagnostic should carry a meaningful source location, got {:?}",
        diagnostic.primary_location
    );
}

#[test]
fn truncated_nested_template_body_reports_eof_with_meaningful_location() {
    let diagnostic = parse_template_diagnostic("[: outer [: inner]");

    assert!(
        matches!(
            diagnostic.payload,
            DiagnosticPayload::UnexpectedEndOfFile { .. }
        ),
        "expected unexpected-end-of-file for truncated nested body, got {:?}",
        diagnostic.payload
    );
    assert_location_is_meaningful(&diagnostic);
}

#[test]
fn malformed_loop_control_missing_close_reports_malformed_break() {
    let diagnostic = parse_template_diagnostic("[loop true: body [break");

    assert_invalid_template_structure(
        &diagnostic,
        InvalidTemplateStructureReason::MalformedTemplateBreak,
    );
    assert_location_is_meaningful(&diagnostic);
}

#[test]
fn malformed_loop_control_missing_close_reports_malformed_continue() {
    let diagnostic = parse_template_diagnostic("[loop true: body [continue");

    assert_invalid_template_structure(
        &diagnostic,
        InvalidTemplateStructureReason::MalformedTemplateContinue,
    );
    assert_location_is_meaningful(&diagnostic);
}

#[test]
fn orphan_break_in_normal_body_reports_orphan_break() {
    let diagnostic = parse_template_diagnostic("[: before [break] after]");

    assert_invalid_template_structure(
        &diagnostic,
        InvalidTemplateStructureReason::OrphanTemplateBreak,
    );
    assert_location_is_meaningful(&diagnostic);
}

#[test]
fn orphan_continue_in_normal_body_reports_orphan_continue() {
    let diagnostic = parse_template_diagnostic("[: before [continue] after]");

    assert_invalid_template_structure(
        &diagnostic,
        InvalidTemplateStructureReason::OrphanTemplateContinue,
    );
    assert_location_is_meaningful(&diagnostic);
}

#[test]
fn children_directive_truncated_argument_template_reports_eof_with_meaningful_location() {
    let diagnostic = parse_template_diagnostic("[$children([: unclosed): body]");

    assert!(
        matches!(
            diagnostic.payload,
            DiagnosticPayload::UnexpectedEndOfFile { .. }
                | DiagnosticPayload::ExpectedToken {
                    expected: TokenKind::CloseParenthesis,
                    ..
                }
        ),
        "expected unexpected-end-of-file or missing-close-paren for truncated $children argument template, got {:?}",
        diagnostic.payload
    );
    assert_location_is_meaningful(&diagnostic);
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
    assert_location_is_meaningful(&diagnostic);
}

#[test]
fn insert_with_truncated_body_reports_eof_with_meaningful_location() {
    let diagnostic = parse_template_diagnostic("[$insert(\"name\"): body");

    assert!(
        matches!(
            diagnostic.payload,
            DiagnosticPayload::UnexpectedEndOfFile { .. }
        ),
        "expected unexpected-end-of-file for truncated $insert body, got {:?}",
        diagnostic.payload
    );
    assert_location_is_meaningful(&diagnostic);
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
    assert_location_is_meaningful(&diagnostic);
}

#[test]
fn slot_definition_with_body_is_rejected() {
    let diagnostic = parse_template_diagnostic("[$slot: body]");

    assert_invalid_template_structure(&diagnostic, InvalidTemplateStructureReason::SlotInHead);
    assert_location_is_meaningful(&diagnostic);
}

#[test]
fn malformed_else_if_missing_condition_keeps_non_default_location() {
    let diagnostic = parse_template_diagnostic("[if true:\n    Then\n[else if]\n    Hidden\n]");

    assert_invalid_template_structure(
        &diagnostic,
        InvalidTemplateStructureReason::MissingTemplateElseIfCondition,
    );
    assert_location_is_meaningful(&diagnostic);
}

#[test]
fn malformed_else_sentinel_keeps_non_default_location() {
    let diagnostic = parse_template_diagnostic("[if true:\nThen\n[else: nope]\n]");

    assert_invalid_template_structure(
        &diagnostic,
        InvalidTemplateStructureReason::MalformedTemplateElse,
    );
    assert_location_is_meaningful(&diagnostic);
}
