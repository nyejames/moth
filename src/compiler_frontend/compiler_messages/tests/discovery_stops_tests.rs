//! Discovery stop metadata aggregation tests.
//!
//! WHAT: checks that prepared batches, kept-first failures and accumulator moves union stop flags.
//! WHY: stop metadata describes shortened discovery after several ownership moves. It must
//!      survive each merge without becoming a diagnostic or growing per-diagnostic state.

use crate::compiler_frontend::compiler_errors::{InfrastructureClass, InfrastructureFailure};
use crate::compiler_frontend::compiler_messages::diagnostics::boundary_accumulator::BoundaryAccumulator;
use crate::compiler_frontend::compiler_messages::diagnostics::families::imports::unused_import;
use crate::compiler_frontend::compiler_messages::diagnostics::families::token_lexical;
use crate::compiler_frontend::compiler_messages::diagnostics::freeze::InstallMode;
use crate::compiler_frontend::compiler_messages::prepared_batch::PreparedDiagnosticBatch;
use crate::compiler_frontend::compiler_messages::{
    CompilerStage, DiagnosticBag, DiagnosticDiscoveryStops, PreparedFailure,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;

fn prepared(diagnostic: DiagnosticBag, stops: DiagnosticDiscoveryStops) -> PreparedDiagnosticBatch {
    let mut batch =
        PreparedDiagnosticBatch::capture_from_bag(diagnostic, StringTable::new(), None, None);
    batch.set_discovery_stops(stops);
    batch
}

#[test]
fn appended_batches_union_their_stop_metadata() {
    let mut accumulator = BoundaryAccumulator::new();
    assert!(accumulator.discovery_stops().is_empty());

    accumulator.append_prepared(
        prepared(
            DiagnosticBag::from(token_lexical::logical_path_capacity()),
            DiagnosticDiscoveryStops::MODULE_ERROR_ALLOWANCE,
        ),
        None,
    );
    accumulator.append_prepared(
        prepared(
            DiagnosticBag::from(unused_import(None)),
            DiagnosticDiscoveryStops::AST_RECOVERY_WORK_GUARD,
        ),
        None,
    );

    let stops = accumulator.discovery_stops();
    assert!(stops.contains(DiagnosticDiscoveryStops::MODULE_ERROR_ALLOWANCE));
    assert!(stops.contains(DiagnosticDiscoveryStops::AST_RECOVERY_WORK_GUARD));
    assert_eq!(
        stops.stop_line().as_deref(),
        Some(
            "Diagnostic discovery stopped early (module error limit and AST recovery work guard): later code was not checked."
        )
    );
}

#[test]
fn keeping_the_first_failure_preserves_later_stop_metadata() {
    let kept = PreparedFailure::from(prepared(
        DiagnosticBag::from(token_lexical::logical_path_capacity()),
        DiagnosticDiscoveryStops::MODULE_ERROR_ALLOWANCE,
    ));
    let later = PreparedFailure::from(prepared(
        DiagnosticBag::from(token_lexical::logical_path_capacity()),
        DiagnosticDiscoveryStops::AST_RECOVERY_WORK_GUARD,
    ));

    let PreparedFailure::Diagnosed(batch) = kept.with_later(later) else {
        panic!("a diagnosed failure must stay diagnosed when a later diagnosis is dropped");
    };
    assert_eq!(batch.diagnostic_count(), 1);
    assert!(
        batch
            .discovery_stops()
            .contains(DiagnosticDiscoveryStops::MODULE_ERROR_ALLOWANCE)
    );
    assert!(
        batch
            .discovery_stops()
            .contains(DiagnosticDiscoveryStops::AST_RECOVERY_WORK_GUARD)
    );
}

#[test]
fn accumulator_replacement_copies_aggregated_stops_back_to_the_caller_owner() {
    let mut warning_owner = BoundaryAccumulator::with_root_string_table(StringTable::new());
    warning_owner.append_prepared(
        prepared(
            DiagnosticBag::from(token_lexical::logical_path_capacity()),
            DiagnosticDiscoveryStops::MODULE_ERROR_ALLOWANCE,
        ),
        None,
    );

    // The finish paths move the accumulator out of the caller-owned owner. The caller must still
    // see the union before the reports are frozen.
    let mut moved = std::mem::replace(&mut warning_owner, BoundaryAccumulator::new());
    moved.append_prepared(
        prepared(
            DiagnosticBag::from(token_lexical::logical_path_capacity()),
            DiagnosticDiscoveryStops::AST_RECOVERY_WORK_GUARD,
        ),
        None,
    );
    warning_owner.merge_discovery_stops(moved.discovery_stops());

    let stops = warning_owner.discovery_stops();
    assert!(stops.contains(DiagnosticDiscoveryStops::MODULE_ERROR_ALLOWANCE));
    assert!(stops.contains(DiagnosticDiscoveryStops::AST_RECOVERY_WORK_GUARD));
}

#[test]
fn infrastructure_and_compiler_bug_lanes_win_over_stop_metadata() {
    let mut accumulator = BoundaryAccumulator::new();
    accumulator.append_prepared(
        prepared(
            DiagnosticBag::from(token_lexical::logical_path_capacity()),
            DiagnosticDiscoveryStops::MODULE_ERROR_ALLOWANCE,
        ),
        None,
    );
    assert!(
        accumulator
            .discovery_stops()
            .contains(DiagnosticDiscoveryStops::MODULE_ERROR_ALLOWANCE)
    );

    // `internal` is the compiler-bug infrastructure class. Stop metadata describes shortened
    // checking, so it never turns an infrastructure or compiler-bug lane into a report set.
    let compiler_bug =
        InfrastructureFailure::internal(CompilerStage::AstConstruction, "stop dominance test");
    assert_eq!(compiler_bug.kind.class(), InfrastructureClass::CompilerBug);
    accumulator.append_infrastructure(Box::new(
        crate::compiler_frontend::compiler_errors::InfrastructureErrors::new(compiler_bug),
    ));
    assert!(
        accumulator.finish(InstallMode::Unfrozen).is_err(),
        "infrastructure dominates a diagnosed batch that carries stop metadata"
    );
}
