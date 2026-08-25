use super::{FutureUseStatus, LastUseAnalysis, LastUseLocation, LastUseSubject, LastUseWitness};
use crate::compiler_frontend::analysis::borrow_checker::problem::{
    Binding, BindingId, BlockId, BorrowProblem, BorrowProblemParts, CfgBlock, CfgEdge, Event,
    EventId, EventKind, EventSource, Place, PlaceId, PointId, ProgramPoint, TerminatorEventKind,
    Use, UseId, UseKind, ValueOrigin, ValueOriginId,
};
use std::collections::BTreeSet;

#[test]
fn last_use_branch_reports_path_dependent_may_with_both_witnesses() {
    let problem = branch_problem();
    let analysis = LastUseAnalysis::from_problem(&problem).expect("branch problem should analyze");
    let result = analysis
        .query(
            LastUseSubject::Place(PlaceId::new(0)),
            LastUseLocation::after_event(EventId::new(0), PointId::new(1)),
        )
        .expect("branch query should succeed");

    assert_eq!(result.status, FutureUseStatus::MayBeUsed);
    assert!(matches!(
        result.witness,
        LastUseWitness::MayBeUsed {
            later_use,
            no_use_exit: Some(no_use_exit),
        } if later_use == UseId::new(0) && no_use_exit == BlockId::new(2)
    ));
}

#[test]
fn last_use_single_block_reports_must_after_entry() {
    let problem = single_use_problem();
    let analysis =
        LastUseAnalysis::from_problem(&problem).expect("single-use problem should analyze");
    let result = analysis
        .query_place(PlaceId::new(0), LastUseLocation::at_point(PointId::new(0)))
        .expect("single-block query should succeed");

    assert_eq!(result.status, FutureUseStatus::MustBeUsed);
    assert!(matches!(
        result.witness,
        LastUseWitness::MustBeUsed { later_use } if later_use == UseId::new(0)
    ));
}

#[test]
fn last_use_after_event_excludes_the_queried_event_observation() {
    let problem = single_use_problem();
    let analysis =
        LastUseAnalysis::from_problem(&problem).expect("single-use problem should analyze");
    let subject = LastUseSubject::Place(PlaceId::new(0));

    let before_event = analysis
        .query(subject, LastUseLocation::at_point(PointId::new(1)))
        .expect("before-event query should succeed");
    let after_event = analysis
        .query(
            subject,
            LastUseLocation::after_event(EventId::new(0), PointId::new(1)),
        )
        .expect("after-event query should succeed");

    assert_eq!(before_event.status, FutureUseStatus::MustBeUsed);
    assert_eq!(after_event.status, FutureUseStatus::NoFutureUse);
}

#[test]
fn last_use_ignores_an_unreachable_use() {
    let problem = unreachable_use_problem();
    let analysis =
        LastUseAnalysis::from_problem(&problem).expect("unreachable-use problem should analyze");
    let result = analysis
        .query_place(PlaceId::new(0), LastUseLocation::at_point(PointId::new(0)))
        .expect("unreachable query should succeed");

    assert_eq!(result.status, FutureUseStatus::NoFutureUse);
    assert!(matches!(
        result.witness,
        LastUseWitness::NoFutureUse { explored_exits } if explored_exits.as_ref() == [BlockId::new(0)]
    ));
}

fn single_use_problem() -> BorrowProblem {
    problem(
        vec![
            ProgramPoint::new(PointId::new(0), BlockId::new(0), 0),
            ProgramPoint::new(PointId::new(1), BlockId::new(0), 1),
            ProgramPoint::new(PointId::new(2), BlockId::new(0), 2),
        ],
        vec![CfgBlock::new(
            BlockId::new(0),
            PointId::new(0),
            PointId::new(2),
            vec![EventId::new(0)],
        )],
        vec![],
        vec![BlockId::new(0)],
        vec![Event::new(
            EventId::new(0),
            PointId::new(1),
            EventKind::Access {
                use_id: UseId::new(0),
            },
            EventSource::none(),
        )],
        vec![Use {
            id: UseId::new(0),
            point: PointId::new(1),
            place: PlaceId::new(0),
            kind: UseKind::Read,
            definition: false,
        }],
    )
}

