use super::super::oracle::{OracleBounds, OracleOutcome};
use super::super::service::BoracleExperiment;
use super::super::{
    BoracleRuleSelection, OracleComparisonClass, OracleComparisonSeverity, compare_problem_parts,
    compare_reference_and_experiments,
};
use crate::compiler_frontend::analysis::borrow_checker::problem::{
    AccessKind, Binding, BindingId, BlockId, BorrowProblem, BorrowProblemParts, CfgBlock, Event,
    EventId, EventKind, EventSource, Loan, LoanId, Place, PlaceId, PointId, ProgramPoint,
    ProjectionElem, TerminatorEventKind, Use, UseId, UseKind, ValueOrigin, ValueOriginId,
};

#[test]
fn boracle_differential_agrees_for_safe_problem() {
    let set = compare_reference_and_experiments(safe_problem(), OracleBounds::default())
        .expect("safe problem should compare");
    assert_eq!(set.reference.class, OracleComparisonClass::Agreement);
}

#[test]
fn boracle_differential_shared_alias_definition_write_through_is_a_conflict_candidate() {
    // The write-through reclassification (`loans.rs:227-234`) keeps the paired access of a
    // write-through conflict-checked, and the direct rule conflicts every exclusive access
    // whose candidate state is a shared alias, with a pending call-result confirmation as the
    // only exemption. The static alias loan is keyed on the source place, so the holder's own
    // defining write never overlaps the loan row and the static side stays clean. This shape
    // therefore pins the deliberate soundness-candidate disagreement the disagreement
    // workflow records, and guards against the oracle silently re-ignoring defining writes.
    let set = compare_reference_and_experiments(
        shared_alias_definition_problem(),
        OracleBounds::default(),
    )
    .expect("shared alias definition should compare");
    assert_eq!(
        set.reference.class,
        OracleComparisonClass::StaticAcceptedRuntimeConflict
    );
    assert!(
        !set.reference
            .static_report
            .as_ref()
            .expect("reference report should be retained")
            .has_conflicts()
    );
    assert!(
        set.comparisons()
            .all(|comparison| comparison.class
                == OracleComparisonClass::StaticAcceptedRuntimeConflict)
    );
    assert!(matches!(
        set.oracle_outcome,
        Some(OracleOutcome::RuntimeConflict { .. })
    ));
}

#[test]
fn boracle_differential_reports_static_rejection_against_bounded_safety() {
    let set =
        compare_reference_and_experiments(dead_exclusive_alias_problem(), OracleBounds::default())
            .expect("dead exclusive alias should compare");
    assert_eq!(
        set.reference.class,
        OracleComparisonClass::StaticRejectedBoundedSafe
    );
    let Some(OracleOutcome::CompleteSafe { trace, .. }) = set.oracle_outcome.as_ref() else {
        panic!(
            "dead exclusive alias should stay bounded-safe: {:?}",
            set.oracle_outcome
        );
    };
    assert!(
        !trace.entries().is_empty(),
        "a bounded-safe outcome must retain its completed execution trace"
    );
    assert!(
        trace.conflict.is_none(),
        "a bounded-safe trace must not carry a conflict witness"
    );
    let dump = set.report_dump();
    assert!(
        dump.contains("runtime-trace:\nExecutionTrace {"),
        "a bounded-safe report must render its retained trace:\n{dump}"
    );
    assert!(
        !dump.contains("runtime-trace:\nnone"),
        "a bounded-safe report must not render its retained trace as none:\n{dump}"
    );
}

#[test]
fn boracle_differential_classifies_malformed_problem_parts() {
    let set = compare_problem_parts(malformed_parts(), OracleBounds::default())
        .expect("malformed problem should be classified");
    assert!(set.problem.is_none());
    assert!(set.oracle_outcome.is_none());
    assert_eq!(set.comparisons().count(), 2);
    assert!(
        set.comparisons()
            .all(|comparison| comparison.class == OracleComparisonClass::MalformedProblem)
    );
    assert!(
        set.comparisons()
            .all(|comparison| comparison.static_report.is_none())
    );
}

