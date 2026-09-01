//! Deterministic bounded generation of normalized operational-oracle problems.
//!
//! WHAT: extends the original seeded fixtures with independent bounded choices for every
//!       operational problem family used by Boracle campaigns.
//! WHY: later reduction and metamorphic layers need one reproducible source of complete,
//!      validated input rather than a second test-only generator.

use crate::compiler_frontend::analysis::borrow_checker::problem::{
    AccessKind, AggregateField, Binding, BindingId, BlockId, BorrowProblem, BorrowProblemParts,
    Call, CallArgument, CallEffect, CallId, CallResult, CallResultProvenance, CfgBlock, CfgEdge,
    Event, EventId, EventKind, EventSource, KillReason, Loan, LoanId, OriginKind, Place, PlaceId,
    PointId, ProgramPoint, ProjectionElem, RebindValue, TerminatorEventKind, Use, UseId, UseKind,
    ValueOrigin, ValueOriginId,
};

const DIGIT_COUNT: u32 = 11;
const DIGIT_RADIX: u32 = 2;

/// Number of deterministic shapes before the seed-to-shape mapping repeats.
///
/// The eleven binary digits correspond, from least to most significant, to block shape, branch
/// shape, back-edge shape, fresh origins, aliases, copies, projections, aggregates, calls,
/// cleanup (loan kinds and scope exits) and conflicts.
pub(crate) const GENERATED_SHAPE_COUNT: u32 = 1_u32 << DIGIT_COUNT;

/// One generated problem together with the inputs that selected it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GeneratedProblem {
    pub(crate) seed: u32,
    pub(crate) cyclic: bool,
    pub(crate) problem: BorrowProblem,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ShapeDigits {
    block_shape: u32,
    branch_shape: u32,
    back_edge_shape: u32,
    fresh_shape: u32,
    alias_shape: u32,
    copy_shape: u32,
    projection_shape: u32,
    aggregate_shape: u32,
    call_shape: u32,
    cleanup_shape: u32,
    conflict_shape: u32,
}

impl ShapeDigits {
    fn from_seed(seed: u32) -> Self {
        let mut remaining = seed % GENERATED_SHAPE_COUNT;
        let block_shape = take_digit(&mut remaining);
        let branch_shape = take_digit(&mut remaining);
        let back_edge_shape = take_digit(&mut remaining);
        let fresh_shape = take_digit(&mut remaining);
        let alias_shape = take_digit(&mut remaining);
        let copy_shape = take_digit(&mut remaining);
        let projection_shape = take_digit(&mut remaining);
        let aggregate_shape = take_digit(&mut remaining);
        let call_shape = take_digit(&mut remaining);
        let cleanup_shape = take_digit(&mut remaining);
        let conflict_shape = take_digit(&mut remaining);

        Self {
            block_shape,
            branch_shape,
            back_edge_shape,
            fresh_shape,
            alias_shape,
            copy_shape,
            projection_shape,
            aggregate_shape,
            call_shape,
            cleanup_shape,
            conflict_shape,
        }
    }
}

fn take_digit(remaining: &mut u32) -> u32 {
    let digit = *remaining % DIGIT_RADIX;
    *remaining /= DIGIT_RADIX;
    digit
}

