//! Focused header closer-label tests.
//!
//! WHAT: binds the diagnostic label role to the authored site for a function body, a trait body
//!       and an unclosed delimiter reported by header capture.
//! WHY: integration `secondary_labels` assertions bind sites only, so a swapped label role would
//!      still pass there. These tests name the role each site must carry.

use super::*;

/// The label roles these tests distinguish, so expectations never borrow stored label data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ExpectedLabelRole {
    UnclosedBlockStartsHere,
    UnclosedDelimiterStartsHere,
    InnerBlockCloser,
}

fn assert_eof_labels(
    diagnostics: &HeaderTestDiagnostics,
    expected: &[(ExpectedLabelRole, SourceSpan)],
) {
    let diagnostic = diagnostics
        .diagnostics
        .first()
        .expect("a rejected header should retain its diagnostic");
    assert_eq!(diagnostic.descriptor().code, "MOTH-SYNTAX-0017");
    let labels: Vec<_> = diagnostic.durable_labels().collect();
    assert_eq!(
        labels.len(),
        expected.len(),
        "the EOF report should carry exactly its opener labels"
    );
    for (label, (expected_role, expected_span)) in labels.iter().zip(expected) {
        let role_matches = match expected_role {
            ExpectedLabelRole::UnclosedBlockStartsHere => {
                matches!(label.message, DurableLabelMessage::UnclosedBlockStartsHere)
            }
            ExpectedLabelRole::UnclosedDelimiterStartsHere => matches!(
                label.message,
                DurableLabelMessage::UnclosedDelimiterStartsHere
            ),
            ExpectedLabelRole::InnerBlockCloser => {
                matches!(label.message, DurableLabelMessage::InnerBlockCloser)
            }
        };
        assert!(role_matches, "unexpected label role {:?}", label.message);
        let span = match label.site {
            SiteRef::Local(span) => span,
            SiteRef::Absent | SiteRef::Foreign { .. } => {
                panic!("the opener label should bind local authored source")
            }
        };
        assert_eq!(&span, expected_span, "label bound to the wrong site");
    }
}

#[test]
fn missing_function_closer_labels_the_opening_colon() {
    let source = "compute || -> I32:\n    return 1\n";
    let result = parse_single_file_headers_with_entry(source, "src/@entry.moth", "src/@entry.moth");
    let errors = expect_header_error(result, "a missing function closer should be rejected");

    assert_eof_labels(
        &errors,
        &[(
            ExpectedLabelRole::UnclosedBlockStartsHere,
            source_span_for(source, ":", 0),
        )],
    );
}

#[test]
fn stolen_function_closer_labels_the_function_colon_and_the_inner_closer() {
    let source =
        "ready ~= true\ncompute || -> I32:\n    if ready:\n        return 1\n    return 0\n;\n";
    let result = parse_single_file_headers_with_entry(source, "src/@entry.moth", "src/@entry.moth");
    let errors = expect_header_error(result, "a stolen function closer should be rejected");

    assert_eof_labels(
        &errors,
        &[
            (
                ExpectedLabelRole::UnclosedBlockStartsHere,
                source_span_for(source, ":", 0),
            ),
            (
                ExpectedLabelRole::InnerBlockCloser,
                source_span_for(source, ";", 0),
            ),
        ],
    );
}

#[test]
fn unclosed_trait_labels_the_must_colon() {
    let source = "NAMED must:\n    name |~This| -> String\n";
    let result = parse_single_file_headers_with_entry(source, "src/@entry.moth", "src/@entry.moth");
    let errors = expect_header_error(result, "a missing trait closer should be rejected");

    assert_eof_labels(
        &errors,
        &[(
            ExpectedLabelRole::UnclosedBlockStartsHere,
            source_span_for(source, ":", 0),
        )],
    );
}

#[test]
fn unclosed_parenthesis_before_a_function_closer_labels_the_paren() {
    let source = "compute || -> I32:\n    return (1 + 2\n;\n";
    let result = parse_single_file_headers_with_entry(source, "src/@entry.moth", "src/@entry.moth");
    let errors = expect_header_error(result, "an unclosed call should be rejected");

    assert_eof_labels(
        &errors,
        &[(
            ExpectedLabelRole::UnclosedDelimiterStartsHere,
            source_span_for(source, "(", 0),
        )],
    );
}

#[test]
fn deeper_open_scopes_do_not_claim_the_inner_closer() {
    let source =
        "compute || -> I32:\n    if ready:\n        loop ready:\n            return 1\n    ;\n";
    let result = parse_single_file_headers_with_entry(source, "src/@entry.moth", "src/@entry.moth");
    let errors = expect_header_error(result, "the unclosed scopes should be rejected");

    // The `;` closed the loop and is recorded as an inner closer, but the `if` block is still open
    // above the function body. Only the innermost open block owns the report, so the closer
    // explanation would be inexact and must not appear.
    assert_eof_labels(
        &errors,
        &[(
            ExpectedLabelRole::UnclosedBlockStartsHere,
            source_span_for(source, ":", 1),
        )],
    );
}