#[test]
fn boracle_differential_inconclusive_overrides_static_acceptance_pairing() {
    let set = compare_reference_and_experiments(safe_problem(), OracleBounds::new(256, 0, 8, 4096))
        .expect("bounded safe problem should compare");
    assert_eq!(
        set.reference.class,
        OracleComparisonClass::OracleInconclusive
    );
    assert!(matches!(
        set.oracle_outcome,
        Some(OracleOutcome::Inconclusive {
            completed_executions: 0,
            ..
        })
    ));
    assert!(
        set.report_dump().contains("completed_executions: 0"),
        "the report must not hide observed completion evidence:\n{}",
        set.report_dump()
    );
}

#[test]
fn boracle_differential_inconclusive_overrides_static_rejection_pairing() {
    let set = compare_reference_and_experiments(
        dead_exclusive_alias_problem(),
        OracleBounds::new(256, 0, 8, 4096),
    )
    .expect("bounded dead exclusive alias should compare");
    assert_eq!(
        set.reference.class,
        OracleComparisonClass::OracleInconclusive
    );
    assert_eq!(
        set.experiments[0].class,
        OracleComparisonClass::OracleInconclusive
    );
}

#[test]
fn boracle_differential_places_reference_before_legality_changing_experiment() {
    let set =
        compare_reference_and_experiments(dead_exclusive_alias_problem(), OracleBounds::default())
            .expect("reference and experiment should compare");
    let mut experiment_selection = BoracleRuleSelection::default();
    experiment_selection
        .experiments
        .insert(BoracleExperiment::DeadExclusiveLoan);
    let selections = set
        .comparisons()
        .map(|comparison| comparison.rule_selection.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        selections,
        vec![BoracleRuleSelection::default(), experiment_selection]
    );
}

#[test]
fn boracle_differential_classifies_experiment_only_acceptance() {
    let set =
        compare_reference_and_experiments(dead_exclusive_alias_problem(), OracleBounds::default())
            .expect("reference and experiment should compare");
    assert_eq!(
        set.experiments[0].class,
        OracleComparisonClass::ExperimentOnlyAcceptedDifference
    );
    assert!(matches!(
        set.oracle_outcome,
        Some(OracleOutcome::CompleteSafe { .. })
    ));
}

#[test]
fn boracle_differential_keeps_runtime_conflict_as_soundness_under_experiment() {
    let set = compare_reference_and_experiments(
        dead_exclusive_runtime_conflict_problem(),
        OracleBounds::default(),
    )
    .expect("reference and experiment should compare");
    assert_eq!(set.reference.class, OracleComparisonClass::Agreement);
    assert!(
        set.reference
            .static_report
            .as_ref()
            .expect("reference report should be retained")
            .has_conflicts()
    );
    assert!(
        !set.experiments[0]
            .static_report
            .as_ref()
            .expect("experiment report should be retained")
            .has_conflicts()
    );
    assert_eq!(
        set.experiments[0].class,
        OracleComparisonClass::StaticAcceptedRuntimeConflict
    );
    assert_eq!(
        set.experiments[0].class.severity(),
        OracleComparisonSeverity::SoundnessFailure
    );
    let Some(OracleOutcome::RuntimeConflict { trace }) = set.oracle_outcome.as_ref() else {
        panic!("explicit loan fixture should produce a runtime conflict");
    };
    // The owner write sits between real holder exercises, so the witness proves interval overlap.
    assert_eq!(
        trace.conflict.as_ref().map(|witness| witness.access_event),
        Some(EventId::new(3))
    );
}

