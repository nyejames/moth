//! Origin relation rows extracted from one solved origin problem.
//!
//! WHAT: pins the solver-owned relation construction on solved fixtures: identity stays
//!       `ValueOriginId` equality, projection and containment rows keep their directional
//!       evidence, explicit copies publish positive disjointness, unknown provenance stays
//!       conservative and mixed traces relate nothing beyond identity.
//! WHY:  origin overlap must have exactly one owner, and every conflict witness must name the
//!       typed relation fact that produced its decision.

use super::super::{
    DisjointReason, OriginDisjointEvidence, OriginOverlapDecision, OriginOverlapEvidence,
    OriginRegistration, OriginRelation, OriginRelationEvidence, OriginRelationKind,
    OriginUnknownEvidence, PrecisionLossReason,
};
use crate::compiler_frontend::analysis::borrow_checker::problem::{
    AccessKind, AggregateField, Binding, BindingId, BlockId, BorrowProblem, BorrowProblemParts,
    Call, CallArgument, CallEffect, CallId, CallResult, CallResultProvenance,
    CallResultUnknownReason, CfgBlock, Event, EventId, EventKind, EventSource, Loan, LoanId,
    OriginKind, Place, PlaceId, PlaceOverlap, PointId, ProgramPoint, ProjectionElem, Use, UseId,
    UseKind, ValueOrigin, ValueOriginId,
};

#[test]
fn mixed_alias_and_slot_generations_stay_unrelated_in_relations() {
    let problem = super::mixed_binding_problem();
    let solution = super::super::OriginSolver::solve(&problem).expect("mixed binding should solve");
    let relations = solution.relations();

    // The mixed write at event 7 publishes the union {0, 2}: origin 0 is the preserved alias
    // generation and origin 2 the replaced slot generation. No row may relate them, and the
    // later mixed write at event 14 must keep the same rule for {0, 3}.
    let preserved = ValueOriginId::new(0);
    for slot in [ValueOriginId::new(2), ValueOriginId::new(3)] {
        assert!(
            relations
                .rows()
                .iter()
                .all(|row| !row_pair_is(row, preserved, slot)),
            "mixed generations {preserved:?} and {slot:?} must not carry a relation row"
        );
        let (left, right) = normalized(preserved, slot);
        assert_eq!(
            relations
                .query_overlap(&[preserved], &[slot])
                .expect("mixed query should validate"),
            OriginOverlapDecision::Disjoint(OriginDisjointEvidence {
                left,
                right,
                reason: DisjointReason::DifferentFreshGenerations,
                relation: None,
            }),
            "preserved alias and slot generations must stay provably disjoint"
        );
        assert!(
            relations
                .mixed_generation_sets()
                .iter()
                .any(|set| set.as_ref() == [preserved, slot]),
            "mixed write publishing {preserved:?} alongside {slot:?} must record its union as \
             MixedBindingMode precision loss"
        );
    }
}

#[test]
fn old_alias_and_fresh_rebind_generations_stay_disjoint() {
    let problem = super::fresh_rebind_property_problem();
    let solution = super::super::OriginSolver::solve(&problem).expect("fresh rebind should solve");
    let relations = solution.relations();

    // The alias at event 3 preserves origin 0 at binding 1; the fresh rebind at event 5
    // replaces binding 0's generation with origin 1. The generations share no row and stay
    // disjoint even though the alias previously observed the rebound binding.
    assert!(relations.rows().iter().all(|row| !row_pair_is(
        row,
        ValueOriginId::new(0),
        ValueOriginId::new(1)
    )));
    assert_eq!(
        relations
            .query_overlap(&[ValueOriginId::new(0)], &[ValueOriginId::new(1)])
            .expect("rebind query should validate"),
        OriginOverlapDecision::Disjoint(OriginDisjointEvidence {
            left: ValueOriginId::new(0),
            right: ValueOriginId::new(1),
            reason: DisjointReason::DifferentFreshGenerations,
            relation: None,
        })
    );
}

