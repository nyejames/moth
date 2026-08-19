//! Typed diagnostic assertion helpers for tests.
//!
//! WHAT: exact diagnostic code, infrastructure error and reason assertions.
//! WHY: a bare `is_err()` or `error_count() > 0` accepts the wrong failure at
//!   multi-lane boundaries. These helpers prove the exact diagnostic kind,
//!   infrastructure error type or authored reason so an unrelated error
//!   cannot satisfy the intended contract.

use crate::compiler_frontend::compiler_errors::{
    CompilerErrorMetadataKey, CompilerMessages, ErrorType,
};
use crate::compiler_frontend::compiler_messages::DiagnosticSeverity;
use std::collections::BTreeMap;

/// Assert that the diagnostic codes of all error-severity diagnostics match
/// the expected multiset exactly.
///
/// WHAT: collects every error diagnostic's stable code into a sorted multiset
///   and compares against the expected multiset.
/// WHY: `error_count() > 0` or `is_err()` accepts any error. Exact codes prove
///   the intended diagnostic, not just that some error happened.
#[track_caller]
pub fn assert_exact_diagnostic_codes(messages: &CompilerMessages, expected: &[&str]) {
    let mut actual: Vec<&str> = messages
        .diagnostics()
        .filter(|d| d.severity == DiagnosticSeverity::Error)
        .map(|d| d.kind.code())
        .collect();
    actual.sort_unstable();
    let mut expected_sorted: Vec<&str> = expected.to_vec();
    expected_sorted.sort_unstable();
    assert_eq!(
        actual, expected_sorted,
        "diagnostic codes must match exactly"
    );
}

/// Assert that `messages` contains no infrastructure errors.
#[track_caller]
pub fn assert_no_infrastructure_errors(messages: &CompilerMessages) {
    let infra_errors: Vec<_> = messages.infrastructure_errors_for_tests().collect();
    assert!(
        infra_errors.is_empty(),
        "expected no infrastructure errors, found: {infra_errors:?}"
    );
}

/// Assert that `messages` contains exactly one error diagnostic overall, and
/// that error is an infrastructure error of the expected `ErrorType`.
///
/// WHAT: verifies the total error diagnostic count is 1, that error is an
///   infrastructure error, and its `ErrorType` matches.
/// WHY: `assert_exact_infrastructure_error` should not pass when additional
///   non-infrastructure error diagnostics accompany the expected one. A
///   missing-file failure should produce exactly one File infrastructure error,
///   not that plus an unrelated semantic error.
#[track_caller]
pub fn assert_exact_infrastructure_error(messages: &CompilerMessages, expected_type: &ErrorType) {
    let total_errors: Vec<_> = messages
        .diagnostics()
        .filter(|d| d.severity == DiagnosticSeverity::Error)
        .collect();
    assert_eq!(
        total_errors.len(),
        1,
        "expected exactly one error diagnostic overall, found {}: {total_errors:?}",
        total_errors.len()
    );

    let infra_errors: Vec<_> = messages.infrastructure_errors_for_tests().collect();
    assert_eq!(
        infra_errors.len(),
        1,
        "expected the single error to be an infrastructure error, found {}: {infra_errors:?}",
        infra_errors.len()
    );
    assert_eq!(
        infra_errors[0].0, expected_type,
        "infrastructure error type mismatch"
    );
}

/// Assert that `messages` contains exactly one error diagnostic, that it is
/// an infrastructure error of type `File`, and that it carries the expected
/// `OutputRejectionReason` metadata value.
///
/// WHAT: extracts the `OutputRejectionReason` metadata from the infrastructure
///   error payload and compares it against the expected reason string.
/// WHY: all output-writer rejections share `ErrorType::File`. The typed reason
///   seam distinguishes between distinct safety contracts (invalid path,
///   duplicate destination, symlink escape, etc.) so a test for one contract
///   cannot pass when a different contract is violated.
#[track_caller]
pub fn assert_output_rejection(messages: &CompilerMessages, expected_reason: &str) {
    assert_exact_infrastructure_error(messages, &ErrorType::File);

    let payloads: Vec<_> = messages.infrastructure_error_payloads_for_tests().collect();
    let payload = &payloads[0];
    let actual_reason = match payload {
        crate::compiler_frontend::compiler_messages::DiagnosticPayload::InfrastructureError {
            metadata,
            ..
        } => metadata
            .get(&CompilerErrorMetadataKey::OutputRejectionReason)
            .map(|s| s.as_str())
            .unwrap_or("<none>"),
        _ => "<none>",
    };
    assert_eq!(
        actual_reason, expected_reason,
        "output rejection reason mismatch: expected '{expected_reason}', got '{actual_reason}'"
    );
}