/// Generate one deterministic normalized problem.
///
/// Raw seeds are retained in [`GeneratedProblem::seed`], while the normalized shape repeats every
/// [`GENERATED_SHAPE_COUNT`] seeds. Every problem includes the core provenance, control-flow,
/// aggregate, call and scope-exit vocabulary; the conflict digit selects either the original
/// explicit kill or a live-loan conflict probe. Digits select bounded variations within those
/// families so every generated result remains executable by the operational oracle.
pub(crate) fn generated_problem(seed: u32, cyclic: bool) -> GeneratedProblem {
    let digits = ShapeDigits::from_seed(seed);
    let projection = if digits.projection_shape == 0 {
        ProjectionElem::Field(0)
    } else {
        ProjectionElem::FixedIndex(0)
    };
    let copied_place = if digits.copy_shape == 0 {
        PlaceId::new(1)
    } else {
        PlaceId::new(7)
    };

    let bindings = (0..8)
        .map(|id| Binding::synthetic(BindingId::new(id)))
        .collect();
    let places = vec![
        Place::new(PlaceId::new(0), BindingId::new(0), Vec::new()),
        Place::new(PlaceId::new(1), BindingId::new(1), Vec::new()),
        Place::new(PlaceId::new(2), BindingId::new(0), vec![projection]),
        Place::new(PlaceId::new(3), BindingId::new(3), Vec::new()),
        Place::new(PlaceId::new(4), BindingId::new(4), Vec::new()),
        Place::new(PlaceId::new(5), BindingId::new(5), Vec::new()),
        Place::new(PlaceId::new(6), BindingId::new(6), Vec::new()),
        Place::new(PlaceId::new(7), BindingId::new(7), Vec::new()),
    ];
    let block_count = if cyclic || digits.block_shape == 1 {
        4
    } else {
        3
    };
    let mut builder = ProblemBuilder::new(bindings, places, block_count);

    let source_origin = builder.origin(OriginKind::Fresh);
    builder.event(
        BlockId::new(0),
        EventKind::Fresh {
            destination: PlaceId::new(0),
            origin: source_origin,
        },
    );

    let copy_origin = builder.origin(OriginKind::Copy(vec![source_origin].into_boxed_slice()));
    builder.event(
        BlockId::new(0),
        EventKind::Copy {
            source: PlaceId::new(0),
            destination: copied_place,
            origin: copy_origin,
        },
    );

    let rebound_origin = builder.origin(OriginKind::Fresh);
    let rebind_block = if cyclic {
        BlockId::new(1)
    } else {
        BlockId::new(0)
    };
    let projection_origin = builder.origin(OriginKind::Projection {
        source: source_origin,
        projection,
    });
    builder.event(
        BlockId::new(0),
        EventKind::Projection {
            source: PlaceId::new(0),
            destination: PlaceId::new(2),
            origin: projection_origin,
        },
    );

    builder.event(
        rebind_block,
        EventKind::Rebind {
            destination: PlaceId::new(0),
            value: RebindValue::Fresh(rebound_origin),
        },
    );

    if digits.fresh_shape == 1 {
        let extra_origin = builder.origin(OriginKind::Fresh);
        builder.event(
            BlockId::new(0),
            EventKind::Fresh {
                destination: PlaceId::new(6),
                origin: extra_origin,
            },
        );
    }

    let aggregate_origin = builder.origin(OriginKind::Fresh);
    let aggregate_fields = if digits.aggregate_shape == 0 {
        vec![AggregateField {
            projection: ProjectionElem::Field(0),
            source: PlaceId::new(0),
        }]
    } else {
        vec![
            AggregateField {
                projection: ProjectionElem::Field(0),
                source: PlaceId::new(0),
            },
            AggregateField {
                projection: ProjectionElem::Field(1),
                source: copied_place,
            },
        ]
    };
    builder.event(
        BlockId::new(0),
        EventKind::Aggregate {
            destination: PlaceId::new(3),
            origin: aggregate_origin,
            fields: aggregate_fields.into_boxed_slice(),
        },
    );

    builder.parts.calls.push(Call {
        id: CallId::new(0),
        label: "generated-call".to_string(),
    });
    let call_argument = builder.call_argument(BlockId::new(0), CallId::new(0));
    let call_result_origin = builder.origin(OriginKind::CallResult {
        call: CallId::new(0),
        provenance: if digits.call_shape == 0 {
            CallResultProvenance::Fresh
        } else {
            CallResultProvenance::AliasParams(vec![0].into_boxed_slice())
        },
    });
    builder.event(
        BlockId::new(0),
        EventKind::CallEffect(CallEffect {
            call: CallId::new(0),
            arguments: vec![call_argument].into_boxed_slice(),
            result: Some(CallResult {
                place: PlaceId::new(4),
                origin: call_result_origin,
            }),
        }),
    );
    builder.access(BlockId::new(0), PlaceId::new(4), UseKind::Write, true);
    builder.access(BlockId::new(0), PlaceId::new(4), UseKind::Read, false);

    let alias_kind = if digits.alias_shape == 0 {
        EventKind::AliasFromPlace {
            source: PlaceId::new(0),
            destination: PlaceId::new(5),
        }
    } else {
        EventKind::ExclusiveAliasFromPlace {
            source: PlaceId::new(0),
            destination: PlaceId::new(5),
        }
    };
    builder.event(BlockId::new(0), alias_kind);

    let loan_kind = if digits.cleanup_shape == 0 {
        AccessKind::Shared
    } else {
        AccessKind::Exclusive
    };
    let loan = builder.loan_issue(
        BlockId::new(0),
        PlaceId::new(0),
        PlaceId::new(5),
        loan_kind,
        rebound_origin,
    );
    if digits.conflict_shape == 1 {
        // WHY: The holder read follows the exclusive owner access so the capability interval spans
        // that earlier access, which is the ordinary conflict both solvers should report.
        builder.access(BlockId::new(0), PlaceId::new(0), UseKind::Write, false);
        builder.access(BlockId::new(0), PlaceId::new(5), UseKind::Read, false);
    } else {
        builder.loan_kill(BlockId::new(0), loan);
    }
    let scope_bindings = if digits.cleanup_shape == 0 {
        vec![BindingId::new(5)]
    } else {
        vec![BindingId::new(4), BindingId::new(5)]
    };
    builder.event(
        BlockId::new(0),
        EventKind::ScopeExit {
            bindings: scope_bindings.into_boxed_slice(),
        },
    );

    let block_zero_targets = if digits.block_shape == 0 {
        vec![BlockId::new(1), BlockId::new(2)]
    } else {
        vec![BlockId::new(1), BlockId::new(2), BlockId::new(3)]
    };
    builder.event(
        BlockId::new(0),
        EventKind::Terminator {
            kind: TerminatorEventKind::Branch {
                targets: block_zero_targets.clone().into_boxed_slice(),
            },
        },
    );

    if digits.branch_shape == 1 {
        builder.event(
            BlockId::new(1),
            EventKind::ReactiveObserve {
                place: copied_place,
            },
        );
    }
    let block_one_terminator = if cyclic {
        if digits.back_edge_shape == 0 {
            TerminatorEventKind::Branch {
                targets: vec![BlockId::new(1), BlockId::new(2)].into_boxed_slice(),
            }
        } else {
            TerminatorEventKind::Branch {
                targets: vec![BlockId::new(2), BlockId::new(3)].into_boxed_slice(),
            }
        }
    } else if digits.branch_shape == 0 {
        TerminatorEventKind::Return
    } else {
        TerminatorEventKind::Jump {
            target: if digits.block_shape == 0 {
                BlockId::new(2)
            } else {
                BlockId::new(3)
            },
        }
    };
    builder.event(
        BlockId::new(1),
        EventKind::Terminator {
            kind: block_one_terminator,
        },
    );

    let block_two_terminator = if digits.back_edge_shape == 0 {
        TerminatorEventKind::Return
    } else {
        TerminatorEventKind::ReturnSuccess
    };
    builder.event(
        BlockId::new(2),
        EventKind::Terminator {
            kind: block_two_terminator,
        },
    );

    if cyclic || digits.block_shape == 1 {
        let block_three_terminator = if cyclic && digits.back_edge_shape == 1 {
            TerminatorEventKind::Jump {
                target: BlockId::new(1),
            }
        } else {
            TerminatorEventKind::Return
        };
        builder.event(
            BlockId::new(3),
            EventKind::Terminator {
                kind: block_three_terminator,
            },
        );
    }

    let mut edges = block_zero_targets
        .iter()
        .copied()
        .map(|target| CfgEdge::new(BlockId::new(0), target))
        .collect::<Vec<_>>();
    if cyclic {
        let targets = if digits.back_edge_shape == 0 {
            [BlockId::new(1), BlockId::new(2)]
        } else {
            [BlockId::new(2), BlockId::new(3)]
        };
        edges.extend(
            targets
                .into_iter()
                .map(|target| CfgEdge::new(BlockId::new(1), target)),
        );
        if digits.back_edge_shape == 1 {
            edges.push(CfgEdge::new(BlockId::new(3), BlockId::new(1)));
        }
    } else if digits.branch_shape == 1 {
        let target = if digits.block_shape == 0 {
            BlockId::new(2)
        } else {
            BlockId::new(3)
        };
        edges.push(CfgEdge::new(BlockId::new(1), target));
    }
    let exits = (0..block_count)
        .map(|id| BlockId::new(id as u32))
        .filter(|block| !edges.iter().any(|edge| edge.from == *block))
        .collect::<Vec<_>>();

    let problem = builder.finish(edges, exits);
    GeneratedProblem {
        seed,
        cyclic,
        problem,
    }
}

