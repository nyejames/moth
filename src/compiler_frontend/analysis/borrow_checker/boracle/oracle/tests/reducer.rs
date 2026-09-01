//! Invariants for deterministic disagreement reduction.
//!
//! WHAT: checks that reductions remain dense, class-preserving, static-verdict-preserving,
//!       outcome-identity preserving and reproducible across generated and hand-authored
//!       normalized problems.
//! WHY: a reducer that merely emits smaller malformed input or changes its bounded outcome would
//!      hide the disagreement it claims to preserve.
//!
//! Fixture skeleton assertions verify rendered text shape and row cardinality only. They do not
//! prove that the emitted source compiles: the skeleton is held as a string literal, not compiled.
use super::super::super::reducer::{
    OracleOutcomeIdentity, ReductionPass, reduce_problem, reduction_size, render_fixture_skeleton,
    terminator_after_edge_removal,
};
use super::super::super::{OracleComparisonClass, compare_problem_parts};
use super::super::OracleBounds;
use super::super::OracleOutcome;
use super::super::generator::generated_problem;
use crate::compiler_frontend::analysis::borrow_checker::problem::{
    AccessKind, AggregateField, Binding, BindingId, BlockId, BorrowProblem, BorrowProblemParts,
    Call, CallArgument, CallEffect, CallId, CallResult, CallResultProvenance,
    CallResultUnknownReason, CfgBlock, CfgEdge, Event, EventId, EventKind, EventSource, KillReason,
    Loan, LoanId, OriginKind, Place, PlaceId, PointId, ProgramPoint, ProjectionElem, RebindValue,
    TerminatorEventKind, Use, UseId, UseKind, ValueOrigin, ValueOriginId,
};
use std::collections::BTreeSet;

#[test]
fn boracle_reduction_preserves_disagreement_class_for_generated_and_hand_authored_problems() {
    for (seed, cyclic) in [(0, false), (1, true), (7, false), (42, true)] {
        let generated = generated_problem(seed, cyclic);
        let expected = class_vector(&generated.problem, OracleBounds::default());
        let expected_static = static_accepts_vector(&generated.problem, OracleBounds::default());
        let expected_outcome = oracle_outcome_identity(&generated.problem, OracleBounds::default());
        let reduced = reduce_problem(generated.problem, OracleBounds::default())
            .expect("generated problem should reduce");

        assert_eq!(
            reduced.comparison_classes(),
            expected.as_ref(),
            "reduction changed generated seed {seed} cyclic={cyclic}"
        );
        assert_eq!(
            reduced.static_accepts(),
            expected_static.as_ref(),
            "reduction changed generated static verdicts for seed {seed} cyclic={cyclic}"
        );
        assert_eq!(
            reduced.oracle_outcome(),
            &expected_outcome,
            "reduction changed generated outcome identity for seed {seed} cyclic={cyclic}"
        );
        assert_eq!(
            class_vector(&reduced.problem, reduced.bounds),
            expected,
            "reduced generated problem no longer has its recorded class"
        );
        assert_eq!(
            static_accepts_vector(&reduced.problem, reduced.bounds),
            expected_static,
            "reduced generated problem no longer has its recorded static verdicts"
        );
    }

    let hand_authored = dead_exclusive_alias_problem();
    let expected = class_vector(&hand_authored, OracleBounds::default());
    let expected_static = static_accepts_vector(&hand_authored, OracleBounds::default());
    let expected_outcome = oracle_outcome_identity(&hand_authored, OracleBounds::default());
    assert!(
        expected.contains(&OracleComparisonClass::StaticRejectedBoundedSafe),
        "hand-authored fixture should exercise a non-Agreement class"
    );
    let reduced = reduce_problem(hand_authored, OracleBounds::default())
        .expect("hand-authored problem should reduce");
    assert_eq!(reduced.comparison_classes(), expected.as_ref());
    assert_eq!(reduced.static_accepts(), expected_static.as_ref());
    assert_eq!(reduced.oracle_outcome(), &expected_outcome);
    assert_eq!(class_vector(&reduced.problem, reduced.bounds), expected);
    assert_eq!(
        static_accepts_vector(&reduced.problem, reduced.bounds),
        expected_static
    );

    let runtime_conflict = dead_exclusive_runtime_conflict_problem();
    let expected = class_vector(&runtime_conflict, OracleBounds::default());
    let expected_static = static_accepts_vector(&runtime_conflict, OracleBounds::default());
    let expected_outcome = oracle_outcome_identity(&runtime_conflict, OracleBounds::default());
    assert_eq!(
        expected.as_ref(),
        &[
            OracleComparisonClass::Agreement,
            OracleComparisonClass::StaticAcceptedRuntimeConflict,
        ],
        "runtime-conflict fixture should exercise the reference and experiment classes"
    );
    assert_eq!(
        expected_static.as_ref(),
        &[false, true],
        "runtime-conflict fixture should distinguish static acceptance"
    );
    assert_eq!(
        expected_outcome,
        OracleOutcomeIdentity::RuntimeConflict,
        "runtime-conflict fixture should preserve a runtime-conflict outcome"
    );
    let reduced = reduce_problem(runtime_conflict, OracleBounds::default())
        .expect("runtime-conflict problem should reduce");
    assert_eq!(reduced.comparison_classes(), expected.as_ref());
    assert_eq!(reduced.static_accepts(), expected_static.as_ref());
    assert_eq!(reduced.oracle_outcome(), &expected_outcome);
    assert_eq!(class_vector(&reduced.problem, reduced.bounds), expected);
    assert_eq!(
        static_accepts_vector(&reduced.problem, reduced.bounds),
        expected_static
    );
}

#[test]
fn boracle_reducer_rejects_zero_bound_components() {
    let cases = [
        ("max_executions", OracleBounds::new(0, 4096, 8, 4096)),
        ("max_executed_events", OracleBounds::new(256, 0, 8, 4096)),
        ("max_block_entries", OracleBounds::new(256, 4096, 0, 4096)),
        (
            "max_dynamic_generations",
            OracleBounds::new(256, 4096, 8, 0),
        ),
    ];
    for (name, bounds) in cases {
        let result = reduce_problem(dead_exclusive_alias_problem(), bounds);
        let error = match result {
            Err(error) => error,
            Ok(_) => panic!("zero bound {name} unexpectedly returned Ok"),
        };
        assert_eq!(
            error.msg,
            format!("Boracle reducer requires {name} to be greater than zero")
        );
    }
}

#[test]
fn boracle_reducer_rejects_candidate_that_changes_static_verdict() {
    let bounds = OracleBounds::new(256, 1, 8, 4096);
    let problem = static_verdict_probe_problem(true);
    let expected_classes = class_vector(&problem, bounds);
    assert!(
        expected_classes
            .iter()
            .all(|class| *class == OracleComparisonClass::OracleInconclusive),
        "static-verdict probe must be inconclusive for every comparison: {expected_classes:?}"
    );
    let expected_static = static_accepts_vector(&problem, bounds);
    assert!(
        expected_static.iter().any(|accepts| !accepts),
        "static-verdict probe must include a rejected static report: {expected_static:?}"
    );
    let reduced = reduce_problem(problem, bounds).expect("static-verdict probe should reduce");
    assert_eq!(
        reduced.comparison_classes(),
        expected_classes.as_ref(),
        "reduction changed the static-verdict probe class vector"
    );
    assert_eq!(
        reduced.static_accepts(),
        expected_static.as_ref(),
        "reduction accepted a candidate that changed the static verdict vector"
    );
    assert_eq!(
        reduced.problem.loans().len(),
        1,
        "static-verdict probe loan was removed despite changing static verdicts"
    );
}