#[test]
fn copy_independence_survives_branch_joins_with_positive_evidence() {
    let generated = super::generated_problem(5, true);
    let problem = &generated.problem;
    let mut predecessors = problem
        .control_flow()
        .edges
        .iter()
        .filter(|edge| edge.to == BlockId::new(3))
        .map(|edge| edge.from)
        .collect::<Vec<_>>();
    predecessors.sort_unstable();
    assert_eq!(
        predecessors,
        vec![BlockId::new(0), BlockId::new(1)],
        "generated seed 5 must reconverge at block 3 from both branch arms"
    );
    assert!(
        problem
            .control_flow()
            .edges
            .iter()
            .any(|edge| edge.from == BlockId::new(3) && edge.to == BlockId::new(1)),
        "generated seed 5 must retain the loop back edge 3 -> 1"
    );
    let solution = super::super::OriginSolver::solve(problem).expect("cyclic copy should solve");
    let relations = solution.relations();

    // The copy at event 1 reads origin 0 and publishes the independent generation 1 across a
    // branch join and a loop back edge. The copy row must survive as positive evidence.
    let copy_rows = relations
        .rows()
        .iter()
        .filter(|row| matches!(row.kind, OriginRelationKind::CopyCorrespondence { .. }))
        .collect::<Vec<_>>();
    assert_eq!(copy_rows.len(), 1, "expected one copy correspondence row");
    assert!(row_pair_is(
        copy_rows[0],
        ValueOriginId::new(0),
        ValueOriginId::new(1)
    ));

    assert_eq!(
        relations
            .query_overlap(&[ValueOriginId::new(0)], &[ValueOriginId::new(1)])
            .expect("copy query should validate"),
        OriginOverlapDecision::Disjoint(OriginDisjointEvidence {
            left: ValueOriginId::new(0),
            right: ValueOriginId::new(1),
            reason: DisjointReason::ExplicitCopy,
            relation: Some(copy_rows[0].evidence),
        })
    );
}

#[test]
fn field_siblings_stay_disjoint_under_containment_rows() {
    let problem = sibling_fields_problem();
    let solution =
        super::super::OriginSolver::solve(&problem).expect("sibling fields should solve");
    let relations = solution.relations();
    let aggregate = ValueOriginId::new(1);
    let first = ValueOriginId::new(0);
    let second = ValueOriginId::new(2);

    // Each stored child keeps one directional containment row; the sibling pair inherits
    // nothing through their parent.
    assert!(relations.rows().iter().any(|row| {
        matches!(
            row.kind,
            OriginRelationKind::AggregateChild {
                projection: ProjectionElem::Field(0)
            }
        ) && row.left == aggregate
            && row.right == first
    }));
    assert!(relations.rows().iter().any(|row| {
        matches!(
            row.kind,
            OriginRelationKind::AggregateChild {
                projection: ProjectionElem::Field(1)
            }
        ) && row.left == aggregate
            && row.right == second
    }));
    assert!(
        relations
            .rows()
            .iter()
            .all(|row| !row_pair_is(row, first, second))
    );

    assert_eq!(
        relations
            .query_overlap(&[first], &[second])
            .expect("sibling query should validate"),
        OriginOverlapDecision::Disjoint(OriginDisjointEvidence {
            left: first,
            right: second,
            reason: DisjointReason::DifferentFreshGenerations,
            relation: None,
        })
    );
}

#[test]
fn base_and_child_generations_overlap_through_typed_rows() {
    let problem = super::aggregate_problem();
    let solution = super::super::OriginSolver::solve(&problem).expect("aggregate should solve");
    let relations = solution.relations();

    // The stored field generation overlaps its aggregate parent through a containment row.
    assert!(matches!(
        relations
            .query_overlap(&[ValueOriginId::new(1)], &[ValueOriginId::new(0)])
            .expect("aggregate child query should validate"),
        OriginOverlapDecision::Overlap(OriginOverlapEvidence::Relation {
            kind: OriginRelationKind::AggregateChild { .. },
            ..
        })
    ));

    // The hand-authored projection row derives origin 2 from its actual source origin 1.
    assert!(matches!(
        relations
            .query_overlap(&[ValueOriginId::new(2)], &[ValueOriginId::new(1)])
            .expect("projection query should validate"),
        OriginOverlapDecision::Overlap(OriginOverlapEvidence::Relation {
            kind: OriginRelationKind::Projection { .. },
            ..
        })
    ));

    // Unrelated generations stay disjoint: old overlap reported false for this pair.
    assert!(matches!(
        relations
            .query_overlap(&[ValueOriginId::new(2)], &[ValueOriginId::new(0)])
            .expect("unrelated projection query should validate"),
        OriginOverlapDecision::Disjoint(_)
    ));
}

#[test]
fn unknown_provenance_stays_conservatively_overlapping() {
    let problem = unknown_provenance_problem();
    let solution =
        super::super::OriginSolver::solve(&problem).expect("unknown provenance should solve");
    let relations = solution.relations();
    let fresh = ValueOriginId::new(0);

    for (origin, reason) in [
        (
            ValueOriginId::new(1),
            PrecisionLossReason::MissingLocalSummary,
        ),
        (
            ValueOriginId::new(2),
            PrecisionLossReason::UnknownCallResult,
        ),
    ] {
        match relations
            .query_overlap(&[origin], &[fresh])
            .expect("unknown query should validate")
        {
            OriginOverlapDecision::Unknown(evidence) => assert_eq!(evidence.reason, reason),
            decision => panic!("origin {origin:?} must stay unknown, got {decision:?}"),
        }
    }
}

