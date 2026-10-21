use crate::compiler_frontend::compiler_messages::compiler_errors::{
    BoundaryFailure, CompilerError, InfrastructureErrors,
};
use crate::compiler_frontend::compiler_messages::diagnostics::report::DiagnosticReportSet;

fn failure_without_infrastructure() -> BoundaryFailure {
    BoundaryFailure {
        reports: DiagnosticReportSet::empty(),
        infrastructure: None,
    }
}

fn failure_with_infrastructure() -> BoundaryFailure {
    BoundaryFailure {
        reports: DiagnosticReportSet::empty(),
        infrastructure: Some(Box::new(InfrastructureErrors::new(
            CompilerError::compiler_error("boundary infrastructure failure"),
        ))),
    }
}

#[test]
fn empty_boundary_failure_reports_no_errors_or_warnings() {
    let failure = failure_without_infrastructure();

    assert_eq!(failure.error_count(), 0);
    assert_eq!(failure.warning_count(), 0);
    assert!(!failure.has_errors());
    assert!(!failure.has_warnings());
    assert!(!failure.has_infrastructure_error());
    assert!(failure.infrastructure_errors().is_none());
}

#[test]
fn infrastructure_lane_counts_as_one_error_and_no_warnings() {
    let failure = failure_with_infrastructure();

    assert_eq!(failure.error_count(), 1);
    assert_eq!(failure.warning_count(), 0);
    assert!(failure.has_errors());
    assert!(!failure.has_warnings());
    assert!(failure.has_infrastructure_error());
    assert!(failure.infrastructure_errors().is_some());
}