#[test]
fn boracle_reducer_preserves_nonzero_bounds_and_limit_reason_for_cyclic_seed() {
    const SEED: u32 = 42;
    let generated = generated_problem(SEED, true);
    let initial_set = super::super::super::compare_reference_and_experiments(
        generated.problem.clone(),
        OracleBounds::default(),
    )
    .expect("cyclic seed should compare");
    let initial_reason = match initial_set.oracle_outcome.as_ref() {
        Some(OracleOutcome::Inconclusive { reason, .. }) => reason.clone(),
        outcome => panic!("cyclic seed should be inconclusive, got {outcome:?}"),
    };

    let reduced = reduce_problem(generated.problem, OracleBounds::default())
        .expect("cyclic seed should reduce");
    let bounds = reduced.bounds;
    for (name, value) in [
        ("max_executions", bounds.max_executions),
        ("max_executed_events", bounds.max_executed_events),
        ("max_block_entries", bounds.max_block_entries),
        ("max_dynamic_generations", bounds.max_dynamic_generations),
    ] {
        assert!(value > 0, "reduced bound {name} was zero: {bounds:?}");
    }

    let reduced_set =
        super::super::super::compare_reference_and_experiments(reduced.problem.clone(), bounds)
            .expect("reduced cyclic seed should compare");
    match reduced_set.oracle_outcome.as_ref() {
        Some(OracleOutcome::Inconclusive { reason, .. }) => {
            assert_eq!(
                reason, &initial_reason,
                "reduced cyclic seed changed its limit reason"
            );
        }
        outcome => panic!("reduced cyclic seed should remain inconclusive, got {outcome:?}"),
    }
    assert_eq!(
        reduced.oracle_outcome(),
        &OracleOutcomeIdentity::Inconclusive {
            reason: initial_reason,
        }
    );
}

#[test]
fn boracle_reducer_can_make_progress_in_every_ordered_pass() {
    let mut fired = BTreeSet::new();
    let inputs = [
        (0, false),
        (1, false),
        (2, true),
        (3, true),
        (7, false),
        (42, true),
    ];

    for (seed, cyclic) in inputs {
        let reduced = reduce_problem(
            generated_problem(seed, cyclic).problem,
            OracleBounds::default(),
        )
        .expect("generated problem should reduce");
        fired.extend(reduced.applied_passes().iter().copied());
    }

    let hand_authored = reduce_problem(dead_exclusive_alias_problem(), OracleBounds::default())
        .expect("hand-authored problem should reduce");
    fired.extend(hand_authored.applied_passes().iter().copied());

    let projected = reduce_problem(problem_with_unused_projection(), OracleBounds::default())
        .expect("projected hand-authored problem should reduce");
    fired.extend(projected.applied_passes().iter().copied());

    for pass in ReductionPass::ALL {
        assert!(
            fired.contains(&pass),
            "reduction pass {pass:?} made no progress on the bounded inputs"
        );
    }
}
#[test]
fn boracle_reducer_uses_the_plan_pass_order() {
    let expected = [
        ReductionPass::RemoveUnreachableBlocks,
        ReductionPass::RemoveEvents,
        ReductionPass::RemoveUsesAndLoans,
        ReductionPass::RemoveEdges,
        ReductionPass::SimplifyProjections,
        ReductionPass::ReduceOrigins,
        ReductionPass::ReduceBindings,
        ReductionPass::ReplaceCallsWithSimplerEffects,
        ReductionPass::LowerLoopBounds,
    ];

    assert_eq!(
        ReductionPass::ALL,
        expected,
        "reduction pass order diverged from the plan"
    );
}

#[test]
fn boracle_reducer_termination_measure_strictly_decreases() {
    let generated = generated_problem(7, true);
    let initial_size = reduction_size(&generated.problem, OracleBounds::default());
    let reduced = reduce_problem(generated.problem, OracleBounds::default())
        .expect("generated problem should reduce");
    let history = reduced.size_history();

    assert_eq!(history.first(), Some(&initial_size));
    assert_eq!(history.last(), Some(&reduced.size_measure()));
    assert_eq!(
        reduced.size_measure(),
        reduction_size(&reduced.problem, reduced.bounds),
        "recorded measure does not match the reduced problem and bounds"
    );
    for pair in history.windows(2) {
        assert!(
            pair[1] < pair[0],
            "termination measure did not decrease: before={:?} after={:?}",
            pair[0],
            pair[1]
        );
    }
}

#[test]
fn boracle_reducer_is_byte_identical_for_repeated_runs() {
    let first = reduce_problem(generated_problem(42, true).problem, OracleBounds::default())
        .expect("first reduction should succeed");
    let second = reduce_problem(generated_problem(42, true).problem, OracleBounds::default())
        .expect("second reduction should succeed");

    assert_eq!(
        first, second,
        "same normalized input was reduced differently"
    );
    assert_eq!(
        first.fixture_skeleton().as_bytes(),
        second.fixture_skeleton().as_bytes(),
        "same normalized input produced a different fixture skeleton"
    );
}

#[test]
fn boracle_reducer_rejects_candidate_that_changes_class() {
    let problem = dead_exclusive_alias_problem();
    let expected = class_vector(&problem, OracleBounds::default());
    let mut candidate = problem_parts(&problem);
    candidate.events.remove(1);
    for (index, event) in candidate.events.iter_mut().enumerate() {
        event.id = EventId::new(index as u32);
    }
    for block in &mut candidate.blocks {
        block.events = block
            .events
            .iter()
            .filter_map(|event_id| (event_id.raw() != 1).then_some(event_id.raw()))
            .map(|raw| EventId::new(if raw > 1 { raw - 1 } else { raw }))
            .collect();
    }

    let candidate_set = compare_problem_parts(candidate, OracleBounds::default())
        .expect("candidate should be classified");
    let candidate_classes = candidate_set
        .comparisons()
        .map(|comparison| comparison.class)
        .collect::<Vec<_>>();
    assert_ne!(
        candidate_classes.as_slice(),
        expected.as_ref(),
        "the deliberately class-changing candidate unexpectedly preserved its class"
    );

    let reduced =
        reduce_problem(problem, OracleBounds::default()).expect("original problem should reduce");
    assert_eq!(
        reduced.comparison_classes(),
        expected.as_ref(),
        "reducer accepted a candidate whose class changed"
    );
    let runtime_problem = dead_exclusive_runtime_conflict_problem();
    let runtime_expected = class_vector(&runtime_problem, OracleBounds::default());
    assert!(
        runtime_expected.contains(&OracleComparisonClass::StaticAcceptedRuntimeConflict),
        "runtime-conflict fixture should exercise the soundness class"
    );
    let mut runtime_candidate = problem_parts(&runtime_problem);
    runtime_candidate.events.remove(1);
    for (index, event) in runtime_candidate.events.iter_mut().enumerate() {
        event.id = EventId::new(index as u32);
    }
    for block in &mut runtime_candidate.blocks {
        block.events = block
            .events
            .iter()
            .filter_map(|event_id| (event_id.raw() != 1).then_some(event_id.raw()))
            .map(|raw| EventId::new(if raw > 1 { raw - 1 } else { raw }))
            .collect();
    }

    let runtime_candidate_set = compare_problem_parts(runtime_candidate, OracleBounds::default())
        .expect("runtime-conflict candidate should be classified");
    let runtime_candidate_classes = runtime_candidate_set
        .comparisons()
        .map(|comparison| comparison.class)
        .collect::<Vec<_>>();
    assert_ne!(
        runtime_candidate_classes.as_slice(),
        runtime_expected.as_ref(),
        "the runtime-conflict class-changing candidate unexpectedly preserved its class"
    );

    let runtime_reduced = reduce_problem(runtime_problem, OracleBounds::default())
        .expect("runtime-conflict problem should reduce");
    assert_eq!(
        runtime_reduced.comparison_classes(),
        runtime_expected.as_ref(),
        "reducer accepted a runtime-conflict candidate whose class changed"
    );
}

