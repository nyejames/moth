//! Focused tests for the feature-gated Boracle reference solver.

use super::super::last_use::{FutureUseStatus, LastUseLocation, LastUseSubject};
use super::super::problem::{
    AccessKind, AggregateField, Binding, BindingId, BlockId, BorrowProblem, BorrowProblemParts,
    Call, CallArgument, CallEffect, CallResult, CfgBlock, CfgEdge, Event, EventId, EventKind,
    EventSource, KillReason, Loan, LoanId, Place, PlaceId, PointId, ProgramPoint,
    TerminatorEventKind, Use, UseId, UseKind, ValueOrigin, ValueOriginId,
};
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::hir::blocks::{HirBlock, HirLocal};
use crate::compiler_frontend::hir::expressions::{HirExpression, HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::functions::HirFunction;
use crate::compiler_frontend::hir::ids::{
    BlockId as HirBlockId, FunctionId, HirValueId, LocalId, RegionId,
};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::regions::HirRegion;
use crate::compiler_frontend::hir::statements::{HirStatement, HirStatementKind};
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;

#[test]
fn boracle_provenance_copy_keeps_source_and_result_origins_independent() {
    let problem = copy_problem();
    let solution = super::OriginSolver::solve(&problem).expect("copy problem should solve");

    assert_eq!(
        solution
            .origins_after_event(EventId::new(1), PlaceId::new(0))
            .expect("source state should be retained"),
        [ValueOriginId::new(0)]
    );
    assert_eq!(
        solution
            .origins_after_event(EventId::new(1), PlaceId::new(1))
            .expect("copy state should be retained"),
        [ValueOriginId::new(1)]
    );
    assert!(
        solution
            .traces()
            .iter()
            .any(|trace| trace.event == EventId::new(1)
                && trace.rule == super::OriginTraceRule::Copy)
    );
}

#[test]
fn borrow_problem_copy_provenance_dump_is_deterministic() {
    let first = super::OriginSolver::solve(&copy_problem()).expect("copy problem should solve");
    let second = super::OriginSolver::solve(&copy_problem()).expect("copy problem should solve");

    assert_eq!(first.debug_dump(), second.debug_dump());
}

#[test]
fn boracle_loans_track_cfg_kills_at_relevant_points() {
    let problem = loan_conflict_problem();
    let origins = super::OriginSolver::solve(&problem).expect("origins should solve");
    let solution =
        super::LoanSolver::solve(&problem, &origins).expect("loan conflict problem should solve");
    let loan = solution
        .loans()
        .first()
        .expect("explicit loan should be retained");

    assert!(loan.live_points.contains(&PointId::new(2)));
    assert!(!loan.live_points.contains(&PointId::new(5)));
}

#[test]
fn boracle_conflicts_produce_structured_overlap_witnesses() {
    let problem = loan_conflict_problem();
    let origins = super::OriginSolver::solve(&problem).expect("origins should solve");
    let solution =
        super::LoanSolver::solve(&problem, &origins).expect("loan conflict problem should solve");

    assert_eq!(solution.conflicts().len(), 1);
    let conflict = &solution.conflicts()[0];
    assert_eq!(conflict.access_event, EventId::new(1));
    assert_eq!(conflict.conflicting_loan, LoanId::new(0));
    assert_eq!(
        conflict.overlap,
        super::super::problem::PlaceOverlap::Overlap
    );
    assert_eq!(conflict.keeping_use, Some(UseId::new(1)));
    assert!(
        solution
            .decisions()
            .iter()
            .any(|decision| { decision.event == EventId::new(4) && decision.allowed })
    );
}

#[test]
fn boracle_calls_project_alias_result_provenance_through_arguments() {
    let problem = call_alias_problem();
    let report = super::BoracleSolver::solve(&problem).expect("call problem should solve");
    let result_origins = report
        .origin
        .origins_after_event(EventId::new(2), PlaceId::new(1))
        .expect("call result state should be retained");

    assert_eq!(result_origins, [ValueOriginId::new(0)]);
    assert!(
        report
            .loans
            .loans()
            .iter()
            .any(|loan| loan.uses.as_ref() == [UseId::new(0)])
    );
}

#[test]
fn boracle_aggregates_retain_stored_child_trace() {
    let problem = aggregate_problem();
    let solution = super::OriginSolver::solve(&problem).expect("aggregate problem should solve");

    assert!(solution.traces().iter().any(|trace| {
        trace.rule == super::OriginTraceRule::Aggregate
            && trace.input_origins.as_ref() == [ValueOriginId::new(0)]
    }));
    assert_eq!(
        solution
            .origins_after_event(EventId::new(2), PlaceId::new(3))
            .expect("projected child state should be retained"),
        [ValueOriginId::new(0)]
    );
    assert!(solution.traces().iter().any(|trace| {
        trace.event == EventId::new(2)
            && trace.rule == super::OriginTraceRule::Projection
            && trace.input_origins.as_ref() == [ValueOriginId::new(0)]
    }));
    let report = super::BoracleSolver::solve(&problem).expect("aggregate report should solve");
    assert!(
        report
            .loans
            .loans()
            .iter()
            .any(|loan| loan.holders.as_ref() == [PlaceId::new(3)])
    );
    assert!(
        report.loans.conflicts().iter().any(|witness| {
            witness.access_place == PlaceId::new(0) && witness.keeping_use == Some(UseId::new(1))
        }),
        "expected aggregate child conflict, got loans={:?} conflicts={:?}",
        report.loans.loans(),
        report.loans.conflicts()
    );
}

#[test]
fn boracle_projection_replacement_tracks_each_holder_generation() {
    let problem = projection_replacement_problem();
    let report =
        super::BoracleSolver::solve(&problem).expect("projection replacement should solve");
    let projection_loans = report
        .loans
        .loans()
        .iter()
        .filter(|loan| loan.holders.as_ref() == [PlaceId::new(2)])
        .collect::<Vec<_>>();

    assert_eq!(projection_loans.len(), 2);
    assert!(
        projection_loans
            .iter()
            .any(|loan| { loan.issue_event == Some(EventId::new(2)) && loan.uses.is_empty() })
    );
    assert!(projection_loans.iter().any(|loan| {
        loan.issue_event == Some(EventId::new(3)) && loan.uses.as_ref() == [UseId::new(0)]
    }));
}

#[test]
fn boracle_aggregate_rebinding_replaces_stored_child_generation() {
    let problem = aggregate_rebinding_problem();
    let solution = super::OriginSolver::solve(&problem).expect("aggregate rebinding should solve");

    assert_eq!(
        solution
            .origins_after_event(EventId::new(3), PlaceId::new(1))
            .expect("rebuilt field should have a current origin"),
        [ValueOriginId::new(2)]
    );
}

#[test]
fn boracle_hir_projection_to_distinct_destination_preserves_stored_child_origin() {
    let problem = hir_distinct_projection_problem();
    let aggregate = problem
        .events()
        .iter()
        .find_map(|event| match &event.kind {
            EventKind::Aggregate { fields, .. } => Some((event.id, fields[0].source)),
            _ => None,
        })
        .expect("HIR tuple assignment should emit aggregate storage");
    let projection = problem
        .events()
        .iter()
        .find_map(|event| match &event.kind {
            EventKind::Projection {
                source,
                destination,
                ..
            } => Some((event.id, *source, *destination)),
            _ => None,
        })
        .expect("HIR tuple assignment should emit a projection");
    assert_ne!(projection.1, projection.2);

    let solution = super::OriginSolver::solve(&problem).expect("HIR projection should solve");
    let child_origins = solution
        .origins_after_event(aggregate.0, aggregate.1)
        .expect("aggregate child origin should be retained");
    assert_eq!(
        solution
            .origins_after_event(projection.0, projection.2)
            .expect("distinct projection destination should retain child origin"),
        child_origins
    );
}

#[test]
fn boracle_hir_aggregate_rebinding_replaces_stale_child_origin() {
    let problem = hir_aggregate_rebinding_problem();
    let aggregates = problem
        .events()
        .iter()
        .filter_map(|event| match &event.kind {
            EventKind::Aggregate { fields, .. } => Some((event.id, fields[0].source)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(aggregates.len(), 2);
    let projection = problem
        .events()
        .iter()
        .find_map(|event| match &event.kind {
            EventKind::Projection { destination, .. } => Some((event.id, *destination)),
            _ => None,
        })
        .expect("rebuilt HIR tuple should project its current child");

    let solution = super::OriginSolver::solve(&problem).expect("HIR aggregate rebind should solve");
    let current_child = solution
        .origins_after_event(aggregates[1].0, aggregates[1].1)
        .expect("current aggregate child should have an origin");
    assert_eq!(
        solution
            .origins_after_event(projection.0, projection.1)
            .expect("projection should use the current aggregate generation"),
        current_child
    );
}

#[test]
fn boracle_same_call_conflict_has_one_truthful_witness() {
    let report = super::BoracleSolver::solve(&same_call_conflict_problem())
        .expect("same-call conflict should solve");

    assert_eq!(report.loans.conflicts().len(), 1);
    let conflict = &report.loans.conflicts()[0];
    assert_eq!(conflict.access_event, EventId::new(1));
    assert_eq!(conflict.conflicting_loan, LoanId::new(0));
    assert_eq!(conflict.keeping_use, None);
}

#[test]
fn boracle_reactivity_is_observability_metadata_not_a_loan() {
    let report =
        super::BoracleSolver::solve(&reactive_problem()).expect("reactive problem should solve");

    assert_eq!(report.reactive_observations.len(), 1);
    assert!(report.loans.loans().is_empty());
    assert!(!report.optional_transfer_allowed(PlaceId::new(0)));
}

#[test]
fn boracle_optional_transfer_requires_a_proven_final_use() {
    let report = super::BoracleSolver::solve(&copy_problem()).expect("copy problem should solve");

    assert!(!report.optional_transfer_allowed_at(PlaceId::new(0), PointId::new(0)));
    assert!(report.optional_transfer_allowed_at(PlaceId::new(0), PointId::new(5)));
    assert!(!report.optional_transfer_allowed_for_origin_after_event(
        ValueOriginId::new(0),
        EventId::new(1),
        PointId::new(2),
    ));
    assert!(report.optional_transfer_allowed_for_origin_after_event(
        ValueOriginId::new(0),
        EventId::new(2),
        PointId::new(3),
    ));
}

#[test]
fn boracle_origin_and_loan_last_use_queries_stop_at_exact_events() {
    let copy_report =
        super::BoracleSolver::solve(&copy_problem()).expect("copy report should solve");
    let origin_after_copy = copy_report
        .origin_last_use_after_event
        .iter()
        .find(|result| {
            result.subject == LastUseSubject::Origin(ValueOriginId::new(0))
                && result.location == LastUseLocation::after_event(EventId::new(1), PointId::new(2))
        })
        .expect("origin query after copy event should be present");
    assert_eq!(origin_after_copy.status, FutureUseStatus::MustBeUsed);

    let origin_after_read = copy_report
        .origin_last_use_after_event
        .iter()
        .find(|result| {
            result.subject == LastUseSubject::Origin(ValueOriginId::new(0))
                && result.location == LastUseLocation::after_event(EventId::new(2), PointId::new(3))
        })
        .expect("origin query after final read should be present");
    assert_eq!(origin_after_read.status, FutureUseStatus::NoFutureUse);

    let loan_report =
        super::BoracleSolver::solve(&loan_conflict_problem()).expect("loan report should solve");
    let loan_after_issue = loan_report
        .loan_last_use_after_event
        .iter()
        .find(|result| {
            result.subject == LastUseSubject::Loan(LoanId::new(0))
                && result.location == LastUseLocation::after_event(EventId::new(0), PointId::new(1))
        })
        .expect("loan query after issue should be present");
    assert_eq!(loan_after_issue.status, FutureUseStatus::MustBeUsed);
}

#[test]
fn boracle_generated_problems_are_deterministic_and_well_formed() {
    for seed in 0..32 {
        let cyclic = seed % 2 == 1;
        let first = generated_problem(seed, cyclic);
        let second = generated_problem(seed, cyclic);
        assert_eq!(first.debug_dump(), second.debug_dump(), "seed={seed}");

        let first_report = super::BoracleSolver::solve(&first)
            .unwrap_or_else(|error| panic!("generated seed {seed} should solve: {error:?}"));
        let second_report = super::BoracleSolver::solve(&second)
            .unwrap_or_else(|error| panic!("generated seed {seed} should solve: {error:?}"));
        assert_eq!(
            first_report.debug_dump(),
            second_report.debug_dump(),
            "seed={seed} cyclic={cyclic}"
        );
        assert!(
            first_report
                .loans
                .decisions()
                .iter()
                .all(|decision| decision.place.index() < first.places().len()),
            "seed={seed} produced an out-of-range access decision"
        );
    }
}

#[test]
fn boracle_generated_problems_preserve_copy_and_rebind_semantics() {
    for seed in 0..32 {
        for cyclic in [false, true] {
            let problem = generated_problem(seed, cyclic);
            let report = super::BoracleSolver::solve(&problem)
                .unwrap_or_else(|error| panic!("generated seed {seed} should solve: {error:?}"));
            let copy_event = problem
                .events()
                .iter()
                .find_map(|event| match event.kind {
                    EventKind::Copy {
                        source,
                        destination,
                        ..
                    } => Some((event.id, source, destination)),
                    _ => None,
                })
                .expect("generated problem should contain its explicit copy event");
            let source_origins =
                report
                    .origin
                    .origins_for_place_after_event(&problem, copy_event.0, copy_event.1);
            let copy_origins = report
                .origin
                .origins_after_event(copy_event.0, copy_event.2)
                .expect("generated copy should publish a destination origin");
            assert!(!source_origins.is_empty(), "seed={seed} cyclic={cyclic}");
            assert!(!copy_origins.is_empty(), "seed={seed} cyclic={cyclic}");
            assert!(
                source_origins
                    .iter()
                    .all(|origin| !copy_origins.contains(origin)),
                "copy origin unexpectedly overlaps source: seed={seed} cyclic={cyclic}"
            );

            let rebind_event = problem
                .events()
                .iter()
                .find_map(|event| match event.kind {
                    EventKind::Rebind { destination, .. } => Some((event.id, destination)),
                    _ => None,
                })
                .expect("generated problem should contain its fresh rebind event");
            let rebound_origins = report
                .origin
                .origins_after_event(rebind_event.0, rebind_event.1)
                .expect("generated rebind should publish a destination origin");
            assert_eq!(rebound_origins, [ValueOriginId::new(2)]);

            if cyclic {
                assert!(
                    problem
                        .control_flow()
                        .edges
                        .iter()
                        .any(|edge| edge.to.raw() <= edge.from.raw())
                );
                assert!(
                    report
                        .loans
                        .loans()
                        .iter()
                        .any(|loan| !loan.origins.is_empty())
                );
            }
        }
    }
}

fn generated_problem(seed: u32, cyclic: bool) -> BorrowProblem {
    let places = vec![
        Place::new(PlaceId::new(0), BindingId::new(0), Vec::new()),
        Place::new(PlaceId::new(1), BindingId::new(1), Vec::new()),
        Place::new(
            PlaceId::new(2),
            BindingId::new(0),
            vec![super::super::problem::ProjectionElem::Field(seed % 2)],
        ),
    ];
    let bindings = (0..3)
        .map(|id| Binding::synthetic(BindingId::new(id)))
        .collect::<Vec<_>>();
    let origins = vec![
        ValueOrigin::fresh(ValueOriginId::new(0)),
        ValueOrigin::new(
            ValueOriginId::new(1),
            super::super::problem::OriginKind::Copy(vec![ValueOriginId::new(0)].into_boxed_slice()),
        ),
        ValueOrigin::fresh(ValueOriginId::new(2)),
    ];

    if cyclic {
        generated_cyclic_problem(bindings, places, origins, seed)
    } else {
        generated_acyclic_problem(bindings, places, origins, seed)
    }
}

fn generated_acyclic_problem(
    bindings: Vec<Binding>,
    places: Vec<Place>,
    origins: Vec<ValueOrigin>,
    seed: u32,
) -> BorrowProblem {
    let events = vec![
        Event::new(
            EventId::new(0),
            PointId::new(1),
            EventKind::Fresh {
                destination: PlaceId::new(0),
                origin: ValueOriginId::new(0),
            },
            EventSource::none(),
        ),
        Event::new(
            EventId::new(1),
            PointId::new(2),
            EventKind::Copy {
                source: PlaceId::new(0),
                destination: PlaceId::new(1),
                origin: ValueOriginId::new(1),
            },
            EventSource::none(),
        ),
        Event::new(
            EventId::new(2),
            PointId::new(3),
            EventKind::LoanIssue {
                loan: LoanId::new(0),
            },
            EventSource::none(),
        ),
        Event::new(
            EventId::new(3),
            PointId::new(4),
            EventKind::Access {
                use_id: UseId::new(0),
            },
            EventSource::none(),
        ),
        Event::new(
            EventId::new(4),
            PointId::new(5),
            EventKind::Rebind {
                destination: PlaceId::new(0),
                value: super::super::problem::RebindValue::Fresh(ValueOriginId::new(2)),
            },
            EventSource::none(),
        ),
        Event::new(
            EventId::new(5),
            PointId::new(6),
            EventKind::Access {
                use_id: UseId::new(1),
            },
            EventSource::none(),
        ),
        Event::new(
            EventId::new(6),
            PointId::new(7),
            EventKind::LoanKill {
                loan: LoanId::new(0),
                reason: KillReason::Explicit,
            },
            EventSource::none(),
        ),
        Event::new(
            EventId::new(7),
            PointId::new(8),
            EventKind::Terminator {
                kind: TerminatorEventKind::Return,
            },
            EventSource::none(),
        ),
    ];
    let uses = vec![
        Use {
            id: UseId::new(0),
            point: PointId::new(4),
            place: PlaceId::new(1),
            kind: UseKind::Read,
            definition: false,
        },
        Use {
            id: UseId::new(1),
            point: PointId::new(6),
            place: PlaceId::new(2),
            kind: if seed.is_multiple_of(2) {
                UseKind::Write
            } else {
                UseKind::Read
            },
            definition: false,
        },
    ];
    let kills = vec![PointId::new(7)];
    BorrowProblem::new(BorrowProblemParts {
        bindings,
        points: (0..=8)
            .map(|id| ProgramPoint::new(PointId::new(id), BlockId::new(0), id))
            .collect(),
        blocks: vec![CfgBlock::new(
            BlockId::new(0),
            PointId::new(0),
            PointId::new(8),
            (0..8).map(EventId::new).collect(),
        )],
        entry: BlockId::new(0),
        exits: vec![BlockId::new(0)],
        places,
        origins,
        loans: vec![Loan {
            id: LoanId::new(0),
            kind: AccessKind::Exclusive,
            issued_at: PointId::new(3),
            place: PlaceId::new(0),
            origins: vec![ValueOriginId::new(0)].into_boxed_slice(),
            holders: vec![PlaceId::new(0)].into_boxed_slice(),
            uses: vec![UseId::new(0)].into_boxed_slice(),
            kills: kills.into_boxed_slice(),
        }],
        uses,
        events,
        ..BorrowProblemParts::default()
    })
    .expect("generated acyclic problem should validate")
}

fn generated_cyclic_problem(
    bindings: Vec<Binding>,
    places: Vec<Place>,
    origins: Vec<ValueOrigin>,
    seed: u32,
) -> BorrowProblem {
    let points = vec![
        ProgramPoint::new(PointId::new(0), BlockId::new(0), 0),
        ProgramPoint::new(PointId::new(1), BlockId::new(0), 1),
        ProgramPoint::new(PointId::new(2), BlockId::new(0), 2),
        ProgramPoint::new(PointId::new(3), BlockId::new(0), 3),
        ProgramPoint::new(PointId::new(4), BlockId::new(1), 0),
        ProgramPoint::new(PointId::new(5), BlockId::new(1), 1),
        ProgramPoint::new(PointId::new(6), BlockId::new(1), 2),
        ProgramPoint::new(PointId::new(7), BlockId::new(1), 3),
        ProgramPoint::new(PointId::new(8), BlockId::new(2), 0),
        ProgramPoint::new(PointId::new(9), BlockId::new(2), 1),
        ProgramPoint::new(PointId::new(10), BlockId::new(2), 2),
        ProgramPoint::new(PointId::new(11), BlockId::new(2), 3),
    ];
    let events = vec![
        Event::new(
            EventId::new(0),
            PointId::new(1),
            EventKind::Fresh {
                destination: PlaceId::new(0),
                origin: ValueOriginId::new(0),
            },
            EventSource::none(),
        ),
        Event::new(
            EventId::new(1),
            PointId::new(2),
            EventKind::Copy {
                source: PlaceId::new(0),
                destination: PlaceId::new(1),
                origin: ValueOriginId::new(1),
            },
            EventSource::none(),
        ),
        Event::new(
            EventId::new(2),
            PointId::new(3),
            EventKind::Terminator {
                kind: TerminatorEventKind::Branch {
                    targets: vec![BlockId::new(1), BlockId::new(2)].into_boxed_slice(),
                },
            },
            EventSource::none(),
        ),
        Event::new(
            EventId::new(3),
            PointId::new(5),
            EventKind::Rebind {
                destination: PlaceId::new(0),
                value: super::super::problem::RebindValue::Fresh(ValueOriginId::new(2)),
            },
            EventSource::none(),
        ),
        Event::new(
            EventId::new(4),
            PointId::new(6),
            EventKind::Access {
                use_id: UseId::new(0),
            },
            EventSource::none(),
        ),
        Event::new(
            EventId::new(5),
            PointId::new(7),
            EventKind::Terminator {
                kind: TerminatorEventKind::Branch {
                    targets: vec![BlockId::new(1), BlockId::new(2)].into_boxed_slice(),
                },
            },
            EventSource::none(),
        ),
        Event::new(
            EventId::new(6),
            PointId::new(9),
            EventKind::AliasFromPlace {
                source: PlaceId::new(0),
                destination: PlaceId::new(2),
            },
            EventSource::none(),
        ),
        Event::new(
            EventId::new(7),
            PointId::new(10),
            EventKind::Access {
                use_id: UseId::new(1),
            },
            EventSource::none(),
        ),
        Event::new(
            EventId::new(8),
            PointId::new(11),
            EventKind::Terminator {
                kind: TerminatorEventKind::Return,
            },
            EventSource::none(),
        ),
    ];
    BorrowProblem::new(BorrowProblemParts {
        bindings,
        points,
        blocks: vec![
            CfgBlock::new(
                BlockId::new(0),
                PointId::new(0),
                PointId::new(3),
                vec![EventId::new(0), EventId::new(1), EventId::new(2)],
            ),
            CfgBlock::new(
                BlockId::new(1),
                PointId::new(4),
                PointId::new(7),
                vec![EventId::new(3), EventId::new(4), EventId::new(5)],
            ),
            CfgBlock::new(
                BlockId::new(2),
                PointId::new(8),
                PointId::new(11),
                vec![EventId::new(6), EventId::new(7), EventId::new(8)],
            ),
        ],
        edges: vec![
            CfgEdge::new(BlockId::new(0), BlockId::new(1)),
            CfgEdge::new(BlockId::new(0), BlockId::new(2)),
            CfgEdge::new(BlockId::new(1), BlockId::new(1)),
            CfgEdge::new(BlockId::new(1), BlockId::new(2)),
        ],
        entry: BlockId::new(0),
        exits: vec![BlockId::new(2)],
        places,
        origins,
        loans: Vec::new(),
        uses: vec![
            Use {
                id: UseId::new(0),
                point: PointId::new(6),
                place: PlaceId::new(1),
                kind: UseKind::Read,
                definition: false,
            },
            Use {
                id: UseId::new(1),
                point: PointId::new(10),
                place: PlaceId::new(2),
                kind: if seed.is_multiple_of(2) {
                    UseKind::Write
                } else {
                    UseKind::Read
                },
                definition: false,
            },
        ],
        events,
        ..BorrowProblemParts::default()
    })
    .expect("generated cyclic problem should validate")
}

fn call_alias_problem() -> BorrowProblem {
    BorrowProblem::new(with_return_terminator(BorrowProblemParts {
        bindings: vec![
            Binding::synthetic(BindingId::new(0)),
            Binding::synthetic(BindingId::new(1)),
        ],
        points: (0..=5)
            .map(|ordinal| ProgramPoint::new(PointId::new(ordinal), BlockId::new(0), ordinal))
            .collect(),
        blocks: vec![CfgBlock::new(
            BlockId::new(0),
            PointId::new(0),
            PointId::new(5),
            vec![
                EventId::new(0),
                EventId::new(1),
                EventId::new(2),
                EventId::new(3),
            ],
        )],
        entry: BlockId::new(0),
        exits: vec![BlockId::new(0)],
        places: vec![
            Place::new(PlaceId::new(0), BindingId::new(0), Vec::new()),
            Place::new(PlaceId::new(1), BindingId::new(1), Vec::new()),
        ],
        origins: vec![
            ValueOrigin::fresh(ValueOriginId::new(0)),
            ValueOrigin::new(
                ValueOriginId::new(1),
                super::super::problem::OriginKind::CallResult {
                    call: super::super::problem::CallId::new(0),
                    provenance: super::super::problem::CallResultProvenance::AliasParams(
                        vec![0].into_boxed_slice(),
                    ),
                },
            ),
        ],
        calls: vec![Call {
            id: super::super::problem::CallId::new(0),
            label: "alias-call".to_owned(),
        }],
        uses: vec![
            Use {
                id: UseId::new(0),
                point: PointId::new(2),
                place: PlaceId::new(0),
                kind: UseKind::Read,
                definition: false,
            },
            Use {
                id: UseId::new(1),
                point: PointId::new(4),
                place: PlaceId::new(1),
                kind: UseKind::Write,
                definition: false,
            },
        ],
        events: vec![
            Event::new(
                EventId::new(0),
                PointId::new(1),
                EventKind::Fresh {
                    destination: PlaceId::new(0),
                    origin: ValueOriginId::new(0),
                },
                EventSource::none(),
            ),
            Event::new(
                EventId::new(1),
                PointId::new(2),
                EventKind::CallArgument {
                    call: super::super::problem::CallId::new(0),
                    index: 0,
                    argument: CallArgument {
                        place: PlaceId::new(0),
                        access: super::super::problem::AccessKind::Shared,
                        use_id: UseId::new(0),
                    },
                },
                EventSource::none(),
            ),
            Event::new(
                EventId::new(2),
                PointId::new(3),
                EventKind::CallEffect(CallEffect {
                    call: super::super::problem::CallId::new(0),
                    arguments: vec![CallArgument {
                        place: PlaceId::new(0),
                        access: super::super::problem::AccessKind::Shared,
                        use_id: UseId::new(0),
                    }]
                    .into_boxed_slice(),
                    result: Some(CallResult {
                        place: PlaceId::new(1),
                        origin: ValueOriginId::new(1),
                    }),
                }),
                EventSource::none(),
            ),
            Event::new(
                EventId::new(3),
                PointId::new(4),
                EventKind::Access {
                    use_id: UseId::new(1),
                },
                EventSource::none(),
            ),
        ],
        ..BorrowProblemParts::default()
    }))
    .expect("call alias problem should validate")
}

fn aggregate_problem() -> BorrowProblem {
    BorrowProblem::new(with_return_terminator(BorrowProblemParts {
        bindings: vec![
            Binding::synthetic(BindingId::new(0)),
            Binding::synthetic(BindingId::new(1)),
            Binding::synthetic(BindingId::new(2)),
        ],
        points: (0..=7)
            .map(|ordinal| ProgramPoint::new(PointId::new(ordinal), BlockId::new(0), ordinal))
            .collect(),
        blocks: vec![CfgBlock::new(
            BlockId::new(0),
            PointId::new(0),
            PointId::new(7),
            vec![
                EventId::new(0),
                EventId::new(1),
                EventId::new(2),
                EventId::new(3),
                EventId::new(4),
                EventId::new(5),
            ],
        )],
        entry: BlockId::new(0),
        exits: vec![BlockId::new(0)],
        places: vec![
            Place::new(PlaceId::new(0), BindingId::new(0), Vec::new()),
            Place::new(PlaceId::new(1), BindingId::new(1), Vec::new()),
            Place::new(
                PlaceId::new(2),
                BindingId::new(1),
                vec![super::super::problem::ProjectionElem::FixedIndex(0)],
            ),
            Place::new(PlaceId::new(3), BindingId::new(2), Vec::new()),
        ],
        origins: vec![
            ValueOrigin::fresh(ValueOriginId::new(0)),
            ValueOrigin::fresh(ValueOriginId::new(1)),
            ValueOrigin::new(
                ValueOriginId::new(2),
                super::super::problem::OriginKind::Projection {
                    source: ValueOriginId::new(1),
                    projection: super::super::problem::ProjectionElem::FixedIndex(0),
                },
            ),
        ],
        events: vec![
            Event::new(
                EventId::new(0),
                PointId::new(1),
                EventKind::Fresh {
                    destination: PlaceId::new(0),
                    origin: ValueOriginId::new(0),
                },
                EventSource::none(),
            ),
            Event::new(
                EventId::new(1),
                PointId::new(2),
                EventKind::Aggregate {
                    destination: PlaceId::new(1),
                    origin: ValueOriginId::new(1),
                    fields: vec![AggregateField {
                        projection: super::super::problem::ProjectionElem::FixedIndex(0),
                        source: PlaceId::new(0),
                    }]
                    .into_boxed_slice(),
                },
                EventSource::none(),
            ),
            Event::new(
                EventId::new(2),
                PointId::new(3),
                EventKind::Projection {
                    source: PlaceId::new(1),
                    destination: PlaceId::new(3),
                    origin: ValueOriginId::new(2),
                },
                EventSource::none(),
            ),
            Event::new(
                EventId::new(3),
                PointId::new(4),
                EventKind::Access {
                    use_id: UseId::new(0),
                },
                EventSource::none(),
            ),
            Event::new(
                EventId::new(4),
                PointId::new(5),
                EventKind::Access {
                    use_id: UseId::new(1),
                },
                EventSource::none(),
            ),
            Event::new(
                EventId::new(5),
                PointId::new(6),
                EventKind::ScopeExit {
                    bindings: vec![BindingId::new(0), BindingId::new(1)].into_boxed_slice(),
                },
                EventSource::none(),
            ),
        ],
        uses: vec![
            Use {
                id: UseId::new(0),
                point: PointId::new(4),
                place: PlaceId::new(0),
                kind: UseKind::Write,
                definition: false,
            },
            Use {
                id: UseId::new(1),
                point: PointId::new(5),
                place: PlaceId::new(3),
                kind: UseKind::Read,
                definition: false,
            },
        ],
        ..BorrowProblemParts::default()
    }))
    .expect("aggregate problem should validate")
}

fn reactive_problem() -> BorrowProblem {
    BorrowProblem::new(with_return_terminator(BorrowProblemParts {
        bindings: vec![Binding::synthetic(BindingId::new(0))],
        points: vec![
            ProgramPoint::new(PointId::new(0), BlockId::new(0), 0),
            ProgramPoint::new(PointId::new(1), BlockId::new(0), 1),
            ProgramPoint::new(PointId::new(2), BlockId::new(0), 2),
        ],
        blocks: vec![CfgBlock::new(
            BlockId::new(0),
            PointId::new(0),
            PointId::new(2),
            vec![EventId::new(0)],
        )],
        entry: BlockId::new(0),
        exits: vec![BlockId::new(0)],
        places: vec![Place::new(PlaceId::new(0), BindingId::new(0), Vec::new())],
        origins: vec![ValueOrigin::fresh(ValueOriginId::new(0))],
        events: vec![Event::new(
            EventId::new(0),
            PointId::new(1),
            EventKind::ReactiveObserve {
                place: PlaceId::new(0),
            },
            EventSource::none(),
        )],
        ..BorrowProblemParts::default()
    }))
    .expect("reactive problem should validate")
}

fn projection_replacement_problem() -> BorrowProblem {
    let projection = super::super::problem::ProjectionElem::FixedIndex(0);
    BorrowProblem::new(with_return_terminator(BorrowProblemParts {
        bindings: vec![
            Binding::synthetic(BindingId::new(0)),
            Binding::synthetic(BindingId::new(1)),
            Binding::synthetic(BindingId::new(2)),
        ],
        points: (0..=6)
            .map(|ordinal| ProgramPoint::new(PointId::new(ordinal), BlockId::new(0), ordinal))
            .collect(),
        blocks: vec![CfgBlock::new(
            BlockId::new(0),
            PointId::new(0),
            PointId::new(6),
            (0..5).map(EventId::new).collect(),
        )],
        entry: BlockId::new(0),
        exits: vec![BlockId::new(0)],
        places: vec![
            Place::new(PlaceId::new(0), BindingId::new(0), Vec::new()),
            Place::new(PlaceId::new(1), BindingId::new(1), Vec::new()),
            Place::new(PlaceId::new(2), BindingId::new(2), Vec::new()),
        ],
        origins: vec![
            ValueOrigin::fresh(ValueOriginId::new(0)),
            ValueOrigin::fresh(ValueOriginId::new(1)),
            ValueOrigin::new(
                ValueOriginId::new(2),
                super::super::problem::OriginKind::Projection {
                    source: ValueOriginId::new(0),
                    projection,
                },
            ),
            ValueOrigin::new(
                ValueOriginId::new(3),
                super::super::problem::OriginKind::Projection {
                    source: ValueOriginId::new(1),
                    projection,
                },
            ),
        ],
        uses: vec![Use {
            id: UseId::new(0),
            point: PointId::new(5),
            place: PlaceId::new(2),
            kind: UseKind::Read,
            definition: false,
        }],
        events: vec![
            Event::new(
                EventId::new(0),
                PointId::new(1),
                EventKind::Fresh {
                    destination: PlaceId::new(0),
                    origin: ValueOriginId::new(0),
                },
                EventSource::none(),
            ),
            Event::new(
                EventId::new(1),
                PointId::new(2),
                EventKind::Fresh {
                    destination: PlaceId::new(1),
                    origin: ValueOriginId::new(1),
                },
                EventSource::none(),
            ),
            Event::new(
                EventId::new(2),
                PointId::new(3),
                EventKind::Projection {
                    source: PlaceId::new(0),
                    destination: PlaceId::new(2),
                    origin: ValueOriginId::new(2),
                },
                EventSource::none(),
            ),
            Event::new(
                EventId::new(3),
                PointId::new(4),
                EventKind::Projection {
                    source: PlaceId::new(1),
                    destination: PlaceId::new(2),
                    origin: ValueOriginId::new(3),
                },
                EventSource::none(),
            ),
            Event::new(
                EventId::new(4),
                PointId::new(5),
                EventKind::Access {
                    use_id: UseId::new(0),
                },
                EventSource::none(),
            ),
        ],
        ..BorrowProblemParts::default()
    }))
    .expect("projection replacement problem should validate")
}

fn aggregate_rebinding_problem() -> BorrowProblem {
    let projection = super::super::problem::ProjectionElem::FixedIndex(0);
    BorrowProblem::new(with_return_terminator(BorrowProblemParts {
        bindings: vec![
            Binding::synthetic(BindingId::new(0)),
            Binding::synthetic(BindingId::new(1)),
            Binding::synthetic(BindingId::new(2)),
        ],
        points: (0..=6)
            .map(|ordinal| ProgramPoint::new(PointId::new(ordinal), BlockId::new(0), ordinal))
            .collect(),
        blocks: vec![CfgBlock::new(
            BlockId::new(0),
            PointId::new(0),
            PointId::new(6),
            (0..5).map(EventId::new).collect(),
        )],
        entry: BlockId::new(0),
        exits: vec![BlockId::new(0)],
        places: vec![
            Place::new(PlaceId::new(0), BindingId::new(0), Vec::new()),
            Place::new(PlaceId::new(1), BindingId::new(0), vec![projection]),
            Place::new(PlaceId::new(2), BindingId::new(1), Vec::new()),
            Place::new(PlaceId::new(3), BindingId::new(2), Vec::new()),
        ],
        origins: vec![
            ValueOrigin::fresh(ValueOriginId::new(0)),
            ValueOrigin::fresh(ValueOriginId::new(1)),
            ValueOrigin::fresh(ValueOriginId::new(2)),
            ValueOrigin::fresh(ValueOriginId::new(3)),
        ],
        uses: vec![Use {
            id: UseId::new(0),
            point: PointId::new(5),
            place: PlaceId::new(1),
            kind: UseKind::Read,
            definition: false,
        }],
        events: vec![
            Event::new(
                EventId::new(0),
                PointId::new(1),
                EventKind::Fresh {
                    destination: PlaceId::new(2),
                    origin: ValueOriginId::new(0),
                },
                EventSource::none(),
            ),
            Event::new(
                EventId::new(1),
                PointId::new(2),
                EventKind::Aggregate {
                    destination: PlaceId::new(0),
                    origin: ValueOriginId::new(1),
                    fields: vec![AggregateField {
                        projection,
                        source: PlaceId::new(2),
                    }]
                    .into_boxed_slice(),
                },
                EventSource::none(),
            ),
            Event::new(
                EventId::new(2),
                PointId::new(3),
                EventKind::Fresh {
                    destination: PlaceId::new(3),
                    origin: ValueOriginId::new(2),
                },
                EventSource::none(),
            ),
            Event::new(
                EventId::new(3),
                PointId::new(4),
                EventKind::Aggregate {
                    destination: PlaceId::new(0),
                    origin: ValueOriginId::new(3),
                    fields: vec![AggregateField {
                        projection,
                        source: PlaceId::new(3),
                    }]
                    .into_boxed_slice(),
                },
                EventSource::none(),
            ),
            Event::new(
                EventId::new(4),
                PointId::new(5),
                EventKind::Access {
                    use_id: UseId::new(0),
                },
                EventSource::none(),
            ),
        ],
        ..BorrowProblemParts::default()
    }))
    .expect("aggregate rebinding problem should validate")
}

fn same_call_conflict_problem() -> BorrowProblem {
    let first_argument = CallArgument {
        place: PlaceId::new(0),
        access: AccessKind::Shared,
        use_id: UseId::new(0),
    };
    let second_argument = CallArgument {
        place: PlaceId::new(0),
        access: AccessKind::Exclusive,
        use_id: UseId::new(1),
    };
    BorrowProblem::new(with_return_terminator(BorrowProblemParts {
        bindings: vec![Binding::synthetic(BindingId::new(0))],
        points: (0..=3)
            .map(|ordinal| ProgramPoint::new(PointId::new(ordinal), BlockId::new(0), ordinal))
            .collect(),
        blocks: vec![CfgBlock::new(
            BlockId::new(0),
            PointId::new(0),
            PointId::new(3),
            vec![EventId::new(0), EventId::new(1), EventId::new(2)],
        )],
        entry: BlockId::new(0),
        exits: vec![BlockId::new(0)],
        places: vec![Place::new(PlaceId::new(0), BindingId::new(0), Vec::new())],
        origins: vec![ValueOrigin::fresh(ValueOriginId::new(0))],
        uses: vec![
            Use {
                id: UseId::new(0),
                point: PointId::new(1),
                place: PlaceId::new(0),
                kind: UseKind::Read,
                definition: false,
            },
            Use {
                id: UseId::new(1),
                point: PointId::new(2),
                place: PlaceId::new(0),
                kind: UseKind::Write,
                definition: false,
            },
        ],
        calls: vec![Call {
            id: super::super::problem::CallId::new(0),
            label: "same-call-conflict".to_owned(),
        }],
        events: vec![
            Event::new(
                EventId::new(0),
                PointId::new(1),
                EventKind::CallArgument {
                    call: super::super::problem::CallId::new(0),
                    index: 0,
                    argument: first_argument.clone(),
                },
                EventSource::none(),
            ),
            Event::new(
                EventId::new(1),
                PointId::new(2),
                EventKind::CallArgument {
                    call: super::super::problem::CallId::new(0),
                    index: 1,
                    argument: second_argument.clone(),
                },
                EventSource::none(),
            ),
            Event::new(
                EventId::new(2),
                PointId::new(3),
                EventKind::CallEffect(CallEffect {
                    call: super::super::problem::CallId::new(0),
                    arguments: vec![first_argument, second_argument].into_boxed_slice(),
                    result: None,
                }),
                EventSource::none(),
            ),
        ],
        ..BorrowProblemParts::default()
    }))
    .expect("same-call conflict problem should validate")
}

fn hir_distinct_projection_problem() -> BorrowProblem {
    let region = RegionId(0);
    let source = LocalId(0);
    let result = LocalId(1);
    let tuple = HirExpression {
        id: HirValueId(0),
        kind: HirExpressionKind::TupleConstruct {
            elements: vec![
                hir_int_expression(1, 1, region),
                hir_int_expression(2, 2, region),
            ],
        },
        ty: builtin_type_ids::INT,
        value_kind: ValueKind::RValue,
        region,
    };
    let projected = HirExpression {
        id: HirValueId(3),
        kind: HirExpressionKind::TupleGet {
            tuple: Box::new(HirExpression {
                id: HirValueId(4),
                kind: HirExpressionKind::Load(HirPlace::Local(source)),
                ty: builtin_type_ids::INT,
                value_kind: ValueKind::Place,
                region,
            }),
            index: 0,
        },
        ty: builtin_type_ids::INT,
        value_kind: ValueKind::RValue,
        region,
    };
    let module = HirModule {
        blocks: vec![HirBlock {
            id: HirBlockId(0),
            region,
            locals: vec![hir_local(source, region), hir_local(result, region)],
            statements: vec![
                HirStatement {
                    id: crate::compiler_frontend::hir::ids::HirNodeId(0),
                    kind: HirStatementKind::Assign {
                        target: HirPlace::Local(source),
                        value: tuple,
                    },
                    location: SourceLocation::default(),
                },
                HirStatement {
                    id: crate::compiler_frontend::hir::ids::HirNodeId(1),
                    kind: HirStatementKind::Assign {
                        target: HirPlace::Local(result),
                        value: projected,
                    },
                    location: SourceLocation::default(),
                },
            ],
            terminator: HirTerminator::Return(HirExpression {
                id: HirValueId(5),
                kind: HirExpressionKind::Load(HirPlace::Local(result)),
                ty: builtin_type_ids::INT,
                value_kind: ValueKind::Place,
                region,
            }),
        }],
        regions: vec![HirRegion::lexical(region, None)],
        ..HirModule::new()
    };
    let function = HirFunction {
        id: FunctionId(0),
        entry: HirBlockId(0),
        params: Vec::new(),
        return_type: builtin_type_ids::INT,
    };
    super::super::problem::from_hir(&module, &function, None, None)
        .expect("distinct HIR projection should extract")
}

fn hir_aggregate_rebinding_problem() -> BorrowProblem {
    let region = RegionId(0);
    let source = LocalId(0);
    let result = LocalId(1);
    let tuple_one = HirExpression {
        id: HirValueId(0),
        kind: HirExpressionKind::TupleConstruct {
            elements: vec![hir_int_expression(1, 1, region)],
        },
        ty: builtin_type_ids::INT,
        value_kind: ValueKind::RValue,
        region,
    };
    let tuple_two = HirExpression {
        id: HirValueId(2),
        kind: HirExpressionKind::TupleConstruct {
            elements: vec![hir_int_expression(3, 2, region)],
        },
        ty: builtin_type_ids::INT,
        value_kind: ValueKind::RValue,
        region,
    };
    let projected = HirExpression {
        id: HirValueId(4),
        kind: HirExpressionKind::TupleGet {
            tuple: Box::new(HirExpression {
                id: HirValueId(5),
                kind: HirExpressionKind::Load(HirPlace::Local(source)),
                ty: builtin_type_ids::INT,
                value_kind: ValueKind::Place,
                region,
            }),
            index: 0,
        },
        ty: builtin_type_ids::INT,
        value_kind: ValueKind::RValue,
        region,
    };
    let module = HirModule {
        blocks: vec![HirBlock {
            id: HirBlockId(0),
            region,
            locals: vec![hir_local(source, region), hir_local(result, region)],
            statements: vec![
                HirStatement {
                    id: crate::compiler_frontend::hir::ids::HirNodeId(0),
                    kind: HirStatementKind::Assign {
                        target: HirPlace::Local(source),
                        value: tuple_one,
                    },
                    location: SourceLocation::default(),
                },
                HirStatement {
                    id: crate::compiler_frontend::hir::ids::HirNodeId(1),
                    kind: HirStatementKind::Assign {
                        target: HirPlace::Local(source),
                        value: tuple_two,
                    },
                    location: SourceLocation::default(),
                },
                HirStatement {
                    id: crate::compiler_frontend::hir::ids::HirNodeId(2),
                    kind: HirStatementKind::Assign {
                        target: HirPlace::Local(result),
                        value: projected,
                    },
                    location: SourceLocation::default(),
                },
            ],
            terminator: HirTerminator::Return(HirExpression {
                id: HirValueId(6),
                kind: HirExpressionKind::Load(HirPlace::Local(result)),
                ty: builtin_type_ids::INT,
                value_kind: ValueKind::Place,
                region,
            }),
        }],
        regions: vec![HirRegion::lexical(region, None)],
        ..HirModule::new()
    };
    let function = HirFunction {
        id: FunctionId(0),
        entry: HirBlockId(0),
        params: Vec::new(),
        return_type: builtin_type_ids::INT,
    };
    super::super::problem::from_hir(&module, &function, None, None)
        .expect("aggregate rebinding HIR should extract")
}

fn hir_local(id: LocalId, region: RegionId) -> HirLocal {
    HirLocal {
        id,
        ty: builtin_type_ids::INT,
        mutable: true,
        region,
        source_info: None,
    }
}

fn hir_int_expression(id: u32, value: i32, region: RegionId) -> HirExpression {
    HirExpression {
        id: HirValueId(id),
        kind: HirExpressionKind::Int(value),
        ty: builtin_type_ids::INT,
        value_kind: ValueKind::RValue,
        region,
    }
}

fn loan_conflict_problem() -> BorrowProblem {
    BorrowProblem::new(with_return_terminator(BorrowProblemParts {
        bindings: vec![Binding::synthetic(BindingId::new(0))],
        points: vec![
            ProgramPoint::new(PointId::new(0), BlockId::new(0), 0),
            ProgramPoint::new(PointId::new(1), BlockId::new(0), 1),
            ProgramPoint::new(PointId::new(2), BlockId::new(0), 2),
            ProgramPoint::new(PointId::new(3), BlockId::new(0), 3),
            ProgramPoint::new(PointId::new(4), BlockId::new(0), 4),
            ProgramPoint::new(PointId::new(5), BlockId::new(0), 5),
        ],
        blocks: vec![CfgBlock::new(
            BlockId::new(0),
            PointId::new(0),
            PointId::new(5),
            vec![
                EventId::new(0),
                EventId::new(1),
                EventId::new(2),
                EventId::new(3),
                EventId::new(4),
            ],
        )],
        entry: BlockId::new(0),
        exits: vec![BlockId::new(0)],
        places: vec![Place::new(PlaceId::new(0), BindingId::new(0), Vec::new())],
        origins: vec![ValueOrigin::fresh(ValueOriginId::new(0))],
        loans: vec![Loan {
            id: LoanId::new(0),
            kind: super::super::problem::AccessKind::Exclusive,
            issued_at: PointId::new(1),
            place: PlaceId::new(0),
            origins: vec![ValueOriginId::new(0)].into_boxed_slice(),
            holders: vec![PlaceId::new(0)].into_boxed_slice(),
            uses: vec![UseId::new(1)].into_boxed_slice(),
            kills: vec![PointId::new(4)].into_boxed_slice(),
        }],
        uses: vec![
            Use {
                id: UseId::new(0),
                point: PointId::new(2),
                place: PlaceId::new(0),
                kind: UseKind::Read,
                definition: false,
            },
            Use {
                id: UseId::new(1),
                point: PointId::new(3),
                place: PlaceId::new(0),
                kind: UseKind::Read,
                definition: false,
            },
            Use {
                id: UseId::new(2),
                point: PointId::new(5),
                place: PlaceId::new(0),
                kind: UseKind::Read,
                definition: false,
            },
        ],
        events: vec![
            Event::new(
                EventId::new(0),
                PointId::new(1),
                EventKind::LoanIssue {
                    loan: LoanId::new(0),
                },
                EventSource::none(),
            ),
            Event::new(
                EventId::new(1),
                PointId::new(2),
                EventKind::Access {
                    use_id: UseId::new(0),
                },
                EventSource::none(),
            ),
            Event::new(
                EventId::new(2),
                PointId::new(3),
                EventKind::Access {
                    use_id: UseId::new(1),
                },
                EventSource::none(),
            ),
            Event::new(
                EventId::new(3),
                PointId::new(4),
                EventKind::LoanKill {
                    loan: LoanId::new(0),
                    reason: KillReason::Explicit,
                },
                EventSource::none(),
            ),
            Event::new(
                EventId::new(4),
                PointId::new(5),
                EventKind::Access {
                    use_id: UseId::new(2),
                },
                EventSource::none(),
            ),
        ],
        ..BorrowProblemParts::default()
    }))
    .expect("loan conflict problem should validate")
}

fn with_return_terminator(mut parts: BorrowProblemParts) -> BorrowProblemParts {
    assert_eq!(parts.blocks.len(), 1, "test helper expects one CFG block");
    let block_exit = parts.blocks[0].exit;
    let event_id = EventId::new(parts.events.len() as u32);
    parts.events.push(Event::new(
        event_id,
        block_exit,
        EventKind::Terminator {
            kind: TerminatorEventKind::Return,
        },
        EventSource::none(),
    ));
    let block = &mut parts.blocks[0];
    let mut event_ids = block.events.to_vec();
    event_ids.push(event_id);
    block.events = event_ids.into_boxed_slice();
    parts
}

fn copy_problem() -> BorrowProblem {
    BorrowProblem::new(with_return_terminator(BorrowProblemParts {
        bindings: vec![
            Binding::synthetic(BindingId::new(0)),
            Binding::synthetic(BindingId::new(1)),
        ],
        points: vec![
            ProgramPoint::new(PointId::new(0), BlockId::new(0), 0),
            ProgramPoint::new(PointId::new(1), BlockId::new(0), 1),
            ProgramPoint::new(PointId::new(2), BlockId::new(0), 2),
            ProgramPoint::new(PointId::new(3), BlockId::new(0), 3),
            ProgramPoint::new(PointId::new(4), BlockId::new(0), 4),
            ProgramPoint::new(PointId::new(5), BlockId::new(0), 5),
        ],
        blocks: vec![CfgBlock::new(
            BlockId::new(0),
            PointId::new(0),
            PointId::new(5),
            (0..5).map(EventId::new).collect(),
        )],
        edges: Vec::<CfgEdge>::new(),
        entry: BlockId::new(0),
        exits: vec![BlockId::new(0)],
        places: vec![
            Place::new(PlaceId::new(0), BindingId::new(0), Vec::new()),
            Place::new(PlaceId::new(1), BindingId::new(1), Vec::new()),
        ],
        origins: vec![
            ValueOrigin::fresh(ValueOriginId::new(0)),
            ValueOrigin::new(
                ValueOriginId::new(1),
                super::super::problem::OriginKind::Copy(
                    vec![ValueOriginId::new(0)].into_boxed_slice(),
                ),
            ),
        ],
        uses: vec![Use {
            id: UseId::new(0),
            point: PointId::new(3),
            place: PlaceId::new(0),
            kind: UseKind::Read,
            definition: false,
        }],
        events: vec![
            Event::new(
                EventId::new(0),
                PointId::new(1),
                EventKind::Fresh {
                    destination: PlaceId::new(0),
                    origin: ValueOriginId::new(0),
                },
                EventSource::none(),
            ),
            Event::new(
                EventId::new(1),
                PointId::new(2),
                EventKind::Copy {
                    source: PlaceId::new(0),
                    destination: PlaceId::new(1),
                    origin: ValueOriginId::new(1),
                },
                EventSource::none(),
            ),
            Event::new(
                EventId::new(2),
                PointId::new(3),
                EventKind::Access {
                    use_id: UseId::new(0),
                },
                EventSource::none(),
            ),
            Event::new(
                EventId::new(3),
                PointId::new(4),
                EventKind::Terminator {
                    kind: TerminatorEventKind::Return,
                },
                EventSource::none(),
            ),
            Event::new(
                EventId::new(4),
                PointId::new(5),
                EventKind::ScopeExit {
                    bindings: vec![BindingId::new(0), BindingId::new(1)].into_boxed_slice(),
                },
                EventSource::none(),
            ),
        ],
        ..BorrowProblemParts::default()
    }))
    .expect("copy problem should validate")
}
