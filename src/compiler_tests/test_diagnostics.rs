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

/// Assert that `messages` carries no outer infrastructure failure.
#[track_caller]
pub fn assert_no_infrastructure_errors(messages: &CompilerMessages) {
    assert!(
        messages.infrastructure_error().is_none(),
        "expected no infrastructure errors, found: {:?}",
        messages.infrastructure_error()
    );
}

/// Assert that `messages` contains no user-facing error diagnostic and exactly one outer
/// infrastructure failure of the expected `ErrorType`.
///
/// WHAT: verifies the diagnostic stream holds no `Error` diagnostic, the outer lane is present,
///   and its `ErrorType` matches.
/// WHY: `assert_exact_infrastructure_error` should not pass when additional user-facing
///   error diagnostics accompany the expected failure. A missing-file failure should
///   produce exactly one File infrastructure failure, not that plus an unrelated
///   semantic error.
#[track_caller]
pub fn assert_exact_infrastructure_error(messages: &CompilerMessages, expected_type: &ErrorType) {
    let user_errors: Vec<_> = messages
        .diagnostics()
        .filter(|d| d.severity == DiagnosticSeverity::Error)
        .collect();
    assert!(
        user_errors.is_empty(),
        "expected no user-facing error diagnostics, found: {user_errors:?}"
    );

    let Some(error) = messages.infrastructure_error() else {
        panic!("expected one outer infrastructure error, found none");
    };
    assert_eq!(
        &error.error_type, expected_type,
        "infrastructure error type mismatch"
    );
}

/// Assert that `messages` carries exactly one outer infrastructure failure of type `File`
/// with the expected `OutputRejectionReason` metadata value.
///
/// WHAT: extracts the `OutputRejectionReason` metadata from the outer `CompilerError` and
///   compares it against the expected reason string.
/// WHY: all output-writer rejections share `ErrorType::File`. The typed reason
///   seam distinguishes between distinct safety contracts (invalid path,
///   duplicate destination, symlink escape, etc.) so a test for one contract
///   cannot pass when a different contract is violated.
#[track_caller]
pub fn assert_output_rejection(messages: &CompilerMessages, expected_reason: &str) {
    assert_exact_infrastructure_error(messages, &ErrorType::File);

    let error = messages
        .infrastructure_error()
        .expect("exact infrastructure failure must be present");
    // An absent reason is a defect in the production seam, not a reason value to compare
    // against: without it the test would silently degrade to "some File error happened".
    let actual_reason = error
        .metadata
        .get(&CompilerErrorMetadataKey::OutputRejectionReason)
        .map(|reason| reason.as_str())
        .unwrap_or_else(|| {
            panic!(
                "infrastructure error carries no OutputRejectionReason metadata, so the \
                 rejection contract '{expected_reason}' cannot be proved; metadata: {:?}",
                error.metadata
            )
        });
    assert_eq!(
        actual_reason, expected_reason,
        "output rejection reason mismatch: expected '{expected_reason}', got '{actual_reason}'"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler_frontend::compiler_messages::{
        CompilerDiagnostic, DiagnosticKind, DiagnosticPayload, RuleDiagnosticKind,
    };
    use crate::compiler_frontend::symbols::string_interning::StringTable;
    use crate::compiler_tests::test_support::assert_panics_with;

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
            None,
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
            None,
            DiagnosticPayload::None,
        );
        let messages = messages_with_errors(vec![diagnostic], table);
        assert_panics_with("diagnostic codes must match exactly", || {
            assert_exact_diagnostic_codes(&messages, &["MOTH-RULE-0001", "MOTH-RULE-0001"]);
        });
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
        let messages = CompilerMessages::from_error(
            crate::compiler_frontend::compiler_errors::CompilerError::compiler_error(
                "test failure",
            ),
            table,
        );
        assert_panics_with("expected no infrastructure errors", || {
            assert_no_infrastructure_errors(&messages);
        });
    }

    #[test]
    fn assert_exact_infrastructure_error_matches_type() {
        let table = StringTable::new();
        let error = crate::compiler_frontend::compiler_errors::CompilerError::file_error(
            std::path::Path::new("missing.moth"),
            "file not found",
        );
        let messages = CompilerMessages::from_error(error, table);
        assert_exact_infrastructure_error(&messages, &ErrorType::File);
    }

    #[test]
    fn assert_exact_infrastructure_error_rejects_additional_rule_error() {
        let table = StringTable::new();
        let rule_diagnostic = CompilerDiagnostic::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::UnknownName),
            None,
            DiagnosticPayload::None,
        );
        let mut messages = CompilerMessages::from_error(
            crate::compiler_frontend::compiler_errors::CompilerError::compiler_error(
                "file not found",
            ),
            table,
        );
        messages.extend_diagnostics(vec![rule_diagnostic]);
        assert_panics_with("expected no user-facing error diagnostics", || {
            assert_exact_infrastructure_error(&messages, &ErrorType::File);
        });
    }

    #[test]
    fn assert_exact_infrastructure_error_rejects_missing_outer() {
        let table = StringTable::new();
        let messages = messages_with_errors(vec![], table);
        assert_panics_with("expected one outer infrastructure error", || {
            assert_exact_infrastructure_error(&messages, &ErrorType::File);
        });
    }
}