#[test]
fn boracle_differential_report_dump_contains_rule_set_and_experiment_identity() {
    let first = compare_reference_and_experiments(
        dead_exclusive_runtime_conflict_problem(),
        OracleBounds::default(),
    )
    .expect("reference and experiment should compare");
    let second = compare_reference_and_experiments(
        dead_exclusive_runtime_conflict_problem(),
        OracleBounds::default(),
    )
    .expect("reference and experiment should compare");
    assert_eq!(first.reference.class, OracleComparisonClass::Agreement);
    assert_eq!(
        first.experiments[0].class,
        OracleComparisonClass::StaticAcceptedRuntimeConflict
    );
    assert!(matches!(
        first.oracle_outcome.as_ref(),
        Some(OracleOutcome::RuntimeConflict { .. })
    ));
    let dump = first.report_dump();
    assert_eq!(dump, second.report_dump());
    assert!(dump.contains("reference-rule-set = boracle-reference-v1"));
    assert!(dump.contains("experiments = dead-exclusive-loan"));
    assert!(dump.contains("bounds = OracleBounds {"));
    assert!(dump.contains("max_executions: 256"));
    assert!(dump.contains("max_executed_events: 4096"));
    assert!(dump.contains("max_block_entries: 8"));
    assert!(dump.contains("max_dynamic_generations: 4096"));
    let normalized_body = dump
        .split_once("normalized-problem:\n")
        .and_then(|(_, rest)| rest.split_once("\ncomparison 0\n"))
        .map(|(body, _)| body.trim())
        .expect("normalized problem body should be rendered");
    assert!(!normalized_body.is_empty());
    assert_ne!(normalized_body, "none");
    assert!(normalized_body.starts_with("BorrowProblem {"));
    assert!(dump.contains("classification = StaticAcceptedRuntimeConflict"));
    assert!(dump.contains("severity = SoundnessFailure"));
    assert!(dump.contains("static-witnesses:\nlast-use witnesses:"));
    assert!(dump.contains("conflict witnesses:"));
    assert!(dump.contains("oracle-outcome = RuntimeConflict"));
    assert!(dump.contains("runtime-trace:\nExecutionTrace {"));
}

#[test]
fn boracle_differential_severity_marks_only_soundness_as_required_failure() {
    assert!(OracleComparisonSeverity::SoundnessFailure.is_required_failure());
    assert!(!OracleComparisonSeverity::PrecisionCandidate.is_required_failure());
    assert!(!OracleComparisonSeverity::MalformedInput.is_required_failure());
    assert!(!OracleComparisonSeverity::Informational.is_required_failure());
    assert_eq!(
        OracleComparisonClass::Agreement.severity(),
        OracleComparisonSeverity::Informational
    );
    assert_eq!(
        OracleComparisonClass::StaticAcceptedRuntimeConflict.severity(),
        OracleComparisonSeverity::SoundnessFailure
    );
    assert_eq!(
        OracleComparisonClass::StaticRejectedBoundedSafe.severity(),
        OracleComparisonSeverity::PrecisionCandidate
    );
    assert_eq!(
        OracleComparisonClass::OracleInconclusive.severity(),
        OracleComparisonSeverity::Informational
    );
    assert_eq!(
        OracleComparisonClass::MalformedProblem.severity(),
        OracleComparisonSeverity::MalformedInput
    );
    assert_eq!(
        OracleComparisonClass::ExperimentOnlyAcceptedDifference.severity(),
        OracleComparisonSeverity::Informational
    );
}

fn safe_problem() -> BorrowProblem {
    problem_with_events(
        vec![Binding::synthetic(BindingId::new(0))],
        vec![Place::new(PlaceId::new(0), BindingId::new(0), Vec::new())],
        vec![ValueOrigin::fresh(ValueOriginId::new(0))],
        Vec::new(),
        vec![EventKind::Fresh {
            destination: PlaceId::new(0),
            origin: ValueOriginId::new(0),
        }],
    )
}