#[test]
fn boracle_reduced_fixture_skeleton_preserves_shape_and_cardinality() {
    let reduced = reduce_problem(dead_exclusive_alias_problem(), OracleBounds::default())
        .expect("hand-authored problem should reduce");
    let skeleton = reduced.fixture_skeleton();

    assert!(
        skeleton.starts_with("fn reduced_boracle_problem() -> (BorrowProblem, OracleBounds) {")
    );
    assert!(skeleton.contains("BorrowProblem::new(BorrowProblemParts {"));
    assert!(skeleton.contains(
        "// Inspection-only HIR locals, regions, and binding/point/event source provenance are omitted."
    ));
    let expected_bounds = format!(
        "let bounds = OracleBounds::new({}, {}, {}, {});",
        reduced.bounds.max_executions,
        reduced.bounds.max_executed_events,
        reduced.bounds.max_block_entries,
        reduced.bounds.max_dynamic_generations,
    );
    assert!(
        skeleton.contains(&expected_bounds),
        "fixture skeleton omitted the reduced bounds: {expected_bounds}"
    );

    for (row_marker, expected_count) in [
        ("Binding::", reduced.problem.bindings().len()),
        ("ProgramPoint::new(", reduced.problem.points().len()),
        (
            "CfgBlock::new(",
            reduced.problem.control_flow().blocks.len(),
        ),
        ("CfgEdge::new(", reduced.problem.control_flow().edges.len()),
        ("Place::new(", reduced.problem.places().len()),
        ("ValueOrigin::new(", reduced.problem.origins().len()),
        ("Loan {", reduced.problem.loans().len()),
        ("Use {", reduced.problem.uses().len()),
        ("Call {", reduced.problem.calls().len()),
        ("Event::new(", reduced.problem.events().len()),
    ] {
        assert_eq!(
            skeleton.matches(row_marker).count(),
            expected_count,
            "fixture skeleton row count for {row_marker:?} was wrong"
        );
    }

    let known_fresh_row = reduced
        .problem
        .events()
        .iter()
        .find_map(|event| {
            let EventKind::Fresh {
                destination,
                origin,
            } = &event.kind
            else {
                return None;
            };
            Some(format!(
                "Event::new(EventId::new({}), PointId::new({}), EventKind::Fresh {{ destination: PlaceId::new({}), origin: ValueOriginId::new({}) }}, EventSource::none()),",
                event.id.raw(),
                event.point.raw(),
                destination.raw(),
                origin.raw(),
            ))
        })
        .expect("runtime conflict reduction should retain its fresh event");
    assert!(
        skeleton.contains(&known_fresh_row),
        "fixture skeleton did not render a reduced event verbatim: {known_fresh_row}"
    );
}

#[test]
fn boracle_reducer_sorts_targets_after_three_way_edge_removal() {
    let problem = three_way_branch_with_out_of_order_edges();
    let EventKind::Terminator { kind } = &problem.events()[0].kind else {
        panic!("three-way fixture must begin with a branch terminator");
    };
    let removed_edge = problem.control_flow().edges[0];
    let outgoing_targets = problem
        .control_flow()
        .edges
        .iter()
        .filter(|edge| **edge != removed_edge)
        .map(|edge| edge.to)
        .collect::<Vec<_>>();
    let Some(TerminatorEventKind::Branch { targets }) =
        terminator_after_edge_removal(kind, &outgoing_targets)
    else {
        panic!("three-way branch should remain a branch after one edge removal");
    };
    assert_eq!(
        targets.as_ref(),
        [BlockId::new(2), BlockId::new(3)],
        "retained branch targets must be sorted after edge removal"
    );
}

