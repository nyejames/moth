//! Typed diagnostic assertion helpers for tests.
//!
//! WHAT: exact diagnostic code, infrastructure error and reason assertions.
//! WHY: a bare `is_err()` or `error_count() > 0` accepts the wrong failure at
//!   multi-lane boundaries. These helpers prove the exact diagnostic kind,
//!   infrastructure error type or authored reason so an unrelated error
//!   cannot satisfy the intended contract.

use crate::compiler_frontend::compiler_errors::{
    BoundaryFailure, CompilerError, CompilerErrorMetadataKey, ErrorType, InfrastructureErrors,
};
use crate::compiler_frontend::compiler_messages::diagnostics::boundary_accumulator::BoundaryAccumulator;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::freeze::InstallMode;
use crate::compiler_frontend::compiler_messages::prepared_batch::PreparedDiagnosticBatch;
use crate::compiler_frontend::compiler_messages::{
    DiagnosticBag, DiagnosticSeverity, PreparedFailure,
};

/// Assert that the diagnostic codes of all error-severity diagnostics match
/// the expected multiset exactly.
///
/// WHAT: collects every error diagnostic's stable code into a sorted multiset
///   and compares against the expected multiset.
/// WHY: `error_count() > 0` or `is_err()` accepts any error. Exact codes prove
///   the intended diagnostic, not just that some error happened.
#[track_caller]
pub fn assert_exact_diagnostic_codes(failure: &BoundaryFailure, expected: &[&str]) {
    let mut actual: Vec<&str> = failure
        .reports
        .iter_severity(DiagnosticSeverity::Error)
        .map(|(_, diagnostic)| diagnostic.descriptor().code)
        .collect();
    actual.sort_unstable();
    let mut expected_sorted: Vec<&str> = expected.to_vec();
    expected_sorted.sort_unstable();
    assert_eq!(
        actual, expected_sorted,
        "diagnostic codes must match exactly"
    );
}

/// Assert that `failure` carries no infrastructure causes.
#[track_caller]
pub fn assert_no_infrastructure_errors(failure: &BoundaryFailure) {
    assert!(
        failure.infrastructure_errors().is_none(),
        "expected no infrastructure errors, found: {:?}",
        failure.infrastructure_errors()
    );
}

/// Assert that `failure` contains no user-facing error diagnostic and exactly one
/// infrastructure cause of the expected `ErrorType`.
///
/// WHAT: verifies the frozen diagnostic stream holds no `Error` diagnostic, the infrastructure
///   lane is present, and its `ErrorType` matches.
/// WHY: `assert_exact_infrastructure_error` should not pass when additional user-facing
///   error diagnostics accompany the expected failure. A missing-file failure should
///   produce exactly one File infrastructure failure, not that plus an unrelated
///   semantic error.
#[track_caller]
pub fn assert_exact_infrastructure_error(failure: &BoundaryFailure, expected_type: &ErrorType) {
    let user_errors: Vec<_> = failure
        .reports
        .iter_severity(DiagnosticSeverity::Error)
        .map(|(_, diagnostic)| diagnostic.descriptor().code)
        .collect();
    assert!(
        user_errors.is_empty(),
        "expected no user-facing error diagnostics, found: {user_errors:?}"
    );

    let Some(errors) = failure.infrastructure_errors() else {
        panic!("expected one outer infrastructure error, found none");
    };
    assert_eq!(
        errors.iter().count(),
        1,
        "expected exactly one infrastructure cause"
    );
    assert_eq!(
        &errors.primary.error_type, expected_type,
        "infrastructure error type mismatch"
    );
}

