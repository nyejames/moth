use crate::compiler_frontend::compiler_errors::{
    BoundaryFailure, CompilerError, InfrastructureErrors,
};
use crate::compiler_frontend::compiler_messages::diagnostic_bag::DiagnosticBag;
use crate::compiler_frontend::compiler_messages::diagnostics::families::names_declarations::UnknownValueNameView;
use crate::compiler_frontend::compiler_messages::diagnostics::freeze::InstallMode;
use crate::compiler_frontend::compiler_messages::diagnostics::legacy_adapter::BoundaryAccumulator;
use crate::compiler_frontend::compiler_messages::prepared_batch::PreparedDiagnosticBatch;
use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, DiagnosticSeverity};
use crate::compiler_frontend::symbols::string_interning::StringTable;

fn failure_without_infrastructure() -> BoundaryFailure {
    BoundaryAccumulator::new().finish(InstallMode::Frozen)
}

fn failure_with_infrastructure() -> BoundaryFailure {
    let mut accumulator = BoundaryAccumulator::new();
    accumulator.append_infrastructure(
        Box::new(InfrastructureErrors::new(CompilerError::compiler_error(
            "boundary infrastructure failure",
        ))),
        None,
    );
    accumulator.finish(InstallMode::Frozen)
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

#[test]
fn prepared_diagnostic_and_infrastructure_counts_are_distinct_lanes() {
    let mut strings = StringTable::new();
    let warning_name = strings.intern("unused_value");
    let error_name = strings.intern("missing_value");
    let mut warning = CompilerDiagnostic::unknown_value_name(warning_name, None);
    warning.severity = DiagnosticSeverity::Warning;
    let error = CompilerDiagnostic::unknown_value_name(error_name, None);
    let prepared = PreparedDiagnosticBatch::capture_from_bag(
        DiagnosticBag::from_diagnostics(vec![warning, error]),
        strings,
        None,
        false,
        None,
    );

    let mut accumulator = BoundaryAccumulator::new();
    accumulator.append_prepared(prepared, None);
    accumulator.append_infrastructure(
        Box::new(InfrastructureErrors::new(CompilerError::compiler_error(
            "backend failure",
        ))),
        None,
    );
    let failure = accumulator.finish(InstallMode::Frozen);

    assert_eq!(failure.error_count(), 2);
    assert_eq!(failure.warning_count(), 1);
    assert!(failure.has_errors());
    assert!(failure.has_warnings());
    assert!(failure.has_infrastructure_error());
    let records = failure
        .reports
        .iter()
        .map(|(report, view)| {
            let facts = view
                .family_view::<UnknownValueNameView>()
                .expect("the frozen report retains the unknown-name family");
            (
                view.severity(),
                report.context().strings().resolve(facts.name).to_owned(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        records,
        [
            (DiagnosticSeverity::Warning, String::from("unused_value")),
            (DiagnosticSeverity::Error, String::from("missing_value")),
        ]
    );
    assert_eq!(
        failure
            .infrastructure_errors()
            .expect("infrastructure cause")
            .iter()
            .map(|cause| cause.msg.as_str())
            .collect::<Vec<_>>(),
        ["backend failure"]
    );
}
