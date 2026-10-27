use crate::compiler_frontend::compiler_errors::{
    CompilationOutcome, CompilationResult, InfrastructureErrors, InfrastructureFailure,
};
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::DiagnosticSeverity;
use crate::compiler_frontend::compiler_messages::diagnostic_bag::{DiagnosticBag, PreparedFailure};
use crate::compiler_frontend::compiler_messages::diagnostics::boundary_accumulator::BoundaryAccumulator;
use crate::compiler_frontend::compiler_messages::diagnostics::families::names_declarations;
use crate::compiler_frontend::compiler_messages::diagnostics::freeze::InstallMode;
use crate::compiler_frontend::compiler_messages::prepared_batch::PreparedDiagnosticBatch;
use crate::compiler_frontend::symbols::string_interning::StringTable;

fn infrastructure_failure() -> CompilationResult<()> {
    let mut accumulator = BoundaryAccumulator::new();
    accumulator.append_infrastructure(Box::new(InfrastructureErrors::new(
        InfrastructureFailure::internal(
            CompilerStage::DiagnosticReporting,
            "boundary infrastructure failure",
        ),
    )));
    accumulator
        .finish(InstallMode::Frozen)
        .map(CompilationOutcome::Diagnosed)
}

#[test]
fn empty_success_has_no_diagnostic_or_warning_reports() {
    let result: CompilationResult<()> = Ok(CompilationOutcome::Success {
        value: (),
        warnings: None,
    });
    let Ok(CompilationOutcome::Success { warnings, .. }) = result else {
        panic!("empty compilation should succeed");
    };
    assert!(warnings.is_none());
}

#[test]
fn infrastructure_failure_uses_only_the_outer_error_lane() {
    let Err(errors) = infrastructure_failure() else {
        panic!("infrastructure failure should use the outer error lane");
    };
    assert_eq!(errors.iter().count(), 1);
    assert_eq!(
        errors.primary.message.as_ref(),
        "boundary infrastructure failure"
    );
}

#[test]
fn infrastructure_dominates_prepared_diagnostics_and_warnings() {
    let mut strings = StringTable::new();
    let warning_name = strings.intern("unused_value");
    let error_name = strings.intern("missing_value");
    let warning = names_declarations::unknown_value_name(warning_name, None)
        .with_severity(DiagnosticSeverity::Warning);
    let error = names_declarations::unknown_value_name(error_name, None);
    let prepared = PreparedDiagnosticBatch::capture_from_bag(
        DiagnosticBag::from_diagnostics(vec![warning, error]),
        strings,
        None,
        None,
    );

    let mut accumulator = BoundaryAccumulator::new();
    accumulator.append_prepared(prepared, None);
    accumulator.append_infrastructure(Box::new(InfrastructureErrors::new(
        InfrastructureFailure::internal(CompilerStage::DiagnosticReporting, "backend failure"),
    )));
    let result: CompilationResult<()> = accumulator
        .finish(InstallMode::Frozen)
        .map(CompilationOutcome::Diagnosed);

    let Err(errors) = result else {
        panic!("infrastructure failure must discard diagnostic and warning reports");
    };
    assert_eq!(
        errors
            .iter()
            .map(|cause| &*cause.message)
            .collect::<Vec<_>>(),
        ["backend failure"]
    );
}

fn diagnosed_owner(missing_names: &[&str]) -> PreparedFailure {
    let mut strings = StringTable::new();
    let diagnostics: Vec<_> = missing_names
        .iter()
        .map(|name| names_declarations::unknown_value_name(strings.intern(name), None))
        .collect();
    PreparedFailure::Diagnosed(PreparedDiagnosticBatch::capture_from_bag(
        DiagnosticBag::from_diagnostics(diagnostics),
        strings,
        None,
        None,
    ))
}

#[test]
fn first_diagnosed_owner_is_kept_whole_when_a_later_diagnosis_is_admitted() {
    let owner = diagnosed_owner(&["first_missing", "second_missing"])
        .with_later(diagnosed_owner(&["later_missing"]));
    let PreparedFailure::Diagnosed(batch) = owner else {
        panic!("a later diagnosis must not replace the first diagnosed owner");
    };
    assert_eq!(batch.diagnostic_count(), 2);
}

#[test]
fn infrastructure_admitted_later_dominates_a_diagnosed_owner() {
    let owner = diagnosed_owner(&["missing_value"]).with_later(PreparedFailure::from(
        InfrastructureFailure::internal(CompilerStage::SourceLoading, "later cause"),
    ));
    let PreparedFailure::Infrastructure(errors) = owner else {
        panic!("a later infrastructure cause must dominate the diagnosed owner");
    };
    assert_eq!(errors.iter().count(), 1);
    assert_eq!(errors.primary.message.as_ref(), "later cause");
}

#[test]
fn infrastructure_causes_accumulate_in_admission_order() {
    let owner = diagnosed_owner(&["missing_value"])
        .with_infrastructure_cause(InfrastructureFailure::internal(
            CompilerStage::SourceLoading,
            "first cause",
        ))
        .with_infrastructure_errors(InfrastructureErrors::new(InfrastructureFailure::internal(
            CompilerStage::SourceLoading,
            "second cause",
        )));
    let PreparedFailure::Infrastructure(errors) = owner else {
        panic!("infrastructure causes must replace the diagnosed owner");
    };
    assert_eq!(errors.iter().count(), 2);
    assert_eq!(errors.primary.message.as_ref(), "first cause");
}