#[test]
fn boracle_fixture_skeleton_renders_every_normalized_variant() {
    let problem = renderer_coverage_problem();
    let skeleton = render_fixture_skeleton(&problem, OracleBounds::default());

    for expected in [
        "CfgEdge::new(BlockId::new(0), BlockId::new(8))",
        "Loan { id: LoanId::new(0), kind: AccessKind::Shared, issued_at: PointId::new(18), place: PlaceId::new(0), origins: vec![ValueOriginId::new(0)].into_boxed_slice(), holders: vec![PlaceId::new(0)].into_boxed_slice(), uses: vec![].into_boxed_slice(), kills: vec![PointId::new(19)].into_boxed_slice() }",
        "Call { id: CallId::new(0), label: \"renderer-call\".to_string() }",
        "Binding::new(BindingId::new(8), None, None, true, false, EventSource::none())",
        "Call { id: CallId::new(1), label: \"renderer-alias-call\".to_string() }",
        "Call { id: CallId::new(2), label: \"renderer-alias-params-call\".to_string() }",
        "Call { id: CallId::new(3), label: \"renderer-summary-unknown-call\".to_string() }",
        "Call { id: CallId::new(4), label: \"renderer-missing-summary-call\".to_string() }",
        "Call { id: CallId::new(5), label: \"renderer-opaque-external-call\".to_string() }",
        "Call { id: CallId::new(6), label: \"renderer-no-result-call\".to_string() }",
        "Event::new(EventId::new(0), PointId::new(1), EventKind::Fresh { destination: PlaceId::new(0), origin: ValueOriginId::new(0) }, EventSource::none()),",
        "Event::new(EventId::new(1), PointId::new(2), EventKind::Alias { source: PlaceId::new(0), destination: PlaceId::new(1), origins: vec![ValueOriginId::new(0)].into_boxed_slice() }, EventSource::none()),",
        "Event::new(EventId::new(2), PointId::new(3), EventKind::AliasFromPlace { source: PlaceId::new(0), destination: PlaceId::new(2) }, EventSource::none()),",
        "Event::new(EventId::new(3), PointId::new(4), EventKind::ExclusiveAlias { source: PlaceId::new(0), destination: PlaceId::new(3), origins: vec![ValueOriginId::new(0)].into_boxed_slice() }, EventSource::none()),",
        "Event::new(EventId::new(4), PointId::new(5), EventKind::ExclusiveAliasFromPlace { source: PlaceId::new(0), destination: PlaceId::new(4) }, EventSource::none()),",
        "Event::new(EventId::new(5), PointId::new(6), EventKind::Copy { source: PlaceId::new(0), destination: PlaceId::new(5), origin: ValueOriginId::new(5) }, EventSource::none()),",
        "Event::new(EventId::new(6), PointId::new(7), EventKind::Projection { source: PlaceId::new(0), destination: PlaceId::new(6), origin: ValueOriginId::new(6) }, EventSource::none()),",
        "Event::new(EventId::new(11), PointId::new(12), EventKind::ScopeExit { bindings: vec![BindingId::new(0)].into_boxed_slice() }, EventSource::none()),",
        "Event::new(EventId::new(12), PointId::new(13), EventKind::ReactiveObserve { place: PlaceId::new(0) }, EventSource::none()),",
        "Event::new(EventId::new(15), PointId::new(16), EventKind::Access { use_id: UseId::new(1) }, EventSource::none()),",
        "Event::new(EventId::new(17), PointId::new(18), EventKind::LoanIssue { loan: LoanId::new(0) }, EventSource::none()),",
        "Event::new(EventId::new(42), PointId::new(43), EventKind::Terminator { kind: TerminatorEventKind::Return }, EventSource::none()),",
        "EventKind::CallArgument { call: CallId::new(0), index: 0, argument: CallArgument { place: PlaceId::new(0), access: AccessKind::Shared, use_id: UseId::new(0) } }",
        "EventKind::CallEffect(CallEffect { call: CallId::new(0), arguments: vec![CallArgument { place: PlaceId::new(0), access: AccessKind::Shared, use_id: UseId::new(0) }].into_boxed_slice(), result: Some(CallResult { place: PlaceId::new(8), origin: ValueOriginId::new(8) }) })",
        "Event::new(EventId::new(27), PointId::new(28), EventKind::CallEffect(CallEffect { call: CallId::new(1), arguments: vec![].into_boxed_slice(), result: Some(CallResult { place: PlaceId::new(9), origin: ValueOriginId::new(9) }) }), EventSource::none()),",
        "Event::new(EventId::new(28), PointId::new(29), EventKind::CallArgument { call: CallId::new(2), index: 0, argument: CallArgument { place: PlaceId::new(0), access: AccessKind::Exclusive, use_id: UseId::new(3) } }, EventSource::none()),",
        "Event::new(EventId::new(29), PointId::new(30), EventKind::CallEffect(CallEffect { call: CallId::new(2), arguments: vec![CallArgument { place: PlaceId::new(0), access: AccessKind::Exclusive, use_id: UseId::new(3) }].into_boxed_slice(), result: Some(CallResult { place: PlaceId::new(10), origin: ValueOriginId::new(10) }) }), EventSource::none()),",
        "Event::new(EventId::new(30), PointId::new(31), EventKind::CallEffect(CallEffect { call: CallId::new(3), arguments: vec![].into_boxed_slice(), result: Some(CallResult { place: PlaceId::new(11), origin: ValueOriginId::new(11) }) }), EventSource::none()),",
        "Event::new(EventId::new(31), PointId::new(32), EventKind::CallEffect(CallEffect { call: CallId::new(4), arguments: vec![].into_boxed_slice(), result: Some(CallResult { place: PlaceId::new(12), origin: ValueOriginId::new(12) }) }), EventSource::none()),",
        "Event::new(EventId::new(32), PointId::new(33), EventKind::CallEffect(CallEffect { call: CallId::new(5), arguments: vec![].into_boxed_slice(), result: Some(CallResult { place: PlaceId::new(13), origin: ValueOriginId::new(13) }) }), EventSource::none()),",
        "Event::new(EventId::new(33), PointId::new(34), EventKind::CallEffect(CallEffect { call: CallId::new(6), arguments: vec![].into_boxed_slice(), result: None }), EventSource::none()),",
        "EventKind::Aggregate { destination: PlaceId::new(7), origin: ValueOriginId::new(0), fields: vec![AggregateField { projection: ProjectionElem::Field(0), source: PlaceId::new(0) }, AggregateField { projection: ProjectionElem::FixedIndex(1), source: PlaceId::new(0) }, AggregateField { projection: ProjectionElem::DynamicIndex, source: PlaceId::new(0) }, AggregateField { projection: ProjectionElem::CollectionElement, source: PlaceId::new(0) }, AggregateField { projection: ProjectionElem::MapEntry, source: PlaceId::new(0) }].into_boxed_slice() }",
        "EventKind::Rebind { destination: PlaceId::new(0), value: RebindValue::Fresh(ValueOriginId::new(1)) }",
        "EventKind::Rebind { destination: PlaceId::new(0), value: RebindValue::Alias(vec![ValueOriginId::new(0)].into_boxed_slice()) }",
        "EventKind::Rebind { destination: PlaceId::new(0), value: RebindValue::AliasFromPlace(PlaceId::new(1)) }",
        "ValueOrigin::new(ValueOriginId::new(1), OriginKind::Unknown)",
        "ValueOrigin::new(ValueOriginId::new(2), OriginKind::Parameter { index: 2 })",
        "ValueOrigin::new(ValueOriginId::new(3), OriginKind::Alias(vec![ValueOriginId::new(0)].into_boxed_slice()))",
        "ValueOrigin::new(ValueOriginId::new(4), OriginKind::ExclusiveAlias(vec![ValueOriginId::new(0)].into_boxed_slice()))",
        "ValueOrigin::new(ValueOriginId::new(5), OriginKind::Copy(vec![ValueOriginId::new(0)].into_boxed_slice()))",
        "ValueOrigin::new(ValueOriginId::new(6), OriginKind::Projection { source: ValueOriginId::new(0), projection: ProjectionElem::MapEntry })",
        "ValueOrigin::new(ValueOriginId::new(7), OriginKind::Join(vec![ValueOriginId::new(0)].into_boxed_slice()))",
        "ValueOrigin::new(ValueOriginId::new(8), OriginKind::CallResult { call: CallId::new(0), provenance: CallResultProvenance::Fresh })",
        "ValueOrigin::new(ValueOriginId::new(9), OriginKind::CallResult { call: CallId::new(1), provenance: CallResultProvenance::Alias(vec![ValueOriginId::new(0)].into_boxed_slice()) })",
        "ValueOrigin::new(ValueOriginId::new(10), OriginKind::CallResult { call: CallId::new(2), provenance: CallResultProvenance::AliasParams(vec![0].into_boxed_slice()) })",
        "ValueOrigin::new(ValueOriginId::new(11), OriginKind::CallResult { call: CallId::new(3), provenance: CallResultProvenance::Unknown(CallResultUnknownReason::SummaryUnknown) })",
        "ValueOrigin::new(ValueOriginId::new(12), OriginKind::CallResult { call: CallId::new(4), provenance: CallResultProvenance::Unknown(CallResultUnknownReason::MissingSummary) })",
        "ValueOrigin::new(ValueOriginId::new(13), OriginKind::CallResult { call: CallId::new(5), provenance: CallResultProvenance::Unknown(CallResultUnknownReason::OpaqueExternal) })",
        "Place::new(PlaceId::new(2), BindingId::new(2), vec![ProjectionElem::Field(0)])",
        "Place::new(PlaceId::new(3), BindingId::new(3), vec![ProjectionElem::FixedIndex(1)])",
        "Place::new(PlaceId::new(4), BindingId::new(4), vec![ProjectionElem::DynamicIndex])",
        "Place::new(PlaceId::new(5), BindingId::new(5), vec![ProjectionElem::CollectionElement])",
        "Place::new(PlaceId::new(6), BindingId::new(6), vec![ProjectionElem::MapEntry])",
        "EventKind::LoanKill { loan: LoanId::new(0), reason: KillReason::FinalUse }",
        "EventKind::LoanKill { loan: LoanId::new(1), reason: KillReason::Rebind }",
        "EventKind::LoanKill { loan: LoanId::new(2), reason: KillReason::ScopeExit }",
        "EventKind::LoanKill { loan: LoanId::new(3), reason: KillReason::UnreachableContinuation }",
        "EventKind::LoanKill { loan: LoanId::new(4), reason: KillReason::Explicit }",
        "EventKind::Terminator { kind: TerminatorEventKind::Jump { target: BlockId::new(8) } }",
        "EventKind::Terminator { kind: TerminatorEventKind::Branch { targets: vec![BlockId::new(1), BlockId::new(2), BlockId::new(3), BlockId::new(4), BlockId::new(5), BlockId::new(6), BlockId::new(7), BlockId::new(8)].into_boxed_slice() } }",
        "EventKind::Terminator { kind: TerminatorEventKind::ReturnSuccess }",
        "EventKind::Terminator { kind: TerminatorEventKind::ReturnError }",
        "EventKind::Terminator { kind: TerminatorEventKind::Break { target: BlockId::new(8) } }",
        "EventKind::Terminator { kind: TerminatorEventKind::Continue { target: BlockId::new(8) } }",
        "EventKind::Terminator { kind: TerminatorEventKind::RuntimeFailure }",
        "EventKind::Terminator { kind: TerminatorEventKind::AssertFailure }",
    ] {
        assert!(
            skeleton.contains(expected),
            "fixture skeleton omitted renderer output: {expected}"
        );
    }
}