fn shared_alias_definition_problem() -> BorrowProblem {
    problem_with_events(
        vec![
            Binding::synthetic(BindingId::new(0)),
            Binding::synthetic(BindingId::new(1)),
        ],
        vec![
            Place::new(PlaceId::new(0), BindingId::new(0), Vec::new()),
            Place::new(PlaceId::new(1), BindingId::new(1), Vec::new()),
        ],
        vec![ValueOrigin::fresh(ValueOriginId::new(0))],
        vec![Use {
            id: UseId::new(0),
            point: PointId::new(3),
            place: PlaceId::new(1),
            kind: UseKind::Write,
            definition: true,
        }],
        vec![
            EventKind::Fresh {
                destination: PlaceId::new(0),
                origin: ValueOriginId::new(0),
            },
            EventKind::Alias {
                source: PlaceId::new(0),
                destination: PlaceId::new(1),
                origins: vec![ValueOriginId::new(0)].into_boxed_slice(),
            },
            EventKind::Access {
                use_id: UseId::new(0),
            },
        ],
    )
}

fn dead_exclusive_runtime_conflict_problem() -> BorrowProblem {
    // WHY: The experiment trusts organiser-provided loan uses while the oracle derives exercises
    // from executed holder accesses. This deliberately under-reported row leaves a genuine
    // conflict visible to the conservative reference but hidden from the use-driven experiment.
    let mut parts = problem_parts(
        vec![Binding::synthetic(BindingId::new(0))],
        vec![
            Place::new(PlaceId::new(0), BindingId::new(0), Vec::new()),
            Place::new(
                PlaceId::new(1),
                BindingId::new(0),
                vec![ProjectionElem::Field(0)],
            ),
        ],
        vec![ValueOrigin::fresh(ValueOriginId::new(0))],
        vec![
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
        vec![
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
        ],
    );
    parts.loans.push(Loan {
        id: LoanId::new(0),
        kind: AccessKind::Exclusive,
        issued_at: PointId::new(2),
        place: PlaceId::new(0),
        origins: vec![ValueOriginId::new(0)].into_boxed_slice(),
        holders: vec![PlaceId::new(1)].into_boxed_slice(),
        uses: Vec::new().into_boxed_slice(),
        kills: Vec::new().into_boxed_slice(),
    });
    BorrowProblem::new(parts).expect("dead exclusive runtime conflict should validate")
}

fn dead_exclusive_alias_problem() -> BorrowProblem {
    problem_with_events(
        vec![
            Binding::synthetic(BindingId::new(0)),
            Binding::synthetic(BindingId::new(1)),
        ],
        vec![
            Place::new(PlaceId::new(0), BindingId::new(0), Vec::new()),
            Place::new(PlaceId::new(1), BindingId::new(1), Vec::new()),
        ],
        vec![ValueOrigin::fresh(ValueOriginId::new(0))],
        vec![Use {
            id: UseId::new(0),
            point: PointId::new(3),
            place: PlaceId::new(0),
            kind: UseKind::Write,
            definition: false,
        }],
        vec![
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
        ],
    )
}

fn problem_with_events(
    bindings: Vec<Binding>,
    places: Vec<Place>,
    origins: Vec<ValueOrigin>,
    uses: Vec<Use>,
    event_kinds: Vec<EventKind>,
) -> BorrowProblem {
    let parts = problem_parts(bindings, places, origins, uses, event_kinds);
    BorrowProblem::new(parts).expect("differential test problem should validate")
}

fn malformed_parts() -> BorrowProblemParts {
    problem_parts(
        vec![Binding::synthetic(BindingId::new(0))],
        vec![Place::new(PlaceId::new(0), BindingId::new(0), Vec::new())],
        vec![ValueOrigin::fresh(ValueOriginId::new(0))],
        vec![Use {
            id: UseId::new(0),
            point: PointId::new(1),
            place: PlaceId::new(1),
            kind: UseKind::Read,
            definition: false,
        }],
        vec![EventKind::Access {
            use_id: UseId::new(0),
        }],
    )
}

fn problem_parts(
    bindings: Vec<Binding>,
    places: Vec<Place>,
    origins: Vec<ValueOrigin>,
    uses: Vec<Use>,
    event_kinds: Vec<EventKind>,
) -> BorrowProblemParts {
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
    BorrowProblemParts {
        bindings,
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
        places,
        origins,
        uses,
        events,
        ..BorrowProblemParts::default()
    }
}