#[test]
fn unknown_call_results_retain_boundary_specific_precision_reasons() {
    let problem = boundary_reason_problem();
    let solution =
        super::super::OriginSolver::solve(&problem).expect("boundary reasons should solve");
    let relations = solution.relations();

    for (origin, reason) in [
        (
            ValueOriginId::new(1),
            PrecisionLossReason::UnknownCallResult,
        ),
        (
            ValueOriginId::new(2),
            PrecisionLossReason::MissingLocalSummary,
        ),
        (
            ValueOriginId::new(3),
            PrecisionLossReason::ExternalOpaqueValue,
        ),
    ] {
        assert_eq!(
            relations
                .query_overlap(&[origin], &[ValueOriginId::new(0)])
                .expect("boundary reason query should validate"),
            OriginOverlapDecision::Unknown(OriginUnknownEvidence {
                left: vec![ValueOriginId::new(0)].into_boxed_slice(),
                right: vec![origin].into_boxed_slice(),
                reason,
                relation: None,
            })
        );
    }
}

#[test]
fn precision_dump_reports_populated_precision_loss_content() {
    // Unknown provenance registers real unknown origins with their boundary reasons, the
    // join fixture keeps its may-alias rows, and the mixed fixture publishes both mixed
    // generation unions: the dump must render that content, not only the headings.
    let unknown = super::super::OriginSolver::solve(&unknown_provenance_problem())
        .expect("unknown provenance should solve")
        .relations()
        .precision_debug_dump();
    let unknown_origins =
        precision_dump_section(&unknown, "unknown-origins:", Some("may-alias-relations:"));
    for (id, reason) in [
        (
            ValueOriginId::new(1),
            PrecisionLossReason::MissingLocalSummary,
        ),
        (
            ValueOriginId::new(2),
            PrecisionLossReason::UnknownCallResult,
        ),
    ] {
        let registration = format!("{:?}", OriginRegistration::unknown(id, reason));
        assert!(
            unknown_origins.contains(&registration),
            "precision dump must register origin {id:?} as {registration:?}, got:\n\
             {unknown_origins}"
        );
    }

    let join = super::super::OriginSolver::solve(&join_origin_problem())
        .expect("join origin should solve")
        .relations()
        .precision_debug_dump();
    let may_alias = precision_dump_section(
        &join,
        "may-alias-relations:",
        Some("mixed-generation-sets:"),
    );
    for member in [ValueOriginId::new(1), ValueOriginId::new(2)] {
        let row =
            OriginRelation::may_alias(ValueOriginId::new(3), member, PrecisionLossReason::PathJoin);
        assert!(
            may_alias.contains(&format!("{row:?}")),
            "precision dump must carry the join may-alias row {row:?}, got:\n{may_alias}"
        );
    }

    let mixed = super::super::OriginSolver::solve(&super::mixed_binding_problem())
        .expect("mixed binding should solve")
        .relations()
        .precision_debug_dump();
    let mixed_sets = precision_dump_section(&mixed, "mixed-generation-sets:", None);
    for slot in [ValueOriginId::new(2), ValueOriginId::new(3)] {
        let expected = format!("{:?}", [ValueOriginId::new(0), slot]);
        assert!(
            mixed_sets.contains(&expected),
            "precision dump must record the mixed union {expected}, got:\n{mixed_sets}"
        );
    }
}

#[test]
fn projection_with_missing_source_state_is_rejected() {
    let problem = projection_missing_source_state_problem();
    let error = super::super::OriginSolver::solve(&problem)
        .expect_err("projection with missing source state must fail");

    assert!(
        error.msg.contains("projection event")
            && error.msg.contains("EventId(0)")
            && error.msg.contains("source place PlaceId(0)")
            && error.msg.contains("destination place PlaceId(1)")
            && error.msg.contains("origin state is missing"),
        "expected the projection error to name event, places and missing origin state, got: {error:?}"
    );
}

#[test]
fn copy_with_missing_source_state_is_rejected() {
    let problem = copy_missing_source_state_problem();
    let error = super::super::OriginSolver::solve(&problem)
        .expect_err("copy with missing source state must fail");

    assert!(
        error.msg.contains("copy event")
            && error.msg.contains("EventId(0)")
            && error.msg.contains("source place PlaceId(0)")
            && error.msg.contains("destination place PlaceId(1)")
            && error.msg.contains("origin state is missing"),
        "expected the copy error to name event, places and missing origin state, got: {error:?}"
    );
}

#[test]
fn aggregate_with_missing_field_source_state_is_rejected() {
    let problem = aggregate_missing_field_source_state_problem();
    let error = super::super::OriginSolver::solve(&problem)
        .expect_err("aggregate with a missing field state must fail");

    assert!(
        error.msg.contains("aggregate event")
            && error.msg.contains("EventId(1)")
            && error.msg.contains("field source place PlaceId(2)")
            && error.msg.contains("destination place PlaceId(1)")
            && error.msg.contains("origin state is missing"),
        "expected the aggregate error to name the event, the failing field place and the \
         destination place, got: {error:?}"
    );
}

