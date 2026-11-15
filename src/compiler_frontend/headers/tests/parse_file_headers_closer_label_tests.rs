//! Focused header closer-label tests.
//!
//! WHAT: binds the diagnostic code, its typed construct and the label role to the authored site
//!       for a function body, a trait declaration, a choice declaration and an unclosed delimiter
//!       reported by header capture.
//! WHY: integration `secondary_labels` assertions bind sites only, so a swapped label role or a
//!      missing construct fact would still pass there. These tests name each one directly.

use super::*;
use crate::compiler_frontend::compiler_messages::SourceScopeConstruct;
use crate::compiler_frontend::compiler_messages::diagnostics::families::token_lexical::UnclosedSourceScopeView;
use crate::compiler_frontend::tests::parse_support::source_end_span;

/// The label roles these tests distinguish, so expectations never borrow stored label data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ExpectedLabelRole {
    UnclosedScopeStartsHere,
    UnclosedDelimiterStartsHere,
    InnerBlockCloser,
}

/// Assert the first header diagnostic's code, construct, primary site and label roles.
fn assert_unclosed_scope(
    diagnostics: &HeaderTestDiagnostics,
    source: &str,
    construct: SourceScopeConstruct,
    expected_reason_key: &str,
    expected: &[(ExpectedLabelRole, SourceSpan)],
) {
    let diagnostic = diagnostics
        .diagnostics
        .first()
        .expect("a rejected header should retain its diagnostic");
    assert_eq!(diagnostic.descriptor().code, "MOTH-SYNTAX-0042");
    assert_eq!(diagnostic.reason_key(), Some(expected_reason_key));
    let facts = diagnostic
        .family_view::<UnclosedSourceScopeView>()
        .expect("the unclosed scope should retain its typed construct");
    assert_eq!(facts.reason, construct);
    assert!(
        matches!(diagnostic.primary_site(), SiteRef::Local(span) if span == source_end_span(source)),
        "an unclosed source scope should report at the truncation"
    );
    assert_labels(diagnostic, expected);
}

/// Assert a delimiter EOF keeps the unexpected-end-of-file family and its opener label.
fn assert_delimiter_eof_labels(
    diagnostics: &HeaderTestDiagnostics,
    expected: &[(ExpectedLabelRole, SourceSpan)],
) {
    let diagnostic = diagnostics
        .diagnostics
        .first()
        .expect("a rejected header should retain its diagnostic");
    assert_eq!(diagnostic.descriptor().code, "MOTH-SYNTAX-0017");
    assert_labels(diagnostic, expected);
}

