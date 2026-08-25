//! Small deterministic problem constructors used only by the Phase 2 tests.

use super::super::{Binding, BindingId, BlockId, EventId};
use super::super::{
    BorrowProblemParts, CfgBlock, CfgEdge, Event, EventKind, EventSource, OriginKind, Place,
    PlaceId, PointId, ProgramPoint, ProjectionElem, RebindValue, Use, UseId, UseKind, ValueOrigin,
    ValueOriginId,
};

pub(crate) fn place(id: u32, root: u32, projections: Vec<ProjectionElem>) -> Place {
    Place::new(PlaceId::new(id), BindingId::new(root), projections)
}

pub(crate) fn single_block(
    places: Vec<Place>,
    origins: Vec<ValueOrigin>,
    uses: Vec<Use>,
    event_kinds: Vec<EventKind>,
) -> BorrowProblemParts {
    let points = (0..=(event_kinds.len() as u32 + 1))
        .map(|ordinal| ProgramPoint::new(PointId::new(ordinal), BlockId::new(0), ordinal))
        .collect::<Vec<_>>();
    let events = event_kinds
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
    let event_ids: Vec<EventId> = events.iter().map(|event| event.id).collect();

    BorrowProblemParts {
        bindings: bindings_for_places(&places),
        points,
        blocks: vec![CfgBlock::new(
            BlockId::new(0),
            PointId::new(0),
            PointId::new(event_ids.len() as u32 + 1),
            event_ids,
        )],
        edges: Vec::new(),
        entry: BlockId::new(0),
        exits: vec![BlockId::new(0)],
        places,
        origins,
        loans: Vec::new(),
        uses,
        calls: Vec::new(),
        events,
    }
}

fn bindings_for_places(places: &[Place]) -> Vec<Binding> {
    let Some(max_root) = places.iter().map(|place| place.root.raw()).max() else {
        return Vec::new();
    };

    (0..=max_root)
        .map(|root| Binding::synthetic(BindingId::new(root)))
        .collect()
}

pub(crate) fn empty() -> BorrowProblemParts {
    single_block(
        vec![place(0, 0, Vec::new())],
        vec![ValueOrigin::fresh(ValueOriginId::new(0))],
        Vec::new(),
        Vec::new(),
    )
}

pub(crate) fn copy() -> BorrowProblemParts {
    single_block(
        vec![place(0, 0, Vec::new()), place(1, 1, Vec::new())],
        vec![
            ValueOrigin::fresh(ValueOriginId::new(0)),
            ValueOrigin::new(
                ValueOriginId::new(1),
                OriginKind::Copy(vec![ValueOriginId::new(0)].into_boxed_slice()),
            ),
        ],
        vec![Use {
            id: UseId::new(0),
            point: PointId::new(3),
            place: PlaceId::new(0),
            kind: UseKind::Read,
        }],
        vec![
            EventKind::Fresh {
                destination: PlaceId::new(0),
                origin: ValueOriginId::new(0),
            },
            EventKind::Copy {
                source: PlaceId::new(0),
                destination: PlaceId::new(1),
                origin: ValueOriginId::new(1),
            },
            EventKind::Access {
                use_id: UseId::new(0),
            },
        ],
    )
}

pub(crate) fn old_alias_after_rebind() -> BorrowProblemParts {
    single_block(
        vec![place(0, 0, Vec::new()), place(1, 1, Vec::new())],
        vec![
            ValueOrigin::fresh(ValueOriginId::new(0)),
            ValueOrigin::fresh(ValueOriginId::new(1)),
        ],
        vec![Use {
            id: UseId::new(0),
            point: PointId::new(4),
            place: PlaceId::new(1),
            kind: UseKind::Read,
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
            EventKind::Rebind {
                destination: PlaceId::new(0),
                value: RebindValue::Fresh(ValueOriginId::new(1)),
            },
            EventKind::Access {
                use_id: UseId::new(0),
            },
        ],
    )
}

pub(crate) fn branch_join() -> BorrowProblemParts {
    let points = vec![
        ProgramPoint::new(PointId::new(0), BlockId::new(0), 0),
        ProgramPoint::new(PointId::new(1), BlockId::new(0), 1),
        ProgramPoint::new(PointId::new(2), BlockId::new(1), 0),
        ProgramPoint::new(PointId::new(3), BlockId::new(1), 1),
        ProgramPoint::new(PointId::new(4), BlockId::new(2), 0),
        ProgramPoint::new(PointId::new(5), BlockId::new(2), 1),
        ProgramPoint::new(PointId::new(6), BlockId::new(3), 0),
        ProgramPoint::new(PointId::new(7), BlockId::new(3), 1),
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
            PointId::new(3),
            EventKind::Rebind {
                destination: PlaceId::new(0),
                value: RebindValue::Fresh(ValueOriginId::new(1)),
            },
            EventSource::none(),
        ),
        Event::new(
            EventId::new(2),
            PointId::new(5),
            EventKind::Rebind {
                destination: PlaceId::new(0),
                value: RebindValue::Fresh(ValueOriginId::new(2)),
            },
            EventSource::none(),
        ),
        Event::new(
            EventId::new(3),
            PointId::new(7),
            EventKind::Alias {
                source: PlaceId::new(0),
                destination: PlaceId::new(1),
                origins: vec![ValueOriginId::new(3)].into_boxed_slice(),
            },
            EventSource::none(),
        ),
    ];

    BorrowProblemParts {
        bindings: vec![
            Binding::synthetic(BindingId::new(0)),
            Binding::synthetic(BindingId::new(1)),
        ],
        points,
        blocks: vec![
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
            CfgBlock::new(
                BlockId::new(2),
                PointId::new(4),
                PointId::new(5),
                vec![EventId::new(2)],
            ),
            CfgBlock::new(
                BlockId::new(3),
                PointId::new(6),
                PointId::new(7),
                vec![EventId::new(3)],
            ),
        ],
        edges: vec![
            CfgEdge::new(BlockId::new(0), BlockId::new(1)),
            CfgEdge::new(BlockId::new(0), BlockId::new(2)),
            CfgEdge::new(BlockId::new(1), BlockId::new(3)),
            CfgEdge::new(BlockId::new(2), BlockId::new(3)),
        ],
        entry: BlockId::new(0),
        exits: vec![BlockId::new(3)],
        places: vec![place(0, 0, Vec::new()), place(1, 1, Vec::new())],
        origins: vec![
            ValueOrigin::fresh(ValueOriginId::new(0)),
            ValueOrigin::fresh(ValueOriginId::new(1)),
            ValueOrigin::fresh(ValueOriginId::new(2)),
            ValueOrigin::new(
                ValueOriginId::new(3),
                OriginKind::Join(
                    vec![ValueOriginId::new(1), ValueOriginId::new(2)].into_boxed_slice(),
                ),
            ),
        ],
        loans: Vec::new(),
        uses: Vec::new(),
        calls: Vec::new(),
        events,
    }
}