#[test]
fn empty_access_states_keep_the_conservative_conflict() {
    let problem = super::loan_conflict_problem();
    let origins = super::super::OriginSolver::solve(&problem).expect("origins should solve");
    let solution = super::super::LoanSolver::solve(&problem, &origins).expect("loans should solve");

    // The conflicting access observes no recorded generation. Old overlap treated the empty
    // set as top; the typed decision must keep that conservative conflict as unknown
    // evidence naming the loan's generation.
    let conflict = solution
        .conflicts()
        .first()
        .expect("loan conflict should exist");
    assert_eq!(
        conflict.origin_overlap,
        OriginOverlapDecision::Unknown(OriginUnknownEvidence {
            left: Vec::new().into_boxed_slice(),
            right: vec![ValueOriginId::new(0)].into_boxed_slice(),
            reason: PrecisionLossReason::MissingLocalSummary,
            relation: None,
        })
    );
}

#[test]
fn identity_witnesses_carry_the_exact_origin() {
    let problem = super::mixed_binding_problem();
    let origins = super::super::OriginSolver::solve(&problem).expect("origins should solve");
    let solution = super::super::LoanSolver::solve(&problem, &origins).expect("loans should solve");

    // Identity witnesses must name the exact shared generation: the identity origin appears
    // in the access origins and the loan origins alike, never a guessed pair.
    let identity_witnesses = solution
        .conflicts()
        .iter()
        .filter(|witness| {
            matches!(
                witness.origin_overlap,
                OriginOverlapDecision::Overlap(OriginOverlapEvidence::Identity { .. })
            )
        })
        .collect::<Vec<_>>();
    assert!(
        !identity_witnesses.is_empty(),
        "mixed binding should produce at least one identity witness"
    );
    for witness in identity_witnesses {
        let OriginOverlapDecision::Overlap(OriginOverlapEvidence::Identity { origin }) =
            witness.origin_overlap
        else {
            unreachable!("filtered above");
        };
        assert!(witness.access_origins.contains(&origin));
        assert!(witness.loan_origins.contains(&origin));
    }
}

#[test]
fn conservative_unknown_witnesses_name_the_exact_reason() {
    let problem = unknown_loan_witness_problem();
    let origins = super::super::OriginSolver::solve(&problem).expect("origins should solve");
    let solution = super::super::LoanSolver::solve(&problem, &origins).expect("loans should solve");

    let conflict = solution
        .conflicts()
        .iter()
        .find(|witness| witness.access_place == PlaceId::new(1))
        .expect("unknown-provenance conflict should exist");
    assert_eq!(conflict.overlap, PlaceOverlap::Conservative);
    match &conflict.origin_overlap {
        OriginOverlapDecision::Unknown(evidence) => {
            assert_eq!(evidence.reason, PrecisionLossReason::MissingLocalSummary);
        }
        decision => panic!("expected unknown overlap evidence, got {decision:?}"),
    }
}

#[test]
fn write_through_witnesses_name_the_path_join_reason() {
    let problem = write_through_witness_problem();
    let origins =
        super::super::OriginSolver::solve(&problem).expect("write-through origins should solve");
    let solution = super::super::LoanSolver::solve(&problem, &origins)
        .expect("write-through loans should solve");

    let conflict = solution
        .conflicts()
        .iter()
        .find(|witness| witness.access_place == PlaceId::new(0))
        .expect("write-through conflict should exist");
    match &conflict.origin_overlap {
        OriginOverlapDecision::Overlap(OriginOverlapEvidence::Relation { kind, .. }) => {
            assert_eq!(
                *kind,
                OriginRelationKind::MayAlias {
                    reason: PrecisionLossReason::PathJoin,
                }
            );
        }
        decision => panic!("expected a may-alias relation witness, got {decision:?}"),
    }
}

#[test]
fn join_origin_rows_overlap_their_members_with_a_path_reason() {
    let problem = join_origin_problem();
    let solution = super::super::OriginSolver::solve(&problem).expect("join origin should solve");
    let relations = solution.relations();
    let joined = ValueOriginId::new(3);

    for member in [ValueOriginId::new(1), ValueOriginId::new(2)] {
        assert!(relations.rows().iter().any(|row| {
            matches!(
                row.kind,
                OriginRelationKind::MayAlias {
                    reason: PrecisionLossReason::PathJoin
                }
            ) && row_pair_is(row, joined, member)
        }));
        assert!(matches!(
            relations
                .query_overlap(&[joined], &[member])
                .expect("join query should validate"),
            OriginOverlapDecision::Overlap(OriginOverlapEvidence::Relation { .. })
        ));
    }

    // The join origin is an independent generation toward everything outside its members.
    assert!(matches!(
        relations
            .query_overlap(&[joined], &[ValueOriginId::new(0)])
            .expect("join query should validate"),
        OriginOverlapDecision::Disjoint(_)
    ));
}

