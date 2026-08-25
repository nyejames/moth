//! Focused tests for the feature-gated Boracle reference solver.

use super::super::problem::{
    AccessKind, AggregateField, Binding, BindingId, BlockId, BorrowProblem, BorrowProblemParts,
    Call, CallArgument, CallEffect, CallResult, CfgBlock, CfgEdge, Event, EventId, EventKind,
    EventSource, KillReason, Loan, LoanId, Place, PlaceId, PointId, ProgramPoint,
    TerminatorEventKind, Use, UseId, UseKind, ValueOrigin, ValueOriginId,
};

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
    let solution = super::LoanSolver::solve(&loan_conflict_problem())
        .expect("loan conflict problem should solve");
    let loan = solution
        .loans()
        .first()
        .expect("explicit loan should be retained");

    assert!(loan.live_points.contains(&PointId::new(2)));
    assert!(!loan.live_points.contains(&PointId::new(4)));
}

#[test]
fn boracle_conflicts_produce_structured_overlap_witnesses() {
    let solution = super::LoanSolver::solve(&loan_conflict_problem())
        .expect("loan conflict problem should solve");

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
            .any(|decision| { decision.event == EventId::new(3) && decision.allowed })
    );
}

#[test]
fn boracle_calls_project_alias_result_provenance_through_arguments() {
    let problem = call_alias_problem();
    let report = super::BoracleSolver::solve(&problem).expect("call problem should solve");
    let result_origins = report
        .origin
        .origins_after_event(EventId::new(1), PlaceId::new(1))
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
    let solution =
        super::OriginSolver::solve(&aggregate_problem()).expect("aggregate problem should solve");

    assert!(solution.traces().iter().any(|trace| {
        trace.rule == super::OriginTraceRule::Aggregate
            && trace.input_origins.as_ref() == [ValueOriginId::new(0)]
    }));
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
            },
        ],
        events,
        ..BorrowProblemParts::default()
    })
    .expect("generated cyclic problem should validate")
}

fn call_alias_problem() -> BorrowProblem {
    BorrowProblem::new(BorrowProblemParts {
        bindings: vec![
            Binding::synthetic(BindingId::new(0)),
            Binding::synthetic(BindingId::new(1)),
        ],
        points: (0..=4)
            .map(|ordinal| ProgramPoint::new(PointId::new(ordinal), BlockId::new(0), ordinal))
            .collect(),
        blocks: vec![CfgBlock::new(
            BlockId::new(0),
            PointId::new(0),
            PointId::new(4),
            vec![EventId::new(0), EventId::new(1), EventId::new(2)],
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
            },
            Use {
                id: UseId::new(1),
                point: PointId::new(3),
                place: PlaceId::new(1),
                kind: UseKind::Write,
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
                EventId::new(2),
                PointId::new(3),
                EventKind::Access {
                    use_id: UseId::new(1),
                },
                EventSource::none(),
            ),
        ],
        ..BorrowProblemParts::default()
    })
    .expect("call alias problem should validate")
}

fn aggregate_problem() -> BorrowProblem {
    BorrowProblem::new(BorrowProblemParts {
        bindings: vec![
            Binding::synthetic(BindingId::new(0)),
            Binding::synthetic(BindingId::new(1)),
        ],
        points: (0..=4)
            .map(|ordinal| ProgramPoint::new(PointId::new(ordinal), BlockId::new(0), ordinal))
            .collect(),
        blocks: vec![CfgBlock::new(
            BlockId::new(0),
            PointId::new(0),
            PointId::new(4),
            vec![EventId::new(0), EventId::new(1), EventId::new(2)],
        )],
        entry: BlockId::new(0),
        exits: vec![BlockId::new(0)],
        places: vec![
            Place::new(PlaceId::new(0), BindingId::new(0), Vec::new()),
            Place::new(PlaceId::new(1), BindingId::new(1), Vec::new()),
        ],
        origins: vec![
            ValueOrigin::fresh(ValueOriginId::new(0)),
            ValueOrigin::fresh(ValueOriginId::new(1)),
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
                EventKind::ScopeExit {
                    bindings: vec![BindingId::new(0), BindingId::new(1)].into_boxed_slice(),
                },
                EventSource::none(),
            ),
        ],
        ..BorrowProblemParts::default()
    })
    .expect("aggregate problem should validate")
}

fn reactive_problem() -> BorrowProblem {
    BorrowProblem::new(BorrowProblemParts {
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
    })
    .expect("reactive problem should validate")
}

fn loan_conflict_problem() -> BorrowProblem {
    BorrowProblem::new(BorrowProblemParts {
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
            kills: vec![PointId::new(3)].into_boxed_slice(),
        }],
        uses: vec![
            Use {
                id: UseId::new(0),
                point: PointId::new(2),
                place: PlaceId::new(0),
                kind: UseKind::Read,
            },
            Use {
                id: UseId::new(1),
                point: PointId::new(4),
                place: PlaceId::new(0),
                kind: UseKind::Read,
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
                EventKind::LoanKill {
                    loan: LoanId::new(0),
                    reason: KillReason::Explicit,
                },
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
    })
    .expect("loan conflict problem should validate")
}

fn copy_problem() -> BorrowProblem {
    BorrowProblem::new(BorrowProblemParts {
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
    })
    .expect("copy problem should validate")
}