fn assert_labels(diagnostic: DiagnosticView<'_>, expected: &[(ExpectedLabelRole, SourceSpan)]) {
    let labels: Vec<_> = diagnostic.durable_labels().collect();
    assert_eq!(
        labels.len(),
        expected.len(),
        "the EOF report should carry exactly its opener labels"
    );
    for (label, (expected_role, expected_span)) in labels.iter().zip(expected) {
        let role_matches = match expected_role {
            ExpectedLabelRole::UnclosedScopeStartsHere => {
                matches!(label.message, DurableLabelMessage::UnclosedScopeStartsHere)
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
fn missing_function_closer_names_the_function_body() {
    let source = "compute || -> I32:\n    return 1\n";
    let result = parse_single_file_headers_with_entry(source, "src/@entry.moth", "src/@entry.moth");
    let errors = expect_header_error(result, "a missing function closer should be rejected");

    assert_unclosed_scope(
        &errors,
        source,
        SourceScopeConstruct::FunctionBody,
        "unclosed_source_scope.function_body",
        &[(
            ExpectedLabelRole::UnclosedScopeStartsHere,
            source_span_for(source, ":", 0),
        )],
    );
}

#[test]
fn missing_inner_if_closer_names_the_if_block() {
    let source = "compute || -> I32:\n    if ready:\n        return 1\n";
    let result = parse_single_file_headers_with_entry(source, "src/@entry.moth", "src/@entry.moth");
    let errors = expect_header_error(result, "a missing inner closer should be rejected");

    // The function body is still open above the `if`, so the innermost open construct owns the
    // report and only its opener is exact.
    assert_unclosed_scope(
        &errors,
        source,
        SourceScopeConstruct::IfBlock,
        "unclosed_source_scope.if_block",
        &[(
            ExpectedLabelRole::UnclosedScopeStartsHere,
            source_span_for(source, ":", 1),
        )],
    );
}

#[test]
fn stolen_function_closer_names_the_function_body_and_the_inner_closer() {
    let source =
        "ready ~= true\ncompute || -> I32:\n    if ready:\n        return 1\n    return 0\n;\n";
    let result = parse_single_file_headers_with_entry(source, "src/@entry.moth", "src/@entry.moth");
    let errors = expect_header_error(result, "a stolen function closer should be rejected");

    assert_unclosed_scope(
        &errors,
        source,
        SourceScopeConstruct::FunctionBody,
        "unclosed_source_scope.function_body",
        &[
            (
                ExpectedLabelRole::UnclosedScopeStartsHere,
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
fn unclosed_trait_names_the_trait_declaration() {
    let source = "MARKER must:\n";
    let result = parse_single_file_headers_with_entry(source, "src/@entry.moth", "src/@entry.moth");
    let errors = expect_header_error(result, "a missing trait closer should be rejected");

    assert_unclosed_scope(
        &errors,
        source,
        SourceScopeConstruct::TraitDeclaration,
        "unclosed_source_scope.trait_declaration",
        &[(
            ExpectedLabelRole::UnclosedScopeStartsHere,
            source_span_for(source, ":", 0),
        )],
    );
}

#[test]
fn unclosed_choice_names_the_choice_declaration() {
    let source = "Status ::\n    Ready,\n    Waiting\n";
    let result = parse_single_file_headers_with_entry(source, "src/@entry.moth", "src/@entry.moth");
    let errors = expect_header_error(result, "a missing choice closer should be rejected");

    assert_unclosed_scope(
        &errors,
        source,
        SourceScopeConstruct::ChoiceDeclaration,
        "unclosed_source_scope.choice_declaration",
        &[(
            ExpectedLabelRole::UnclosedScopeStartsHere,
            source_span_for(source, "::", 0),
        )],
    );
}

#[test]
fn unclosed_declared_region_names_the_region() {
    let source = "run ||:\n    request:\n        x = 1\n";
    let result = parse_single_file_headers_with_entry(source, "src/@entry.moth", "src/@entry.moth");
    let errors = expect_header_error(result, "an unclosed declared region should be rejected");

    // Declared regions remain a deferred feature, but capture still balances their `;`. When one
    // is the innermost open scope, the report names that region instead of inventing a function
    // closer.
    assert_unclosed_scope(
        &errors,
        source,
        SourceScopeConstruct::DeclaredRegion,
        "unclosed_source_scope.declared_region",
        &[(
            ExpectedLabelRole::UnclosedScopeStartsHere,
            source_span_for(source, ":", 1),
        )],
    );
}

#[test]
fn unclosed_parenthesis_before_a_function_closer_stays_a_delimiter_error() {
    let source = "compute || -> I32:\n    return (1 + 2\n;\n";
    let result = parse_single_file_headers_with_entry(source, "src/@entry.moth", "src/@entry.moth");
    let errors = expect_header_error(result, "an unclosed call should be rejected");

    assert_delimiter_eof_labels(
        &errors,
        &[(
            ExpectedLabelRole::UnclosedDelimiterStartsHere,
            source_span_for(source, "(", 0),
        )],
    );
}

#[test]
fn a_direct_child_closer_is_labelled_below_the_innermost_open_scope() {
    let source =
        "compute || -> I32:\n    if ready:\n        loop ready:\n            return 1\n    ;\n";
    let result = parse_single_file_headers_with_entry(source, "src/@entry.moth", "src/@entry.moth");
    let errors = expect_header_error(result, "the unclosed scopes should be rejected");

    // The `;` closed the loop, which was a direct child of the `if` that remains open with the
    // function body. Both openers are still exact context, and so is the closer that consumed the
    // loop's `;` instead of the `if`'s.
    assert_unclosed_scope(
        &errors,
        source,
        SourceScopeConstruct::IfBlock,
        "unclosed_source_scope.if_block",
        &[
            (
                ExpectedLabelRole::UnclosedScopeStartsHere,
                source_span_for(source, ":", 1),
            ),
            (
                ExpectedLabelRole::InnerBlockCloser,
                source_span_for(source, ";", 0),
            ),
        ],
    );
}

#[test]
fn grouped_semicolon_precedes_a_later_invalid_inner_closer() {
    let source = "run ||:\n    io.line(1; 2)\n    if ready:\n        io.line(\"x\");\n";
    let result = parse_single_file_headers_with_entry(source, "src/@entry.moth", "src/@entry.moth");
    let errors = expect_header_error(result, "the unclosed function should be rejected");

    // The `;` inside the call is an earlier authored error that only the grouped AST path can
    // name, so the later misplaced inner closer must not take its place.
    assert_unclosed_scope(
        &errors,
        source,
        SourceScopeConstruct::FunctionBody,
        "unclosed_source_scope.function_body",
        &[
            (
                ExpectedLabelRole::UnclosedScopeStartsHere,
                source_span_for(source, ":", 0),
            ),
            (
                ExpectedLabelRole::InnerBlockCloser,
                source_span_for(source, ";", 1),
            ),
        ],
    );
}