#[test]
fn builder_projection_rows_never_reference_the_unknown_placeholder() {
    let problem = super::hir_distinct_projection_problem();
    let solution =
        super::super::OriginSolver::solve(&problem).expect("HIR projection should solve");
    let unknown = problem
        .origins()
        .iter()
        .find(|origin| matches!(origin.kind, OriginKind::Unknown))
        .map(|origin| origin.id);

    // Builder projection rows share one Unknown placeholder as their OriginKind source.
    // Relation rows must come from solved event sources, never that placeholder.
    let projection_rows = solution
        .relations()
        .rows()
        .iter()
        .filter(|row| {
            matches!(
                row.evidence,
                OriginRelationEvidence::Projection { source, .. } if Some(source) != unknown
            )
        })
        .count();
    assert!(
        projection_rows > 0,
        "HIR projection events must publish a source-to-derived row from solved sources"
    );
}

fn row_pair_is(row: &OriginRelation, left: ValueOriginId, right: ValueOriginId) -> bool {
    (row.left == left && row.right == right) || (row.left == right && row.right == left)
}

fn normalized(left: ValueOriginId, right: ValueOriginId) -> (ValueOriginId, ValueOriginId) {
    if left <= right {
        (left, right)
    } else {
        (right, left)
    }
}

/// Slice one heading-delimited section out of a precision dump so assertions check the
/// rendered rows between headings rather than only the headings themselves.
fn precision_dump_section<'a>(dump: &'a str, heading: &str, next_heading: Option<&str>) -> &'a str {
    let start = dump
        .find(heading)
        .unwrap_or_else(|| panic!("precision dump must contain the {heading} heading"));
    let rest = &dump[start + heading.len()..];
    let end = next_heading
        .and_then(|next| rest.find(next))
        .unwrap_or(rest.len());
    &rest[..end]
}

fn sibling_fields_problem() -> BorrowProblem {
    BorrowProblem::new(super::with_return_terminator(BorrowProblemParts {
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
            (0..4).map(EventId::new).collect(),
        )],
        entry: BlockId::new(0),
        exits: vec![BlockId::new(0)],
        places: vec![
            Place::new(PlaceId::new(0), BindingId::new(0), Vec::new()),
            Place::new(PlaceId::new(1), BindingId::new(1), Vec::new()),
            Place::new(
                PlaceId::new(2),
                BindingId::new(1),
                vec![ProjectionElem::Field(0)],
            ),
            Place::new(
                PlaceId::new(3),
                BindingId::new(1),
                vec![ProjectionElem::Field(1)],
            ),
            Place::new(PlaceId::new(4), BindingId::new(2), Vec::new()),
        ],
        origins: vec![
            ValueOrigin::fresh(ValueOriginId::new(0)),
            ValueOrigin::fresh(ValueOriginId::new(1)),
            ValueOrigin::fresh(ValueOriginId::new(2)),
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
                EventKind::Fresh {
                    destination: PlaceId::new(4),
                    origin: ValueOriginId::new(2),
                },
                EventSource::none(),
            ),
            Event::new(
                EventId::new(2),
                PointId::new(3),
                EventKind::Aggregate {
                    destination: PlaceId::new(1),
                    origin: ValueOriginId::new(1),
                    fields: vec![
                        AggregateField {
                            projection: ProjectionElem::Field(0),
                            source: PlaceId::new(0),
                        },
                        AggregateField {
                            projection: ProjectionElem::Field(1),
                            source: PlaceId::new(4),
                        },
                    ]
                    .into_boxed_slice(),
                },
                EventSource::none(),
            ),
            Event::new(
                EventId::new(3),
                PointId::new(5),
                EventKind::Access {
                    use_id: UseId::new(0),
                },
                EventSource::none(),
            ),
        ],
        uses: vec![Use {
            id: UseId::new(0),
            point: PointId::new(5),
            place: PlaceId::new(2),
            kind: UseKind::Read,
            definition: false,
        }],
        ..BorrowProblemParts::default()
    }))
    .expect("sibling fields problem should validate")
}