struct ProblemBuilder {
    parts: BorrowProblemParts,
    block_events: Vec<Vec<EventId>>,
    block_points: Vec<Vec<PointId>>,
}

impl ProblemBuilder {
    fn new(bindings: Vec<Binding>, places: Vec<Place>, block_count: usize) -> Self {
        Self {
            parts: BorrowProblemParts {
                bindings,
                places,
                ..BorrowProblemParts::default()
            },
            block_events: vec![Vec::new(); block_count],
            block_points: vec![Vec::new(); block_count],
        }
    }

    fn origin(&mut self, kind: OriginKind) -> ValueOriginId {
        let id = ValueOriginId::new(self.parts.origins.len() as u32);
        self.parts.origins.push(ValueOrigin::new(id, kind));
        id
    }

    fn event(&mut self, block: BlockId, kind: EventKind) -> EventId {
        let point = self.new_point(block);
        self.event_at(block, point, kind)
    }

    fn event_at(&mut self, block: BlockId, point: PointId, kind: EventKind) -> EventId {
        let id = EventId::new(self.parts.events.len() as u32);
        self.parts
            .events
            .push(Event::new(id, point, kind, EventSource::none()));
        self.block_events[block.index()].push(id);
        id
    }

    fn new_point(&mut self, block: BlockId) -> PointId {
        let id = PointId::new(self.parts.points.len() as u32);
        let ordinal = self.block_points[block.index()].len() as u32;
        self.parts
            .points
            .push(ProgramPoint::new(id, block, ordinal));
        self.block_points[block.index()].push(id);
        id
    }