/// Assert that the error diagnostic with the given `code` and 1-based
/// `occurrence` carries the expected stable reason key.
///
/// WHAT: finds the n-th occurrence of `code` among error diagnostics and checks
///   that its `diagnostic.identity().reason_key` matches the expected value.
/// WHY: broad `is_err()` accepts any failure. Reason assertions prove the
///   correct diagnostic lane, not just that an error was emitted. Reason keys
///   come from compiler payload identity — this helper does not invent a
///   parallel reason taxonomy.
#[track_caller]
#[allow(dead_code)]
pub fn assert_diagnostic_reason(
    messages: &CompilerMessages,
    code: &str,
    occurrence: usize,
    expected_reason: &str,
) {
    let matching: Vec<_> = messages
        .diagnostics()
        .filter(|d| d.severity == DiagnosticSeverity::Error && d.kind.code() == code)
        .collect();

    assert!(
        occurrence >= 1 && occurrence <= matching.len(),
        "diagnostic code '{code}' has {} occurrence(s), cannot select occurrence {occurrence}",
        matching.len()
    );

    let diagnostic = matching[occurrence - 1];
    let identity = diagnostic.identity();
    let actual_reason = identity.reason_key.unwrap_or("<none>");
    assert_eq!(
        actual_reason, expected_reason,
        "diagnostic '{code}' occurrence {occurrence} has reason '{actual_reason}', \
         expected '{expected_reason}'"
    );
}