fn oracle_outcome_identity(problem: &BorrowProblem, bounds: OracleBounds) -> OracleOutcomeIdentity {
    let set = super::super::super::compare_reference_and_experiments(problem.clone(), bounds)
        .expect("problem should compare");
    let outcome = set
        .oracle_outcome
        .as_ref()
        .expect("valid problem comparison should have an oracle outcome");
    OracleOutcomeIdentity::from_outcome(outcome)
}

fn class_vector(problem: &BorrowProblem, bounds: OracleBounds) -> Box<[OracleComparisonClass]> {
    let set = super::super::super::compare_reference_and_experiments(problem.clone(), bounds)
        .expect("problem should compare");
    set.comparisons()
        .map(|comparison| comparison.class)
        .collect::<Vec<_>>()
        .into_boxed_slice()
}

fn static_accepts_vector(problem: &BorrowProblem, bounds: OracleBounds) -> Box<[bool]> {
    let set = super::super::super::compare_reference_and_experiments(problem.clone(), bounds)
        .expect("problem should compare");
    set.comparisons()
        .map(|comparison| {
            !comparison
                .static_report
                .as_ref()
                .expect("valid problem comparison should have a static report")
                .has_conflicts()
        })
        .collect::<Vec<_>>()
        .into_boxed_slice()
}

fn dead_exclusive_alias_problem() -> BorrowProblem {
    // WHY: This unused exclusive alias is a stable non-Agreement fixture. Its static conservative
    // loan remains live for the owner write while the bounded oracle sees no runtime conflict.
    let event_kinds = vec![
        EventKind::Fresh {
            destination: PlaceId::new(0),
            origin: ValueOriginId::new(0),
        },
        EventKind::ExclusiveAliasFromPlace {
            source: PlaceId::new(0),
            destination: PlaceId::new(1),
        },
        EventKind::Access {
            use_id: UseId::new(0),
        },
    ];
    let exit = event_kinds.len() as u32 + 1;
    let mut events = event_kinds
        .into_iter()
        .enumerate()
        .map(|(index, kind)| {
            Event::new(
                EventId::new(index as u32),
                PointId::new(index as u32 + 1),
                kind,
                EventSource::none(),
            )
        })
        .collect::<Vec<_>>();
    events.push(Event::new(
        EventId::new(events.len() as u32),
        PointId::new(exit),
        EventKind::Terminator {
            kind: TerminatorEventKind::Return,
        },
        EventSource::none(),
    ));
    BorrowProblem::new(BorrowProblemParts {
        bindings: vec![
            Binding::synthetic(BindingId::new(0)),
            Binding::synthetic(BindingId::new(1)),
        ],
        points: (0..=exit)
            .map(|ordinal| ProgramPoint::new(PointId::new(ordinal), BlockId::new(0), ordinal))
            .collect(),
        blocks: vec![CfgBlock::new(
            BlockId::new(0),
            PointId::new(0),
            PointId::new(exit),
            (0..events.len())
                .map(|id| EventId::new(id as u32))
                .collect(),
        )],
        entry: BlockId::new(0),
        exits: vec![BlockId::new(0)],
        places: vec![
            Place::new(PlaceId::new(0), BindingId::new(0), Vec::new()),
            Place::new(PlaceId::new(1), BindingId::new(1), Vec::new()),
        ],
        origins: vec![ValueOrigin::fresh(ValueOriginId::new(0))],
        uses: vec![Use {
            id: UseId::new(0),
            point: PointId::new(3),
            place: PlaceId::new(0),
            kind: UseKind::Write,
            definition: false,
        }],
        events,
        ..BorrowProblemParts::default()
    })
    .expect("dead exclusive alias should validate")
}

fn dead_exclusive_runtime_conflict_problem() -> BorrowProblem {
    // WHY: The explicit loan advertises no uses while both holder reads execute. The conservative
    // reference retains the loan and conflicts at the owner write while use-driven liveness drops it.
    let event_kinds = vec![
        EventKind::Fresh {
            destination: PlaceId::new(0),
            origin: ValueOriginId::new(0),
        },
        EventKind::LoanIssue {
            loan: LoanId::new(0),
        },
        EventKind::Access {
            use_id: UseId::new(0),
        },
        EventKind::Access {
            use_id: UseId::new(1),
        },
        EventKind::Access {
            use_id: UseId::new(2),
        },
    ];
    let exit = event_kinds.len() as u32 + 1;
    let mut events = event_kinds
        .into_iter()
        .enumerate()
        .map(|(index, kind)| {
            Event::new(
                EventId::new(index as u32),
                PointId::new(index as u32 + 1),
                kind,
                EventSource::none(),
            )
        })
        .collect::<Vec<_>>();
    events.push(Event::new(
        EventId::new(events.len() as u32),
        PointId::new(exit),
        EventKind::Terminator {
            kind: TerminatorEventKind::Return,
        },
        EventSource::none(),
    ));
    BorrowProblem::new(BorrowProblemParts {
        bindings: vec![Binding::synthetic(BindingId::new(0))],
        points: (0..=exit)
            .map(|ordinal| ProgramPoint::new(PointId::new(ordinal), BlockId::new(0), ordinal))
            .collect(),
        blocks: vec![CfgBlock::new(
            BlockId::new(0),
            PointId::new(0),
            PointId::new(exit),
            (0..events.len())
                .map(|id| EventId::new(id as u32))
                .collect(),
        )],
        entry: BlockId::new(0),
        exits: vec![BlockId::new(0)],
        places: vec![
            Place::new(PlaceId::new(0), BindingId::new(0), Vec::new()),
            Place::new(
                PlaceId::new(1),
                BindingId::new(0),
                vec![ProjectionElem::Field(0)],
            ),
        ],
        origins: vec![ValueOrigin::fresh(ValueOriginId::new(0))],
        loans: vec![Loan {
            id: LoanId::new(0),
            kind: AccessKind::Exclusive,
            issued_at: PointId::new(2),
            place: PlaceId::new(0),
            origins: vec![ValueOriginId::new(0)].into_boxed_slice(),
            holders: vec![PlaceId::new(1)].into_boxed_slice(),
            uses: Vec::new().into_boxed_slice(),
            kills: Vec::new().into_boxed_slice(),
        }],
        uses: vec![
            Use {
                id: UseId::new(0),
                point: PointId::new(3),
                place: PlaceId::new(1),
                kind: UseKind::Read,
                definition: false,
            },
            Use {
                id: UseId::new(1),
                point: PointId::new(4),
                place: PlaceId::new(0),
                kind: UseKind::Write,
                definition: false,
            },
            Use {
                id: UseId::new(2),
                point: PointId::new(5),
                place: PlaceId::new(1),
                kind: UseKind::Read,
                definition: false,
            },
        ],
        events,
        ..BorrowProblemParts::default()
    })
    .expect("dead exclusive runtime conflict should validate")
}