/// Assert that `failure` carries exactly one infrastructure failure of type `File`
/// with the expected `OutputRejectionReason` metadata value.
///
/// WHAT: extracts the `OutputRejectionReason` metadata from the infrastructure `CompilerError`
///   and compares it against the expected reason string.
/// WHY: all output-writer rejections share `ErrorType::File`. The typed reason
///   seam distinguishes between distinct safety contracts (invalid path,
///   duplicate destination, symlink escape, etc.) so a test for one contract
///   cannot pass when a different contract is violated.
#[track_caller]
pub fn assert_output_rejection(failure: &BoundaryFailure, expected_reason: &str) {
    assert_exact_infrastructure_error(failure, &ErrorType::File);

    let error = &failure
        .infrastructure_errors()
        .expect("exact infrastructure failure must be present")
        .primary;
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

fn boundary_failure(failure: PreparedFailure) -> BoundaryFailure {
    let mut accumulator = BoundaryAccumulator::new();
    match failure {
        PreparedFailure::Diagnosed(batch) => accumulator.append_prepared(batch, None),
        PreparedFailure::Infrastructure(errors) => {
            accumulator.append_infrastructure(errors, None);
        }
        PreparedFailure::Mixed { batch, error } => {
            accumulator.append_prepared(batch, None);
            accumulator.append_infrastructure(error, None);
        }
    }
    accumulator.finish(InstallMode::Frozen)
}

fn diagnostic_failure(
    diagnostics: Vec<DiagnosticDraft>,
    table: crate::compiler_frontend::symbols::string_interning::StringTable,
) -> BoundaryFailure {
    boundary_failure(PreparedFailure::Diagnosed(
        PreparedDiagnosticBatch::capture_from_bag(
            DiagnosticBag::from_diagnostics(diagnostics),
            table,
            None,
            false,
            None,
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler_frontend::compiler_messages::diagnostics::families::names_declarations;
    use crate::compiler_frontend::symbols::string_interning::StringTable;
    use crate::compiler_tests::test_support::assert_panics_with;

    fn failure_with_diagnostics(
        diagnostics: Vec<DiagnosticDraft>,
        table: StringTable,
    ) -> BoundaryFailure {
        diagnostic_failure(diagnostics, table)
    }

    fn infrastructure_failure(error: CompilerError) -> BoundaryFailure {
        boundary_failure(PreparedFailure::Infrastructure(error.into()))
    }

    #[test]
    fn assert_exact_diagnostic_codes_matches_single_error() {
        let mut table = StringTable::new();
        let diagnostic = names_declarations::unknown_value_name(table.intern("missing"), None);
        let failure = failure_with_diagnostics(vec![diagnostic], table);
        assert_exact_diagnostic_codes(&failure, &["MOTH-RULE-0034"]);
    }

    #[test]
    fn assert_exact_diagnostic_codes_rejects_wrong_count() {
        let mut table = StringTable::new();
        let diagnostic = names_declarations::unknown_value_name(table.intern("missing"), None);
        let failure = failure_with_diagnostics(vec![diagnostic], table);
        assert_panics_with("diagnostic codes must match exactly", || {
            assert_exact_diagnostic_codes(&failure, &["MOTH-RULE-0034", "MOTH-RULE-0034"]);
        });
    }

    #[test]
    fn assert_no_infrastructure_errors_accepts_clean_failure() {
        let failure = failure_with_diagnostics(vec![], StringTable::new());
        assert_no_infrastructure_errors(&failure);
    }

    #[test]
    fn assert_no_infrastructure_errors_rejects_infra_error() {
        let failure = infrastructure_failure(CompilerError::compiler_error("test failure"));
        assert_panics_with("expected no infrastructure errors", || {
            assert_no_infrastructure_errors(&failure);
        });
    }

    #[test]
    fn assert_exact_infrastructure_error_matches_type() {
        let error =
            CompilerError::file_error(std::path::Path::new("missing.moth"), "file not found");
        assert_exact_infrastructure_error(&infrastructure_failure(error.clone()), &ErrorType::File);

        let mut errors = InfrastructureErrors::new(error);
        errors.push(CompilerError::compiler_error("later failure"));
        let failure = boundary_failure(PreparedFailure::Infrastructure(Box::new(errors)));
        assert_panics_with("expected exactly one infrastructure cause", || {
            assert_exact_infrastructure_error(&failure, &ErrorType::File);
        });
    }

    #[test]
    fn assert_exact_infrastructure_error_rejects_additional_rule_error() {
        let mut table = StringTable::new();
        let diagnostic = names_declarations::unknown_value_name(table.intern("missing"), None);
        let batch = PreparedDiagnosticBatch::capture_from_bag(
            DiagnosticBag::from(diagnostic),
            table,
            None,
            false,
            None,
        );
        let failure = boundary_failure(PreparedFailure::Mixed {
            batch,
            error: CompilerError::compiler_error("file not found").into(),
        });
        assert_panics_with("expected no user-facing error diagnostics", || {
            assert_exact_infrastructure_error(&failure, &ErrorType::File);
        });
    }

    #[test]
    fn assert_exact_infrastructure_error_rejects_missing_outer() {
        let failure = failure_with_diagnostics(vec![], StringTable::new());
        assert_panics_with("expected one outer infrastructure error", || {
            assert_exact_infrastructure_error(&failure, &ErrorType::File);
        });
    }
}