fn boundary_reason_problem() -> BorrowProblem {
    BorrowProblem::new(super::with_return_terminator(BorrowProblemParts {
        bindings: vec![
            Binding::synthetic(BindingId::new(0)),
            Binding::synthetic(BindingId::new(1)),
            Binding::synthetic(BindingId::new(2)),
            Binding::synthetic(BindingId::new(3)),
        ],
        points: (0..=3)
            .map(|ordinal| ProgramPoint::new(PointId::new(ordinal), BlockId::new(0), ordinal))
            .collect(),
        blocks: vec![CfgBlock::new(
            BlockId::new(0),
            PointId::new(0),
            PointId::new(3),
            (0..3).map(EventId::new).collect(),
        )],
        entry: BlockId::new(0),
        exits: vec![BlockId::new(0)],
        places: vec![
            Place::new(PlaceId::new(0), BindingId::new(0), Vec::new()),
            Place::new(PlaceId::new(1), BindingId::new(1), Vec::new()),
            Place::new(PlaceId::new(2), BindingId::new(2), Vec::new()),
            Place::new(PlaceId::new(3), BindingId::new(3), Vec::new()),
        ],
        origins: vec![
            ValueOrigin::fresh(ValueOriginId::new(0)),
            ValueOrigin::new(
                ValueOriginId::new(1),
                OriginKind::CallResult {
                    call: CallId::new(0),
                    provenance: CallResultProvenance::Unknown(
                        CallResultUnknownReason::SummaryUnknown,
                    ),
                },
            ),
            ValueOrigin::new(
                ValueOriginId::new(2),
                OriginKind::CallResult {
                    call: CallId::new(1),
                    provenance: CallResultProvenance::Unknown(
                        CallResultUnknownReason::MissingSummary,
                    ),
                },
            ),
            ValueOrigin::new(
                ValueOriginId::new(3),
                OriginKind::CallResult {
                    call: CallId::new(2),
                    provenance: CallResultProvenance::Unknown(
                        CallResultUnknownReason::OpaqueExternal,
                    ),
                },
            ),
        ],
        calls: vec![
            Call {
                id: CallId::new(0),
                label: "summary-unknown".to_owned(),
            },
            Call {
                id: CallId::new(1),
                label: "missing-summary".to_owned(),
            },
            Call {
                id: CallId::new(2),
                label: "opaque-external".to_owned(),
            },
        ],
        events: vec![
            Event::new(
                EventId::new(0),
                PointId::new(1),
                EventKind::CallEffect(CallEffect {
                    call: CallId::new(0),
                    arguments: Vec::new().into_boxed_slice(),
                    result: Some(CallResult {
                        place: PlaceId::new(1),
                        origin: ValueOriginId::new(1),
                    }),
                }),
                EventSource::none(),
            ),
            Event::new(
                EventId::new(1),
                PointId::new(2),
                EventKind::CallEffect(CallEffect {
                    call: CallId::new(1),
                    arguments: Vec::new().into_boxed_slice(),
                    result: Some(CallResult {
                        place: PlaceId::new(2),
                        origin: ValueOriginId::new(2),
                    }),
                }),
                EventSource::none(),
            ),
            Event::new(
                EventId::new(2),
                PointId::new(3),
                EventKind::CallEffect(CallEffect {
                    call: CallId::new(2),
                    arguments: Vec::new().into_boxed_slice(),
                    result: Some(CallResult {
                        place: PlaceId::new(3),
                        origin: ValueOriginId::new(3),
                    }),
                }),
                EventSource::none(),
            ),
        ],
        ..BorrowProblemParts::default()
    }))
    .expect("boundary reason problem should validate")
}

fn unknown_provenance_problem() -> BorrowProblem {
    BorrowProblem::new(super::with_return_terminator(BorrowProblemParts {
        bindings: vec![Binding::synthetic(BindingId::new(0))],
        points: (0..=5)
            .map(|ordinal| ProgramPoint::new(PointId::new(ordinal), BlockId::new(0), ordinal))
            .collect(),
        blocks: vec![CfgBlock::new(
            BlockId::new(0),
            PointId::new(0),
            PointId::new(5),
            (0..4).map(EventId::new).collect(),
        )],
        entry: BlockId::new(0),
        exits: vec![BlockId::new(0)],
        places: vec![Place::new(PlaceId::new(0), BindingId::new(0), Vec::new())],
        origins: vec![
            ValueOrigin::fresh(ValueOriginId::new(0)),
            ValueOrigin::unknown(ValueOriginId::new(1)),
            ValueOrigin::new(
                ValueOriginId::new(2),
                OriginKind::CallResult {
                    call: CallId::new(0),
                    provenance: CallResultProvenance::Unknown(
                        CallResultUnknownReason::SummaryUnknown,
                    ),
                },
            ),
        ],
        calls: vec![Call {
            id: CallId::new(0),
            label: "unknown-provenance".to_owned(),
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
                    call: CallId::new(0),
                    index: 0,
                    argument: CallArgument {
                        place: PlaceId::new(0),
                        access: AccessKind::Shared,
                        use_id: UseId::new(0),
                    },
                },
                EventSource::none(),
            ),
            Event::new(
                EventId::new(2),
                PointId::new(3),
                EventKind::CallEffect(CallEffect {
                    call: CallId::new(0),
                    arguments: vec![CallArgument {
                        place: PlaceId::new(0),
                        access: AccessKind::Shared,
                        use_id: UseId::new(0),
                    }]
                    .into_boxed_slice(),
                    result: Some(CallResult {
                        place: PlaceId::new(0),
                        origin: ValueOriginId::new(2),
                    }),
                }),
                EventSource::none(),
            ),
            Event::new(
                EventId::new(3),
                PointId::new(5),
                EventKind::Access {
                    use_id: UseId::new(1),
                },
                EventSource::none(),
            ),
        ],
        ..BorrowProblemParts::default()
    }))
    .expect("unknown provenance problem should validate")
}