fn static_verdict_probe_problem(with_loan: bool) -> BorrowProblem {
    let mut events = Vec::new();
    renderer_event(
        &mut events,
        EventKind::Fresh {
            destination: PlaceId::new(0),
            origin: ValueOriginId::new(0),
        },
    );
    let issued_at = with_loan.then(|| {
        renderer_event(
            &mut events,
            EventKind::LoanIssue {
                loan: LoanId::new(0),
            },
        )
    });
    let conflict_use_point = renderer_event(
        &mut events,
        EventKind::Access {
            use_id: UseId::new(1),
        },
    );
    let keeping_use_point = renderer_event(
        &mut events,
        EventKind::Access {
            use_id: UseId::new(0),
        },
    );
    let killed_at = with_loan.then(|| {
        renderer_event(
            &mut events,
            EventKind::LoanKill {
                loan: LoanId::new(0),
                reason: KillReason::Explicit,
            },
        )
    });
    let end = renderer_event(
        &mut events,
        EventKind::Terminator {
            kind: TerminatorEventKind::Return,
        },
    );
    let loans = match (issued_at, killed_at) {
        (Some(issued_at), Some(killed_at)) => vec![Loan {
            id: LoanId::new(0),
            kind: AccessKind::Shared,
            issued_at,
            place: PlaceId::new(0),
            origins: vec![ValueOriginId::new(0)].into_boxed_slice(),
            holders: vec![PlaceId::new(0)].into_boxed_slice(),
            uses: vec![UseId::new(0)].into_boxed_slice(),
            kills: vec![killed_at].into_boxed_slice(),
        }],
        (None, None) => Vec::new(),
        _ => panic!("static-verdict probe loan rows were incomplete"),
    };
    BorrowProblem::new(BorrowProblemParts {
        bindings: vec![Binding::synthetic(BindingId::new(0))],
        points: (0..=end.raw())
            .map(|ordinal| ProgramPoint::new(PointId::new(ordinal), BlockId::new(0), ordinal))
            .collect(),
        blocks: vec![CfgBlock::new(
            BlockId::new(0),
            PointId::new(0),
            end,
            (0..events.len())
                .map(|id| EventId::new(id as u32))
                .collect(),
        )],
        entry: BlockId::new(0),
        exits: vec![BlockId::new(0)],
        places: vec![Place::new(PlaceId::new(0), BindingId::new(0), Vec::new())],
        origins: vec![ValueOrigin::fresh(ValueOriginId::new(0))],
        loans,
        uses: vec![
            Use {
                id: UseId::new(0),
                point: keeping_use_point,
                place: PlaceId::new(0),
                kind: UseKind::Read,
                definition: false,
            },
            Use {
                id: UseId::new(1),
                point: conflict_use_point,
                place: PlaceId::new(0),
                kind: UseKind::Write,
                definition: false,
            },
        ],
        events,
        ..BorrowProblemParts::default()
    })
    .expect("static-verdict probe should validate")
}