    fn access(&mut self, block: BlockId, place: PlaceId, kind: UseKind, definition: bool) {
        let id = UseId::new(self.parts.uses.len() as u32);
        let point = self.new_point(block);
        self.parts.uses.push(Use {
            id,
            point,
            place,
            kind,
            definition,
        });
        self.event_at(block, point, EventKind::Access { use_id: id });
    }

    fn call_argument(&mut self, block: BlockId, call: CallId) -> CallArgument {
        let use_id = UseId::new(self.parts.uses.len() as u32);
        let point = self.new_point(block);
        let argument = CallArgument {
            place: PlaceId::new(0),
            access: AccessKind::Shared,
            use_id,
        };
        self.parts.uses.push(Use {
            id: use_id,
            point,
            place: argument.place,
            kind: UseKind::Read,
            definition: false,
        });
        self.event_at(
            block,
            point,
            EventKind::CallArgument {
                call,
                index: 0,
                argument: argument.clone(),
            },
        );

        argument
    }

    fn loan_issue(
        &mut self,
        block: BlockId,
        place: PlaceId,
        holder: PlaceId,
        kind: AccessKind,
        origin: ValueOriginId,
    ) -> LoanId {
        let id = LoanId::new(self.parts.loans.len() as u32);
        let point = self.new_point(block);
        self.parts.loans.push(Loan {
            id,
            kind,
            issued_at: point,
            place,
            origins: vec![origin].into_boxed_slice(),
            holders: vec![holder].into_boxed_slice(),
            uses: Box::new([]),
            kills: Box::new([]),
        });
        self.event_at(block, point, EventKind::LoanIssue { loan: id });
        id
    }

    fn loan_kill(&mut self, block: BlockId, loan: LoanId) {
        let point = self.new_point(block);
        self.parts.loans[loan.index()].kills = vec![point].into_boxed_slice();
        self.event_at(
            block,
            point,
            EventKind::LoanKill {
                loan,
                reason: KillReason::Explicit,
            },
        );
    }

    fn finish(mut self, edges: Vec<CfgEdge>, exits: Vec<BlockId>) -> BorrowProblem {
        let blocks = self
            .block_events
            .iter()
            .zip(&self.block_points)
            .enumerate()
            .map(|(index, (events, points))| {
                let entry = points
                    .first()
                    .copied()
                    .expect("generated CFG block should have an entry point");
                let exit = points
                    .last()
                    .copied()
                    .expect("generated CFG block should have an exit point");
                CfgBlock::new(BlockId::new(index as u32), entry, exit, events.clone())
            })
            .collect();
        self.parts.blocks = blocks;
        self.parts.edges = edges;
        self.parts.entry = BlockId::new(0);
        self.parts.exits = exits;
        BorrowProblem::new(self.parts).expect("generated problem should validate")
    }
}