fn unknown_loan_witness_problem() -> BorrowProblem {
    BorrowProblem::new(super::with_return_terminator(BorrowProblemParts {
        bindings: vec![Binding::synthetic(BindingId::new(0))],
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
            Place::new(
                PlaceId::new(0),
                BindingId::new(0),
                vec![ProjectionElem::DynamicIndex],
            ),
            Place::new(
                PlaceId::new(1),
                BindingId::new(0),
                vec![ProjectionElem::FixedIndex(0)],
            ),
        ],
        origins: vec![
            ValueOrigin::fresh(ValueOriginId::new(0)),
            ValueOrigin::unknown(ValueOriginId::new(1)),
        ],
        loans: vec![Loan {
            id: LoanId::new(0),
            kind: AccessKind::Exclusive,
            issued_at: PointId::new(2),
            place: PlaceId::new(0),
            origins: vec![ValueOriginId::new(1)].into_boxed_slice(),
            holders: vec![PlaceId::new(0)].into_boxed_slice(),
            uses: vec![UseId::new(0)].into_boxed_slice(),
            kills: Vec::new().into_boxed_slice(),
        }],
        uses: vec![
            Use {
                id: UseId::new(0),
                point: PointId::new(5),
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
                EventKind::LoanIssue {
                    loan: LoanId::new(0),
                },
                EventSource::none(),
            ),
            Event::new(
                EventId::new(2),
                PointId::new(3),
                EventKind::Fresh {
                    destination: PlaceId::new(1),
                    origin: ValueOriginId::new(0),
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
    .expect("unknown loan witness problem should validate")
}

fn write_through_witness_problem() -> BorrowProblem {
    BorrowProblem::new(super::with_return_terminator(BorrowProblemParts {
        bindings: vec![
            Binding::synthetic(BindingId::new(0)),
            Binding::synthetic(BindingId::new(1)),
            Binding::synthetic(BindingId::new(2)),
        ],
        points: (0..=8)
            .map(|ordinal| ProgramPoint::new(PointId::new(ordinal), BlockId::new(0), ordinal))
            .collect(),
        blocks: vec![CfgBlock::new(
            BlockId::new(0),
            PointId::new(0),
            PointId::new(8),
            (0..7).map(EventId::new).collect(),
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
        ],
        loans: vec![Loan {
            id: LoanId::new(0),
            kind: AccessKind::Exclusive,
            issued_at: PointId::new(5),
            place: PlaceId::new(2),
            origins: vec![ValueOriginId::new(1)].into_boxed_slice(),
            holders: vec![PlaceId::new(2)].into_boxed_slice(),
            uses: vec![UseId::new(1)].into_boxed_slice(),
            kills: Vec::new().into_boxed_slice(),
        }],
        uses: vec![
            Use {
                id: UseId::new(0),
                point: PointId::new(6),
                place: PlaceId::new(0),
                kind: UseKind::Read,
                definition: false,
            },
            Use {
                id: UseId::new(1),
                point: PointId::new(8),
                place: PlaceId::new(2),
                kind: UseKind::Read,
                definition: false,
            },
        ],
        events: vec![
            Event::new(
                EventId::new(0),
                PointId::new(1),
                EventKind::Fresh {
                    destination: PlaceId::new(1),
                    origin: ValueOriginId::new(0),
                },
                EventSource::none(),
            ),
            Event::new(
                EventId::new(1),
                PointId::new(2),
                EventKind::Fresh {
                    destination: PlaceId::new(2),
                    origin: ValueOriginId::new(1),
                },
                EventSource::none(),
            ),
            Event::new(
                EventId::new(2),
                PointId::new(3),
                EventKind::AliasFromPlace {
                    source: PlaceId::new(1),
                    destination: PlaceId::new(0),
                },
                EventSource::none(),
            ),
            Event::new(
                EventId::new(3),
                PointId::new(4),
                EventKind::AliasFromPlace {
                    source: PlaceId::new(2),
                    destination: PlaceId::new(0),
                },
                EventSource::none(),
            ),
            Event::new(
                EventId::new(4),
                PointId::new(5),
                EventKind::LoanIssue {
                    loan: LoanId::new(0),
                },
                EventSource::none(),
            ),
            Event::new(
                EventId::new(5),
                PointId::new(6),
                EventKind::Access {
                    use_id: UseId::new(0),
                },
                EventSource::none(),
            ),
            Event::new(
                EventId::new(6),
                PointId::new(8),
                EventKind::Access {
                    use_id: UseId::new(1),
                },
                EventSource::none(),
            ),
        ],
        ..BorrowProblemParts::default()
    }))
    .expect("write-through witness problem should validate")
}

fn join_origin_problem() -> BorrowProblem {
    BorrowProblem::new(super::with_return_terminator(BorrowProblemParts {
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
            (0..3).map(EventId::new).collect(),
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
            ValueOrigin::fresh(ValueOriginId::new(2)),
            ValueOrigin::new(
                ValueOriginId::new(3),
                OriginKind::Join(
                    vec![ValueOriginId::new(1), ValueOriginId::new(2)].into_boxed_slice(),
                ),
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
                EventKind::Alias {
                    source: PlaceId::new(0),
                    destination: PlaceId::new(1),
                    origins: vec![ValueOriginId::new(3)].into_boxed_slice(),
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
        ],
        uses: vec![Use {
            id: UseId::new(0),
            point: PointId::new(3),
            place: PlaceId::new(1),
            kind: UseKind::Read,
            definition: false,
        }],
        ..BorrowProblemParts::default()
    }))
    .expect("join origin problem should validate")
}

fn projection_missing_source_state_problem() -> BorrowProblem {
    BorrowProblem::new(super::with_return_terminator(BorrowProblemParts {
        bindings: vec![
            Binding::synthetic(BindingId::new(0)),
            Binding::synthetic(BindingId::new(1)),
        ],
        points: (0..=3)
            .map(|ordinal| ProgramPoint::new(PointId::new(ordinal), BlockId::new(0), ordinal))
            .collect(),
        blocks: vec![CfgBlock::new(
            BlockId::new(0),
            PointId::new(0),
            PointId::new(3),
            vec![EventId::new(0), EventId::new(1)],
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
                OriginKind::Projection {
                    source: ValueOriginId::new(0),
                    projection: ProjectionElem::FixedIndex(0),
                },
            ),
        ],
        uses: vec![Use {
            id: UseId::new(0),
            point: PointId::new(2),
            place: PlaceId::new(1),
            kind: UseKind::Read,
            definition: false,
        }],
        events: vec![
            // No Fresh event records the source place, so the projection collects an empty
            // child generation set from missing local state.
            Event::new(
                EventId::new(0),
                PointId::new(1),
                EventKind::Projection {
                    source: PlaceId::new(0),
                    destination: PlaceId::new(1),
                    origin: ValueOriginId::new(1),
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
        ],
        ..BorrowProblemParts::default()
    }))
    .expect("projection missing source state problem should validate")
}

fn copy_missing_source_state_problem() -> BorrowProblem {
    BorrowProblem::new(super::with_return_terminator(BorrowProblemParts {
        bindings: vec![
            Binding::synthetic(BindingId::new(0)),
            Binding::synthetic(BindingId::new(1)),
        ],
        points: (0..=3)
            .map(|ordinal| ProgramPoint::new(PointId::new(ordinal), BlockId::new(0), ordinal))
            .collect(),
        blocks: vec![CfgBlock::new(
            BlockId::new(0),
            PointId::new(0),
            PointId::new(3),
            vec![EventId::new(0), EventId::new(1)],
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
                OriginKind::Copy(vec![ValueOriginId::new(0)].into_boxed_slice()),
            ),
        ],
        uses: vec![Use {
            id: UseId::new(0),
            point: PointId::new(2),
            place: PlaceId::new(1),
            kind: UseKind::Read,
            definition: false,
        }],
        events: vec![
            // No Fresh event records the source place, so the copy collects an empty source
            // generation set from missing local state instead of a copied-from generation.
            Event::new(
                EventId::new(0),
                PointId::new(1),
                EventKind::Copy {
                    source: PlaceId::new(0),
                    destination: PlaceId::new(1),
                    origin: ValueOriginId::new(1),
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
        ],
        ..BorrowProblemParts::default()
    }))
    .expect("copy missing source state problem should validate")
}

fn aggregate_missing_field_source_state_problem() -> BorrowProblem {
    BorrowProblem::new(super::with_return_terminator(BorrowProblemParts {
        bindings: vec![
            Binding::synthetic(BindingId::new(0)),
            Binding::synthetic(BindingId::new(1)),
            Binding::synthetic(BindingId::new(2)),
        ],
        points: (0..=3)
            .map(|ordinal| ProgramPoint::new(PointId::new(ordinal), BlockId::new(0), ordinal))
            .collect(),
        blocks: vec![CfgBlock::new(
            BlockId::new(0),
            PointId::new(0),
            PointId::new(3),
            vec![EventId::new(0), EventId::new(1)],
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
        ],
        events: vec![
            // The first field observes its stored generation, but the second field's source
            // place carries no recorded origin state: the aggregate must not invent a fresh
            // parent generation for a field whose provenance is missing.
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
                    fields: vec![
                        AggregateField {
                            projection: ProjectionElem::Field(0),
                            source: PlaceId::new(0),
                        },
                        AggregateField {
                            projection: ProjectionElem::Field(1),
                            source: PlaceId::new(2),
                        },
                    ]
                    .into_boxed_slice(),
                },
                EventSource::none(),
            ),
        ],
        ..BorrowProblemParts::default()
    }))
    .expect("aggregate missing field source state problem should validate")
}