fn renderer_coverage_problem() -> BorrowProblem {
    let bindings = (0..14)
        .map(|id| match id {
            8 => Binding::new(
                BindingId::new(id),
                None,
                None,
                true,
                false,
                EventSource::none(),
            ),
            _ => Binding::synthetic(BindingId::new(id)),
        })
        .collect::<Vec<_>>();
    let places = vec![
        Place::new(PlaceId::new(0), BindingId::new(0), Vec::new()),
        Place::new(PlaceId::new(1), BindingId::new(1), Vec::new()),
        Place::new(
            PlaceId::new(2),
            BindingId::new(2),
            vec![ProjectionElem::Field(0)],
        ),
        Place::new(
            PlaceId::new(3),
            BindingId::new(3),
            vec![ProjectionElem::FixedIndex(1)],
        ),
        Place::new(
            PlaceId::new(4),
            BindingId::new(4),
            vec![ProjectionElem::DynamicIndex],
        ),
        Place::new(
            PlaceId::new(5),
            BindingId::new(5),
            vec![ProjectionElem::CollectionElement],
        ),
        Place::new(
            PlaceId::new(6),
            BindingId::new(6),
            vec![ProjectionElem::MapEntry],
        ),
        Place::new(PlaceId::new(7), BindingId::new(7), Vec::new()),
        Place::new(PlaceId::new(8), BindingId::new(8), Vec::new()),
        Place::new(PlaceId::new(9), BindingId::new(9), Vec::new()),
        Place::new(PlaceId::new(10), BindingId::new(10), Vec::new()),
        Place::new(PlaceId::new(11), BindingId::new(11), Vec::new()),
        Place::new(PlaceId::new(12), BindingId::new(12), Vec::new()),
        Place::new(PlaceId::new(13), BindingId::new(13), Vec::new()),
    ];
    let origins = vec![
        ValueOrigin::fresh(ValueOriginId::new(0)),
        ValueOrigin::new(ValueOriginId::new(1), OriginKind::Unknown),
        ValueOrigin::new(ValueOriginId::new(2), OriginKind::Parameter { index: 2 }),
        ValueOrigin::new(
            ValueOriginId::new(3),
            OriginKind::Alias(vec![ValueOriginId::new(0)].into_boxed_slice()),
        ),
        ValueOrigin::new(
            ValueOriginId::new(4),
            OriginKind::ExclusiveAlias(vec![ValueOriginId::new(0)].into_boxed_slice()),
        ),
        ValueOrigin::new(
            ValueOriginId::new(5),
            OriginKind::Copy(vec![ValueOriginId::new(0)].into_boxed_slice()),
        ),
        ValueOrigin::new(
            ValueOriginId::new(6),
            OriginKind::Projection {
                source: ValueOriginId::new(0),
                projection: ProjectionElem::MapEntry,
            },
        ),
        ValueOrigin::new(
            ValueOriginId::new(7),
            OriginKind::Join(vec![ValueOriginId::new(0)].into_boxed_slice()),
        ),
        ValueOrigin::new(
            ValueOriginId::new(8),
            OriginKind::CallResult {
                call: CallId::new(0),
                provenance: CallResultProvenance::Fresh,
            },
        ),
        ValueOrigin::new(
            ValueOriginId::new(9),
            OriginKind::CallResult {
                call: CallId::new(1),
                provenance: CallResultProvenance::Alias(
                    vec![ValueOriginId::new(0)].into_boxed_slice(),
                ),
            },
        ),
        ValueOrigin::new(
            ValueOriginId::new(10),
            OriginKind::CallResult {
                call: CallId::new(2),
                provenance: CallResultProvenance::AliasParams(vec![0].into_boxed_slice()),
            },
        ),
        ValueOrigin::new(
            ValueOriginId::new(11),
            OriginKind::CallResult {
                call: CallId::new(3),
                provenance: CallResultProvenance::Unknown(CallResultUnknownReason::SummaryUnknown),
            },
        ),
        ValueOrigin::new(
            ValueOriginId::new(12),
            OriginKind::CallResult {
                call: CallId::new(4),
                provenance: CallResultProvenance::Unknown(CallResultUnknownReason::MissingSummary),
            },
        ),
        ValueOrigin::new(
            ValueOriginId::new(13),
            OriginKind::CallResult {
                call: CallId::new(5),
                provenance: CallResultProvenance::Unknown(CallResultUnknownReason::OpaqueExternal),
            },
        ),
    ];
    let mut events = Vec::new();
    renderer_event(
        &mut events,
        EventKind::Fresh {
            destination: PlaceId::new(0),
            origin: ValueOriginId::new(0),
        },
    );
    renderer_event(
        &mut events,
        EventKind::Alias {
            source: PlaceId::new(0),
            destination: PlaceId::new(1),
            origins: vec![ValueOriginId::new(0)].into_boxed_slice(),
        },
    );
    renderer_event(
        &mut events,
        EventKind::AliasFromPlace {
            source: PlaceId::new(0),
            destination: PlaceId::new(2),
        },
    );
    renderer_event(
        &mut events,
        EventKind::ExclusiveAlias {
            source: PlaceId::new(0),
            destination: PlaceId::new(3),
            origins: vec![ValueOriginId::new(0)].into_boxed_slice(),
        },
    );
    renderer_event(
        &mut events,
        EventKind::ExclusiveAliasFromPlace {
            source: PlaceId::new(0),
            destination: PlaceId::new(4),
        },
    );
    renderer_event(
        &mut events,
        EventKind::Copy {
            source: PlaceId::new(0),
            destination: PlaceId::new(5),
            origin: ValueOriginId::new(5),
        },
    );
    renderer_event(
        &mut events,
        EventKind::Projection {
            source: PlaceId::new(0),
            destination: PlaceId::new(6),
            origin: ValueOriginId::new(6),
        },
    );
    renderer_event(
        &mut events,
        EventKind::Rebind {
            destination: PlaceId::new(0),
            value: RebindValue::Fresh(ValueOriginId::new(1)),
        },
    );
    renderer_event(
        &mut events,
        EventKind::Rebind {
            destination: PlaceId::new(0),
            value: RebindValue::Alias(vec![ValueOriginId::new(0)].into_boxed_slice()),
        },
    );
    renderer_event(
        &mut events,
        EventKind::Rebind {
            destination: PlaceId::new(0),
            value: RebindValue::AliasFromPlace(PlaceId::new(1)),
        },
    );
    renderer_event(
        &mut events,
        EventKind::Aggregate {
            destination: PlaceId::new(7),
            origin: ValueOriginId::new(0),
            fields: vec![
                AggregateField {
                    projection: ProjectionElem::Field(0),
                    source: PlaceId::new(0),
                },
                AggregateField {
                    projection: ProjectionElem::FixedIndex(1),
                    source: PlaceId::new(0),
                },
                AggregateField {
                    projection: ProjectionElem::DynamicIndex,
                    source: PlaceId::new(0),
                },
                AggregateField {
                    projection: ProjectionElem::CollectionElement,
                    source: PlaceId::new(0),
                },
                AggregateField {
                    projection: ProjectionElem::MapEntry,
                    source: PlaceId::new(0),
                },
            ]
            .into_boxed_slice(),
        },
    );
    renderer_event(
        &mut events,
        EventKind::ScopeExit {
            bindings: vec![BindingId::new(0)].into_boxed_slice(),
        },
    );
    renderer_event(
        &mut events,
        EventKind::ReactiveObserve {
            place: PlaceId::new(0),
        },
    );
    let call_argument_point = renderer_event(
        &mut events,
        EventKind::CallArgument {
            call: CallId::new(0),
            index: 0,
            argument: CallArgument {
                place: PlaceId::new(0),
                access: AccessKind::Shared,
                use_id: UseId::new(0),
            },
        },
    );
    renderer_event(
        &mut events,
        EventKind::CallEffect(CallEffect {
            call: CallId::new(0),
            arguments: vec![CallArgument {
                place: PlaceId::new(0),
                access: AccessKind::Shared,
                use_id: UseId::new(0),
            }]
            .into_boxed_slice(),
            result: Some(CallResult {
                place: PlaceId::new(8),
                origin: ValueOriginId::new(8),
            }),
        }),
    );
    let access_write_point = renderer_event(
        &mut events,
        EventKind::Access {
            use_id: UseId::new(1),
        },
    );
    let access_observe_point = renderer_event(
        &mut events,
        EventKind::Access {
            use_id: UseId::new(2),
        },
    );
    let mut loans = Vec::new();
    for (loan, reason) in [
        (LoanId::new(0), KillReason::FinalUse),
        (LoanId::new(1), KillReason::Rebind),
        (LoanId::new(2), KillReason::ScopeExit),
        (LoanId::new(3), KillReason::UnreachableContinuation),
        (LoanId::new(4), KillReason::Explicit),
    ] {
        let issued_at = renderer_event(&mut events, EventKind::LoanIssue { loan });
        let killed_at = renderer_event(&mut events, EventKind::LoanKill { loan, reason });
        loans.push(Loan {
            id: loan,
            kind: AccessKind::Shared,
            issued_at,
            place: PlaceId::new(0),
            origins: vec![ValueOriginId::new(0)].into_boxed_slice(),
            holders: vec![PlaceId::new(0)].into_boxed_slice(),
            uses: Vec::<UseId>::new().into_boxed_slice(),
            kills: vec![killed_at].into_boxed_slice(),
        });
    }
    renderer_event(
        &mut events,
        EventKind::CallEffect(CallEffect {
            call: CallId::new(1),
            arguments: vec![].into_boxed_slice(),
            result: Some(CallResult {
                place: PlaceId::new(9),
                origin: ValueOriginId::new(9),
            }),
        }),
    );
    let alias_params_argument = CallArgument {
        place: PlaceId::new(0),
        access: AccessKind::Exclusive,
        use_id: UseId::new(3),
    };
    let alias_params_argument_point = renderer_event(
        &mut events,
        EventKind::CallArgument {
            call: CallId::new(2),
            index: 0,
            argument: alias_params_argument.clone(),
        },
    );
    renderer_event(
        &mut events,
        EventKind::CallEffect(CallEffect {
            call: CallId::new(2),
            arguments: vec![alias_params_argument].into_boxed_slice(),
            result: Some(CallResult {
                place: PlaceId::new(10),
                origin: ValueOriginId::new(10),
            }),
        }),
    );
    for (call, place, origin) in [
        (3, PlaceId::new(11), ValueOriginId::new(11)),
        (4, PlaceId::new(12), ValueOriginId::new(12)),
        (5, PlaceId::new(13), ValueOriginId::new(13)),
    ] {
        renderer_event(
            &mut events,
            EventKind::CallEffect(CallEffect {
                call: CallId::new(call),
                arguments: vec![].into_boxed_slice(),
                result: Some(CallResult { place, origin }),
            }),
        );
    }
    renderer_event(
        &mut events,
        EventKind::CallEffect(CallEffect {
            call: CallId::new(6),
            arguments: vec![].into_boxed_slice(),
            result: None,
        }),
    );
    let branch_point = renderer_event(
        &mut events,
        EventKind::Terminator {
            kind: TerminatorEventKind::Branch {
                targets: (1..=8).map(BlockId::new).collect(),
            },
        },
    );
    let block_zero_event_count = events.len();
    let terminal_specs = [
        (
            BlockId::new(1),
            TerminatorEventKind::Jump {
                target: BlockId::new(8),
            },
        ),
        (
            BlockId::new(2),
            TerminatorEventKind::Break {
                target: BlockId::new(8),
            },
        ),
        (
            BlockId::new(3),
            TerminatorEventKind::Continue {
                target: BlockId::new(8),
            },
        ),
        (BlockId::new(4), TerminatorEventKind::ReturnSuccess),
        (BlockId::new(5), TerminatorEventKind::ReturnError),
        (BlockId::new(6), TerminatorEventKind::RuntimeFailure),
        (BlockId::new(7), TerminatorEventKind::AssertFailure),
        (BlockId::new(8), TerminatorEventKind::Return),
    ];
    let mut terminal_rows = Vec::new();
    for (index, (block, kind)) in terminal_specs.into_iter().enumerate() {
        let event_id = EventId::new(events.len() as u32);
        let point = PointId::new(branch_point.raw() + index as u32 + 1);
        events.push(Event::new(
            event_id,
            point,
            EventKind::Terminator { kind },
            EventSource::none(),
        ));
        terminal_rows.push((block, event_id, point));
    }
    let mut points = (0..=branch_point.raw())
        .map(|ordinal| ProgramPoint::new(PointId::new(ordinal), BlockId::new(0), ordinal))
        .collect::<Vec<_>>();
    points.extend(
        terminal_rows
            .iter()
            .map(|(block, _, point)| ProgramPoint::new(*point, *block, 0)),
    );
    let mut blocks = vec![CfgBlock::new(
        BlockId::new(0),
        PointId::new(0),
        branch_point,
        (0..block_zero_event_count)
            .map(|id| EventId::new(id as u32))
            .collect(),
    )];
    blocks.extend(
        terminal_rows
            .iter()
            .map(|(block, event_id, point)| CfgBlock::new(*block, *point, *point, vec![*event_id])),
    );
    let mut edges = (1..=8)
        .map(|target| CfgEdge::new(BlockId::new(0), BlockId::new(target)))
        .collect::<Vec<_>>();
    edges.extend(
        [1, 2, 3]
            .into_iter()
            .map(|from| CfgEdge::new(BlockId::new(from), BlockId::new(8))),
    );
    BorrowProblem::new(BorrowProblemParts {
        bindings,
        points,
        blocks,
        edges,
        entry: BlockId::new(0),
        exits: vec![
            BlockId::new(4),
            BlockId::new(5),
            BlockId::new(6),
            BlockId::new(7),
            BlockId::new(8),
        ],
        places,
        origins,
        loans,
        uses: vec![
            Use {
                id: UseId::new(0),
                point: call_argument_point,
                place: PlaceId::new(0),
                kind: UseKind::Read,
                definition: false,
            },
            Use {
                id: UseId::new(1),
                point: access_write_point,
                place: PlaceId::new(1),
                kind: UseKind::Write,
                definition: false,
            },
            Use {
                id: UseId::new(2),
                point: access_observe_point,
                place: PlaceId::new(2),
                kind: UseKind::LoanObservation,
                definition: false,
            },
            Use {
                id: UseId::new(3),
                point: alias_params_argument_point,
                place: PlaceId::new(0),
                kind: UseKind::Write,
                definition: false,
            },
        ],
        calls: vec![
            Call {
                id: CallId::new(0),
                label: "renderer-call".to_string(),
            },
            Call {
                id: CallId::new(1),
                label: "renderer-alias-call".to_string(),
            },
            Call {
                id: CallId::new(2),
                label: "renderer-alias-params-call".to_string(),
            },
            Call {
                id: CallId::new(3),
                label: "renderer-summary-unknown-call".to_string(),
            },
            Call {
                id: CallId::new(4),
                label: "renderer-missing-summary-call".to_string(),
            },
            Call {
                id: CallId::new(5),
                label: "renderer-opaque-external-call".to_string(),
            },
            Call {
                id: CallId::new(6),
                label: "renderer-no-result-call".to_string(),
            },
        ],
        events,
    })
    .expect("renderer coverage fixture should validate")
}