fn branch_problem() -> BorrowProblem {
    problem(
        vec![
            ProgramPoint::new(PointId::new(0), BlockId::new(0), 0),
            ProgramPoint::new(PointId::new(1), BlockId::new(0), 1),
            ProgramPoint::new(PointId::new(2), BlockId::new(1), 0),
            ProgramPoint::new(PointId::new(3), BlockId::new(1), 1),
            ProgramPoint::new(PointId::new(4), BlockId::new(2), 0),
            ProgramPoint::new(PointId::new(5), BlockId::new(2), 1),
        ],
        vec![
            CfgBlock::new(
                BlockId::new(0),
                PointId::new(0),
                PointId::new(1),
                vec![EventId::new(0)],
            ),
            CfgBlock::new(
                BlockId::new(1),
                PointId::new(2),
                PointId::new(3),
                vec![EventId::new(1)],
            ),
            CfgBlock::new(BlockId::new(2), PointId::new(4), PointId::new(5), vec![]),
        ],
        vec![
            CfgEdge::new(BlockId::new(0), BlockId::new(1)),
            CfgEdge::new(BlockId::new(0), BlockId::new(2)),
        ],
        vec![BlockId::new(1), BlockId::new(2)],
        vec![
            Event::new(
                EventId::new(0),
                PointId::new(1),
                EventKind::Terminator {
                    kind: super::super::problem::TerminatorEventKind::Branch {
                        targets: vec![BlockId::new(1), BlockId::new(2)].into_boxed_slice(),
                    },
                },
                EventSource::none(),
            ),
            Event::new(
                EventId::new(1),
                PointId::new(3),
                EventKind::Access {
                    use_id: UseId::new(0),
                },
                EventSource::none(),
            ),
        ],
        vec![Use {
            id: UseId::new(0),
            point: PointId::new(3),
            place: PlaceId::new(0),
            kind: UseKind::Read,
            definition: false,
        }],
    )
}

fn unreachable_use_problem() -> BorrowProblem {
    let mut problem = single_use_problem_parts_without_use();
    problem.blocks.push(CfgBlock::new(
        BlockId::new(1),
        PointId::new(3),
        PointId::new(4),
        vec![EventId::new(0)],
    ));
    problem.points.extend([
        ProgramPoint::new(PointId::new(3), BlockId::new(1), 0),
        ProgramPoint::new(PointId::new(4), BlockId::new(1), 1),
    ]);
    problem.events.push(Event::new(
        EventId::new(0),
        PointId::new(4),
        EventKind::Access {
            use_id: UseId::new(0),
        },
        EventSource::none(),
    ));
    problem.uses.push(Use {
        id: UseId::new(0),
        point: PointId::new(4),
        place: PlaceId::new(0),
        kind: UseKind::Read,
        definition: false,
    });
    problem.exits.push(BlockId::new(1));
    ensure_terminal_events(&mut problem);
    BorrowProblem::new(problem).expect("unreachable-use fixture should validate")
}

fn single_use_problem_parts_without_use() -> BorrowProblemParts {
    BorrowProblemParts {
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
            vec![],
        )],
        entry: BlockId::new(0),
        exits: vec![BlockId::new(0)],
        places: vec![Place::new(PlaceId::new(0), BindingId::new(0), Vec::new())],
        origins: vec![ValueOrigin::fresh(ValueOriginId::new(0))],
        ..BorrowProblemParts::default()
    }
}

fn problem(
    points: Vec<ProgramPoint>,
    blocks: Vec<CfgBlock>,
    edges: Vec<CfgEdge>,
    exits: Vec<BlockId>,
    events: Vec<Event>,
    uses: Vec<Use>,
) -> BorrowProblem {
    let mut parts = BorrowProblemParts {
        bindings: vec![Binding::synthetic(BindingId::new(0))],
        points,
        blocks,
        edges,
        entry: BlockId::new(0),
        exits,
        places: vec![Place::new(PlaceId::new(0), BindingId::new(0), Vec::new())],
        origins: vec![ValueOrigin::fresh(ValueOriginId::new(0))],
        uses,
        events,
        ..BorrowProblemParts::default()
    };
    ensure_terminal_events(&mut parts);
    BorrowProblem::new(parts).expect("last-use fixture should validate")
}

fn ensure_terminal_events(parts: &mut BorrowProblemParts) {
    let outgoing = parts
        .edges
        .iter()
        .map(|edge| edge.from)
        .collect::<BTreeSet<_>>();
    for block_index in 0..parts.blocks.len() {
        let block_id = parts.blocks[block_index].id;
        if outgoing.contains(&block_id) {
            continue;
        }
        let has_terminal_terminator = parts.blocks[block_index]
            .events
            .last()
            .and_then(|event_id| parts.events.get(event_id.index()))
            .is_some_and(|event| {
                matches!(
                    event.kind,
                    EventKind::Terminator {
                        kind: TerminatorEventKind::Return
                            | TerminatorEventKind::ReturnSuccess
                            | TerminatorEventKind::ReturnError
                            | TerminatorEventKind::RuntimeFailure
                            | TerminatorEventKind::AssertFailure
                    }
                )
            });
        if has_terminal_terminator {
            continue;
        }
        let point = parts.blocks[block_index].exit;
        let event_id = EventId::new(parts.events.len() as u32);
        parts.events.push(Event::new(
            event_id,
            point,
            EventKind::Terminator {
                kind: TerminatorEventKind::Return,
            },
            EventSource::none(),
        ));
        let block = &mut parts.blocks[block_index];
        let mut event_ids = block.events.to_vec();
        event_ids.push(event_id);
        block.events = event_ids.into_boxed_slice();
    }
}