pub(crate) fn loop_with_rebind() -> BorrowProblemParts {
    let points = vec![
        ProgramPoint::new(PointId::new(0), BlockId::new(0), 0),
        ProgramPoint::new(PointId::new(1), BlockId::new(0), 1),
        ProgramPoint::new(PointId::new(2), BlockId::new(1), 0),
        ProgramPoint::new(PointId::new(3), BlockId::new(1), 1),
        ProgramPoint::new(PointId::new(4), BlockId::new(2), 0),
        ProgramPoint::new(PointId::new(5), BlockId::new(2), 1),
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
            PointId::new(3),
            EventKind::Rebind {
                destination: PlaceId::new(0),
                value: RebindValue::Fresh(ValueOriginId::new(1)),
            },
            EventSource::none(),
        ),
    ];

    BorrowProblemParts {
        bindings: vec![Binding::synthetic(BindingId::new(0))],
        points,
        blocks: vec![
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
            CfgBlock::new(
                BlockId::new(2),
                PointId::new(4),
                PointId::new(5),
                Vec::new(),
            ),
        ],
        edges: vec![
            CfgEdge::new(BlockId::new(0), BlockId::new(1)),
            CfgEdge::new(BlockId::new(1), BlockId::new(1)),
            CfgEdge::new(BlockId::new(1), BlockId::new(2)),
        ],
        entry: BlockId::new(0),
        exits: vec![BlockId::new(2)],
        places: vec![place(0, 0, Vec::new())],
        origins: vec![
            ValueOrigin::fresh(ValueOriginId::new(0)),
            ValueOrigin::fresh(ValueOriginId::new(1)),
        ],
        loans: Vec::new(),
        uses: Vec::new(),
        calls: Vec::new(),
        events,
    }
}

pub(crate) fn same_statement_access_order() -> BorrowProblemParts {
    let points = vec![
        ProgramPoint::new(PointId::new(0), BlockId::new(0), 0),
        ProgramPoint::new(PointId::new(1), BlockId::new(0), 1),
    ];
    let events = vec![
        Event::new(
            EventId::new(0),
            PointId::new(1),
            EventKind::Access {
                use_id: UseId::new(0),
            },
            EventSource::none(),
        ),
        Event::new(
            EventId::new(1),
            PointId::new(1),
            EventKind::Access {
                use_id: UseId::new(1),
            },
            EventSource::none(),
        ),
    ];

    BorrowProblemParts {
        bindings: vec![Binding::synthetic(BindingId::new(0))],
        points,
        blocks: vec![CfgBlock::new(
            BlockId::new(0),
            PointId::new(0),
            PointId::new(1),
            vec![EventId::new(0), EventId::new(1)],
        )],
        edges: Vec::new(),
        entry: BlockId::new(0),
        exits: vec![BlockId::new(0)],
        places: vec![
            place(0, 0, vec![ProjectionElem::Field(0)]),
            place(1, 0, vec![ProjectionElem::Field(1)]),
        ],
        origins: vec![ValueOrigin::fresh(ValueOriginId::new(0))],
        loans: Vec::new(),
        uses: vec![
            Use {
                id: UseId::new(0),
                point: PointId::new(1),
                place: PlaceId::new(0),
                kind: UseKind::Read,
            },
            Use {
                id: UseId::new(1),
                point: PointId::new(1),
                place: PlaceId::new(1),
                kind: UseKind::Read,
            },
        ],
        calls: Vec::new(),
        events,
    }
}

pub(crate) fn field_accesses() -> BorrowProblemParts {
    single_block(
        vec![
            place(0, 0, Vec::new()),
            place(1, 0, vec![ProjectionElem::Field(0)]),
            place(2, 0, vec![ProjectionElem::Field(1)]),
        ],
        vec![ValueOrigin::fresh(ValueOriginId::new(0))],
        vec![
            Use {
                id: UseId::new(0),
                point: PointId::new(2),
                place: PlaceId::new(1),
                kind: UseKind::Read,
            },
            Use {
                id: UseId::new(1),
                point: PointId::new(3),
                place: PlaceId::new(2),
                kind: UseKind::Read,
            },
        ],
        vec![
            EventKind::Fresh {
                destination: PlaceId::new(0),
                origin: ValueOriginId::new(0),
            },
            EventKind::Access {
                use_id: UseId::new(0),
            },
            EventKind::Access {
                use_id: UseId::new(1),
            },
        ],
    )
}