fn renderer_event(events: &mut Vec<Event>, kind: EventKind) -> PointId {
    let event_id = EventId::new(events.len() as u32);
    let point = PointId::new(event_id.raw() + 1);
    events.push(Event::new(event_id, point, kind, EventSource::none()));
    point
}

fn problem_parts(problem: &BorrowProblem) -> BorrowProblemParts {
    let flow = problem.control_flow();
    BorrowProblemParts {
        bindings: problem.bindings().to_vec(),
        points: problem.points().to_vec(),
        blocks: flow.blocks.to_vec(),
        edges: flow.edges.to_vec(),
        entry: flow.entry,
        exits: flow.exits.to_vec(),
        places: problem.places().to_vec(),
        origins: problem.origins().to_vec(),
        loans: problem.loans().to_vec(),
        uses: problem.uses().to_vec(),
        calls: problem.calls().to_vec(),
        events: problem.events().to_vec(),
    }
}

fn problem_with_unused_projection() -> BorrowProblem {
    let mut parts = problem_parts(&dead_exclusive_alias_problem());
    parts.places.push(Place::new(
        PlaceId::new(parts.places.len() as u32),
        BindingId::new(0),
        vec![ProjectionElem::FixedIndex(0)],
    ));
    BorrowProblem::new(parts).expect("unused projected place should validate")
}
fn three_way_branch_with_out_of_order_edges() -> BorrowProblem {
    let points = (0..4)
        .map(|id| ProgramPoint::new(PointId::new(id), BlockId::new(id), 0))
        .collect::<Vec<_>>();
    let blocks = (0..4)
        .map(|id| {
            CfgBlock::new(
                BlockId::new(id),
                PointId::new(id),
                PointId::new(id),
                vec![EventId::new(id)],
            )
        })
        .collect::<Vec<_>>();
    let events = vec![
        Event::new(
            EventId::new(0),
            PointId::new(0),
            EventKind::Terminator {
                kind: TerminatorEventKind::Branch {
                    targets: vec![BlockId::new(1), BlockId::new(2), BlockId::new(3)]
                        .into_boxed_slice(),
                },
            },
            EventSource::none(),
        ),
        Event::new(
            EventId::new(1),
            PointId::new(1),
            EventKind::Terminator {
                kind: TerminatorEventKind::Return,
            },
            EventSource::none(),
        ),
        Event::new(
            EventId::new(2),
            PointId::new(2),
            EventKind::Terminator {
                kind: TerminatorEventKind::Return,
            },
            EventSource::none(),
        ),
        Event::new(
            EventId::new(3),
            PointId::new(3),
            EventKind::Terminator {
                kind: TerminatorEventKind::Return,
            },
            EventSource::none(),
        ),
    ];

    BorrowProblem::new(BorrowProblemParts {
        points,
        blocks,
        edges: vec![
            CfgEdge::new(BlockId::new(0), BlockId::new(1)),
            CfgEdge::new(BlockId::new(0), BlockId::new(3)),
            CfgEdge::new(BlockId::new(0), BlockId::new(2)),
        ],
        entry: BlockId::new(0),
        exits: vec![BlockId::new(1), BlockId::new(2), BlockId::new(3)],
        events,
        ..BorrowProblemParts::default()
    })
    .expect("three-way branch with out-of-order edges should validate")
}