/// Build an exact count map of error diagnostic codes.
///
/// WHAT: returns a `BTreeMap` from code to occurrence count.
/// WHY: useful for comparing multisets in tests that need exact cardinality.
#[track_caller]
#[allow(dead_code)]
pub fn error_code_counts(messages: &CompilerMessages) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for diagnostic in messages.diagnostics() {
        if diagnostic.severity == DiagnosticSeverity::Error {
            *counts
                .entry(diagnostic.kind.code().to_string())
                .or_insert(0) += 1;
        }
    }
    counts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler_frontend::compiler_errors::SourceLocation;
    use crate::compiler_frontend::compiler_messages::{
        CompilerDiagnostic, DiagnosticKind, DiagnosticPayload, DiagnosticSeverity,
        RuleDiagnosticKind,
    };
    use crate::compiler_frontend::symbols::string_interning::StringTable;

    fn messages_with_errors(
        diagnostics: Vec<CompilerDiagnostic>,
        table: StringTable,
    ) -> CompilerMessages {
        CompilerMessages::from_diagnostics(diagnostics, table)
    }

    #[test]
    fn assert_exact_diagnostic_codes_matches_single_error() {
        let table = StringTable::new();
        let diagnostic = CompilerDiagnostic::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::UnknownName),
            SourceLocation::default(),
            DiagnosticPayload::None,
        );
        let messages = messages_with_errors(vec![diagnostic], table);
        assert_exact_diagnostic_codes(&messages, &["MOTH-RULE-0001"]);
    }

    #[test]
    fn assert_exact_diagnostic_codes_rejects_wrong_count() {
        let table = StringTable::new();
        let diagnostic = CompilerDiagnostic::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::UnknownName),
            SourceLocation::default(),
            DiagnosticPayload::None,
        );
        let messages = messages_with_errors(vec![diagnostic], table);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            assert_exact_diagnostic_codes(&messages, &["MOTH-RULE-0001", "MOTH-RULE-0001"]);
        }));
        assert!(result.is_err(), "should panic for wrong count");
    }

    #[test]
    fn assert_no_infrastructure_errors_accepts_clean_messages() {
        let table = StringTable::new();
        let messages = messages_with_errors(vec![], table);
        assert_no_infrastructure_errors(&messages);
    }

    #[test]
    fn assert_no_infrastructure_errors_rejects_infra_error() {
        let table = StringTable::new();
        let diagnostic = CompilerDiagnostic::with_severity(
            DiagnosticKind::Infrastructure(
                crate::compiler_frontend::compiler_messages::InfrastructureDiagnosticKind::InfrastructureFailure,
            ),
            DiagnosticSeverity::Error,
            SourceLocation::default(),
            DiagnosticPayload::InfrastructureError {
                msg: "test failure".to_string(),
                error_type: ErrorType::File,
                metadata: std::collections::HashMap::new(),
            },
        );
        let messages = messages_with_errors(vec![diagnostic], table);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            assert_no_infrastructure_errors(&messages);
        }));
        assert!(result.is_err(), "should panic for an infrastructure error");
    }

    #[test]
    fn assert_exact_infrastructure_error_matches_type() {
        let table = StringTable::new();
        let diagnostic = CompilerDiagnostic::with_severity(
            DiagnosticKind::Infrastructure(
                crate::compiler_frontend::compiler_messages::InfrastructureDiagnosticKind::InfrastructureFailure,
            ),
            DiagnosticSeverity::Error,
            SourceLocation::default(),
            DiagnosticPayload::InfrastructureError {
                msg: "file not found".to_string(),
                error_type: ErrorType::File,
                metadata: std::collections::HashMap::new(),
            },
        );
        let messages = messages_with_errors(vec![diagnostic], table);
        assert_exact_infrastructure_error(&messages, &ErrorType::File);
    }

    #[test]
    fn assert_exact_infrastructure_error_rejects_additional_rule_error() {
        let table = StringTable::new();
        let infra_diagnostic = CompilerDiagnostic::with_severity(
            DiagnosticKind::Infrastructure(
                crate::compiler_frontend::compiler_messages::InfrastructureDiagnosticKind::InfrastructureFailure,
            ),
            DiagnosticSeverity::Error,
            SourceLocation::default(),
            DiagnosticPayload::InfrastructureError {
                msg: "file not found".to_string(),
                error_type: ErrorType::File,
                metadata: std::collections::HashMap::new(),
            },
        );
        let rule_diagnostic = CompilerDiagnostic::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::UnknownName),
            SourceLocation::default(),
            DiagnosticPayload::None,
        );
        let messages = messages_with_errors(vec![infra_diagnostic, rule_diagnostic], table);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            assert_exact_infrastructure_error(&messages, &ErrorType::File);
        }));
        assert!(
            result.is_err(),
            "should panic when an additional non-infrastructure error is present"
        );
    }

    #[test]
    fn assert_exact_infrastructure_error_rejects_wrong_count() {
        let table = StringTable::new();
        let diagnostic1 = CompilerDiagnostic::with_severity(
            DiagnosticKind::Infrastructure(
                crate::compiler_frontend::compiler_messages::InfrastructureDiagnosticKind::InfrastructureFailure,
            ),
            DiagnosticSeverity::Error,
            SourceLocation::default(),
            DiagnosticPayload::InfrastructureError {
                msg: "first".to_string(),
                error_type: ErrorType::File,
                metadata: std::collections::HashMap::new(),
            },
        );
        let diagnostic2 = CompilerDiagnostic::with_severity(
            DiagnosticKind::Infrastructure(
                crate::compiler_frontend::compiler_messages::InfrastructureDiagnosticKind::InfrastructureFailure,
            ),
            DiagnosticSeverity::Error,
            SourceLocation::default(),
            DiagnosticPayload::InfrastructureError {
                msg: "second".to_string(),
                error_type: ErrorType::Config,
                metadata: std::collections::HashMap::new(),
            },
        );
        let messages = messages_with_errors(vec![diagnostic1, diagnostic2], table);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            assert_exact_infrastructure_error(&messages, &ErrorType::File);
        }));
        assert!(result.is_err(), "should panic for two infra errors");
    }
}
