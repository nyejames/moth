//! Typed diagnostic assertion helpers for tests.
//!
//! WHAT: exact diagnostic code, infrastructure error and reason assertions.
//! WHY: a bare `is_err()` or `error_count() > 0` accepts the wrong failure at
//!   multi-lane boundaries. These helpers prove the exact diagnostic kind,
//!   infrastructure error type or authored reason so an unrelated error
//!   cannot satisfy the intended contract.

use crate::compiler_frontend::compiler_errors::{
    CompilationOutcome, CompilationResult, InfrastructureDetail, InfrastructureErrors,
    InfrastructureFailure, InfrastructureFailureKind,
};
use crate::compiler_frontend::compiler_messages::diagnostics::boundary_accumulator::BoundaryAccumulator;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::freeze::InstallMode;
use crate::compiler_frontend::compiler_messages::prepared_batch::PreparedDiagnosticBatch;
use crate::compiler_frontend::compiler_messages::{
    CompilerStage, DiagnosticBag, DiagnosticSeverity, PreparedFailure,
};

/// Assert that the diagnostic codes of all error-severity diagnostics match
/// the expected multiset exactly.
///
/// WHAT: collects every error diagnostic's stable code into a sorted multiset
///   and compares against the expected multiset.
/// WHY: `error_count() > 0` or `is_err()` accepts any error. Exact codes prove
///   the intended diagnostic, not just that some error happened.
#[track_caller]
pub fn assert_exact_diagnostic_codes(result: &CompilationResult<()>, expected: &[&str]) {
    let Ok(CompilationOutcome::Diagnosed(reports)) = result else {
        panic!("expected diagnosed compilation result, found {result:?}");
    };
    let mut actual: Vec<&str> = reports
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

/// Assert that `result` carries no infrastructure causes.
#[track_caller]
pub fn assert_no_infrastructure_errors(result: &CompilationResult<()>) {
    assert!(
        result.is_ok(),
        "expected no infrastructure errors, found: {result:?}"
    );
}

/// Assert that `result` is an infrastructure failure with exactly one cause of the expected kind.
///
/// Infrastructure failures occupy the outer error lane, which cannot retain user diagnostics.
#[track_caller]
pub fn assert_exact_infrastructure_error(
    result: &CompilationResult<()>,
    expected_kind: InfrastructureFailureKind,
) {
    let Err(errors) = result else {
        panic!("expected one outer infrastructure error, found none");
    };
    assert_eq!(
        errors.iter().count(),
        1,
        "expected exactly one infrastructure cause"
    );
    assert_eq!(
        errors.primary.kind, expected_kind,
        "infrastructure failure kind mismatch"
    );
}

/// Assert that `result` carries exactly one output filesystem failure with the expected typed
/// `OutputRejectionReason`.
///
/// WHAT: extracts the typed output rejection fact from the infrastructure `InfrastructureFailure` and
///   compares it against the expected reason string.
/// WHY: the typed reason seam distinguishes between distinct safety contracts (invalid path,
///   duplicate destination, symlink escape, etc.) so a test for one contract cannot pass when a
///   different contract is violated.
#[track_caller]
pub fn assert_output_rejection(result: &CompilationResult<()>, expected_reason: &str) {
    assert_exact_infrastructure_error(result, InfrastructureFailureKind::Filesystem);

    let Err(errors) = result else {
        unreachable!("exact infrastructure failure was asserted above");
    };
    let error = &errors.primary;
    assert_eq!(
        error.stage,
        CompilerStage::Output,
        "output rejection stage mismatch",
    );
    assert_eq!(
        error.details.as_ref(),
        &[InfrastructureDetail::OutputRejectionReason(
            expected_reason.into()
        )],
        "output rejection reason mismatch",
    );
}

fn boundary_result(failure: PreparedFailure) -> CompilationResult<()> {
    match failure {
        PreparedFailure::Diagnosed(batch) => {
            let mut accumulator = BoundaryAccumulator::new();
            accumulator.append_prepared(batch, None);
            accumulator
                .finish(InstallMode::Frozen)
                .map(CompilationOutcome::Diagnosed)
        }
        PreparedFailure::Infrastructure(errors) => Err(*errors),
    }
}

fn diagnostic_failure(
    diagnostics: Vec<DiagnosticDraft>,
    table: crate::compiler_frontend::symbols::string_interning::StringTable,
) -> CompilationResult<()> {
    boundary_result(PreparedFailure::Diagnosed(
        PreparedDiagnosticBatch::capture_from_bag(
            DiagnosticBag::from_diagnostics(diagnostics),
            table,
            None,
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
    ) -> CompilationResult<()> {
        diagnostic_failure(diagnostics, table)
    }

    fn infrastructure_failure(error: InfrastructureFailure) -> CompilationResult<()> {
        boundary_result(PreparedFailure::Infrastructure(error.into()))
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
        let failure = infrastructure_failure(InfrastructureFailure::internal(
            crate::compiler_frontend::compiler_messages::CompilerStage::ProjectBuildAssembly,
            "test failure",
        ));
        assert_panics_with("expected no infrastructure errors", || {
            assert_no_infrastructure_errors(&failure);
        });
    }

    #[test]
    fn assert_exact_infrastructure_error_matches_type() {
        let error = InfrastructureFailure::filesystem(
            crate::compiler_frontend::compiler_messages::CompilerStage::SourceLoading,
            std::path::Path::new("missing.moth"),
            "file not found",
        );
        assert_exact_infrastructure_error(
            &infrastructure_failure(error.clone()),
            InfrastructureFailureKind::Filesystem,
        );

        let mut errors = InfrastructureErrors::new(error);
        errors.push(InfrastructureFailure::internal(
            crate::compiler_frontend::compiler_messages::CompilerStage::ProjectBuildAssembly,
            "later failure",
        ));
        let failure = boundary_result(PreparedFailure::Infrastructure(Box::new(errors)));
        assert_panics_with("expected exactly one infrastructure cause", || {
            assert_exact_infrastructure_error(&failure, InfrastructureFailureKind::Filesystem);
        });
    }

    #[test]
    fn infrastructure_failure_dominates_diagnostics() {
        let mut table = StringTable::new();
        let diagnostic = names_declarations::unknown_value_name(table.intern("missing"), None);
        let batch = PreparedDiagnosticBatch::capture_from_bag(
            DiagnosticBag::from(diagnostic),
            table,
            None,
            None,
        );
        let mut accumulator = BoundaryAccumulator::new();
        accumulator.append_prepared(batch, None);
        accumulator.append_infrastructure(
            InfrastructureFailure::internal(
                crate::compiler_frontend::compiler_messages::CompilerStage::ProjectBuildAssembly,
                "infrastructure failure",
            )
            .into(),
        );
        let failure = accumulator
            .finish(InstallMode::Frozen)
            .map(CompilationOutcome::Diagnosed);
        assert_exact_infrastructure_error(&failure, InfrastructureFailureKind::Internal);
    }

    #[test]
    fn assert_exact_infrastructure_error_rejects_missing_outer() {
        let failure = failure_with_diagnostics(vec![], StringTable::new());
        assert_panics_with("expected one outer infrastructure error", || {
            assert_exact_infrastructure_error(&failure, InfrastructureFailureKind::Filesystem);
        });
    }
}
