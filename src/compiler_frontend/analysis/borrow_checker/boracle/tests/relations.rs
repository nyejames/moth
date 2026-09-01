use super::super::{
    CopyGraphId, DisjointReason, OriginDisjointEvidence, OriginOverlapDecision,
    OriginOverlapEvidence, OriginRegistration, OriginRelation, OriginRelationEvidence,
    OriginRelationKind, OriginRelations, PrecisionLossReason,
};
use crate::compiler_frontend::analysis::borrow_checker::problem::{ProjectionElem, ValueOriginId};
use crate::compiler_frontend::compiler_errors::ErrorType;

fn origin(raw: u32) -> ValueOriginId {
    ValueOriginId::new(raw)
}

fn fresh(raw: u32) -> OriginRegistration {
    OriginRegistration::fresh(origin(raw))
}

fn derived(raw: u32) -> OriginRegistration {
    OriginRegistration::derived(origin(raw))
}

fn relation_table(
    registrations: impl IntoIterator<Item = OriginRegistration>,
    relations: impl IntoIterator<Item = OriginRelation>,
) -> OriginRelations {
    OriginRelations::new(registrations, relations).expect("hand-authored relation rows validate")
}

#[test]
fn same_origin_is_overlapping_by_identity_without_a_relation_row() {
    // Invariant: one ValueOriginId is an exact source-semantic generation, so identity wins even
    // when no explicit relation row exists.
    let relations = relation_table([fresh(0)], []);

    assert!(relations.rows().is_empty());
    assert_eq!(
        relations
            .query_overlap(&[origin(0)], &[origin(0)])
            .expect("identity query should validate"),
        OriginOverlapDecision::Overlap(OriginOverlapEvidence::Identity { origin: origin(0) })
    );
}

#[test]
fn fresh_origins_are_disjoint_by_generation_identity() {
    // Invariant: independently registered fresh generations are disjoint without an alias edge.
    let relations = relation_table([fresh(0), fresh(1)], []);

    let forward = relations
        .query_overlap(&[origin(0)], &[origin(1)])
        .expect("fresh-origin query should validate");
    let reverse = relations
        .query_overlap(&[origin(1)], &[origin(0)])
        .expect("reverse fresh-origin query should validate");

    assert_eq!(forward, reverse, "origin-set overlap is symmetric");
    assert_eq!(
        forward,
        OriginOverlapDecision::Disjoint(OriginDisjointEvidence {
            left: origin(0),
            right: origin(1),
            reason: DisjointReason::DifferentFreshGenerations,
            relation: None,
        })
    );
}

#[test]
fn copy_source_and_result_are_positive_explicit_disjointness() {
    // Invariant: copying reads the source but creates an independent result generation.
    let copy = OriginRelation::copy_correspondence(origin(0), origin(1), CopyGraphId::new(7));
    let relations = relation_table([fresh(0), fresh(1)], [copy]);

    let forward = relations
        .query_overlap(&[origin(0)], &[origin(1)])
        .expect("copy query should validate");
    let reverse = relations
        .query_overlap(&[origin(1)], &[origin(0)])
        .expect("reverse copy query should validate");

    assert_eq!(
        forward, reverse,
        "copy disjointness must be query-symmetric"
    );
    assert!(matches!(
        forward,
        OriginOverlapDecision::Disjoint(OriginDisjointEvidence {
            reason: DisjointReason::ExplicitCopy,
            relation: Some(OriginRelationEvidence::CopyCorrespondence {
                source,
                result,
                copy_graph,
            }),
            ..
        }) if source == origin(0)
            && result == origin(1)
            && copy_graph == CopyGraphId::new(7)
    ));
}

#[test]
fn may_alias_rows_are_overlap_with_precision_loss_evidence() {
    // Invariant: a pair-specific precision-loss fact answers the may-overlap query positively;
    // only absent provenance uses the Unknown decision.
    let may_alias = OriginRelation::may_alias(origin(0), origin(1), PrecisionLossReason::PathJoin);
    let relations = relation_table([fresh(0), derived(1)], [may_alias]);

    let decision = relations
        .query_overlap(&[origin(1)], &[origin(0)])
        .expect("MayAlias query should validate");
    assert!(matches!(
        decision,
        OriginOverlapDecision::Overlap(OriginOverlapEvidence::Relation {
            left,
            right,
            kind: OriginRelationKind::MayAlias {
                reason: PrecisionLossReason::PathJoin,
            },
            evidence: OriginRelationEvidence::MayAlias {
                left: evidence_left,
                right: evidence_right,
                reason: PrecisionLossReason::PathJoin,
            },
        }) if left == origin(0)
            && right == origin(1)
            && evidence_left == origin(0)
            && evidence_right == origin(1)
    ));
}

#[test]
fn projection_relationship_is_directional_but_query_decision_is_symmetric() {
    // Invariant: a projection row records source -> derived direction, while overlap asks a
    // symmetric question and therefore sees the same pair in either query order.
    let projection = OriginRelation::projection(origin(0), origin(1), ProjectionElem::Field(2));
    let relations = relation_table([derived(0), derived(1)], [projection]);

    let forward = relations
        .query_overlap(&[origin(0)], &[origin(1)])
        .expect("projection query should validate");
    let reverse = relations
        .query_overlap(&[origin(1)], &[origin(0)])
        .expect("reverse projection query should validate");

    assert_eq!(forward, reverse, "projection overlap decision is symmetric");
    assert!(matches!(
        forward,
        OriginOverlapDecision::Overlap(OriginOverlapEvidence::Relation {
            left,
            right,
            kind: OriginRelationKind::Projection { projection: ProjectionElem::Field(2) },
            evidence: OriginRelationEvidence::Projection {
                source,
                derived,
                projection: ProjectionElem::Field(2),
            },
        }) if left == origin(0)
            && right == origin(1)
            && source == origin(0)
            && derived == origin(1)
    ));
}

#[test]
fn projection_siblings_do_not_become_related_through_their_parent() {
    // Invariant: relation lookup is pair-local; an undirected parent edge must not transitively
    // relate two independent children.
    let relations = relation_table(
        [fresh(0), fresh(1), fresh(2)],
        [
            OriginRelation::projection(origin(0), origin(1), ProjectionElem::Field(0)),
            OriginRelation::projection(origin(0), origin(2), ProjectionElem::Field(1)),
        ],
    );

    let decision = relations
        .query_overlap(&[origin(1)], &[origin(2)])
        .expect("sibling query should validate");
    assert_eq!(
        decision,
        OriginOverlapDecision::Disjoint(OriginDisjointEvidence {
            left: origin(1),
            right: origin(2),
            reason: DisjointReason::DifferentFreshGenerations,
            relation: None,
        }),
        "sibling origins must not inherit an overlap edge through their parent"
    );
}

#[test]
fn aggregate_containment_does_not_imply_sibling_overlap() {
    // Invariant: aggregate-child rows explain each child separately; containment is not a sibling
    // alias relation.
    let relations = relation_table(
        [fresh(0), fresh(1), fresh(2)],
        [
            OriginRelation::aggregate_child(origin(0), origin(1), ProjectionElem::Field(0)),
            OriginRelation::aggregate_child(origin(0), origin(2), ProjectionElem::Field(1)),
        ],
    );

    let parent_child = relations
        .query_overlap(&[origin(0)], &[origin(1)])
        .expect("aggregate-child query should validate");
    let siblings = relations
        .query_overlap(&[origin(1)], &[origin(2)])
        .expect("aggregate sibling query should validate");

    assert!(matches!(
        parent_child,
        OriginOverlapDecision::Overlap(OriginOverlapEvidence::Relation {
            kind: OriginRelationKind::AggregateChild {
                projection: ProjectionElem::Field(0)
            },
            ..
        })
    ));
    assert_eq!(
        siblings,
        OriginOverlapDecision::Disjoint(OriginDisjointEvidence {
            left: origin(1),
            right: origin(2),
            reason: DisjointReason::DifferentFreshGenerations,
            relation: None,
        })
    );
}

#[test]
fn unknown_call_result_returns_unknown_overlap_evidence() {
    // Invariant: an opaque call result is top-like uncertainty, not a MayAlias edge to every
    // registered origin.
    let relations = relation_table(
        [fresh(0), OriginRegistration::unknown_call_result(origin(1))],
        [],
    );

    let forward = relations
        .query_overlap(&[origin(0)], &[origin(1)])
        .expect("unknown call-result query should validate");
    let reverse = relations
        .query_overlap(&[origin(1)], &[origin(0)])
        .expect("reverse unknown call-result query should validate");

    assert_eq!(forward, reverse, "unknown decision is query-symmetric");
    assert!(matches!(
        forward,
        OriginOverlapDecision::Unknown(evidence)
            if evidence.reason == PrecisionLossReason::UnknownCallResult
                && evidence.relation.is_none()
                && evidence.left.as_ref() == [origin(0)]
                && evidence.right.as_ref() == [origin(1)]
    ));
    assert!(
        relations.rows().is_empty(),
        "unknown provenance is not a graph edge"
    );
}

#[test]
fn relation_rows_and_debug_dumps_are_deterministic() {
    // Invariant: construction canonicalises map/set input, so row order and debug output do not
    // depend on extraction or fixture iteration order.
    let rows = [
        OriginRelation::proven_disjoint(origin(1), origin(2), DisjointReason::DistinctFixedIndices),
        OriginRelation::projection(origin(0), origin(1), ProjectionElem::Field(0)),
        OriginRelation::copy_correspondence(origin(0), origin(2), CopyGraphId::new(3)),
    ];
    let first = relation_table([fresh(2), fresh(0), fresh(1)], rows);
    let second = relation_table([fresh(1), fresh(2), fresh(0)], [rows[2], rows[0], rows[1]]);

    assert_eq!(first.rows(), second.rows());
    assert_eq!(first.debug_dump(), second.debug_dump());
    assert_eq!(
        first.debug_dump(),
        concat!(
            "origin-registrations:\n",
            "  Fresh(ValueOriginId(0))\n",
            "  Fresh(ValueOriginId(1))\n",
            "  Fresh(ValueOriginId(2))\n",
            "mixed-generation-sets:\n",
            "relations:\n",
            "  OriginRelation { left: ValueOriginId(0), right: ValueOriginId(1), kind: Projection { projection: Field(0) }, evidence: Projection { source: ValueOriginId(0), derived: ValueOriginId(1), projection: Field(0) } }\n",
            "  OriginRelation { left: ValueOriginId(0), right: ValueOriginId(2), kind: CopyCorrespondence { copy_graph: CopyGraphId(3) }, evidence: CopyCorrespondence { source: ValueOriginId(0), result: ValueOriginId(2), copy_graph: CopyGraphId(3) } }\n",
            "  OriginRelation { left: ValueOriginId(1), right: ValueOriginId(2), kind: ProvenDisjoint { reason: DistinctFixedIndices }, evidence: ProvenDisjoint { left: ValueOriginId(1), right: ValueOriginId(2), reason: DistinctFixedIndices } }\n",
        )
    );
}

#[test]
fn unknown_origin_ids_are_rejected_as_compiler_errors() {
    // Invariant: relation endpoints must belong to the registered origin identity space.
    assert_compiler_error(
        OriginRelations::new(
            [fresh(0)],
            [OriginRelation::proven_disjoint(
                origin(0),
                origin(9),
                DisjointReason::ExperimentProof,
            )],
        ),
        "unknown right origin",
    );
}

#[test]
fn forbidden_self_relations_are_rejected_as_compiler_errors() {
    // Invariant: identity already expresses self-overlap; relationship rows require two origins.
    assert_compiler_error(
        OriginRelations::new(
            [fresh(0)],
            [OriginRelation::projection(
                origin(0),
                origin(0),
                ProjectionElem::Field(0),
            )],
        ),
        "forbidden self-relation",
    );
}

#[test]
fn copy_source_and_result_must_differ() {
    // Invariant: a copy correspondence is positive independence between distinct source and
    // result generations.
    assert_compiler_error(
        OriginRelations::new(
            [fresh(0)],
            [OriginRelation::copy_correspondence(
                origin(0),
                origin(0),
                CopyGraphId::new(1),
            )],
        ),
        "source and result must be different",
    );
}

#[test]
fn invalid_projection_evidence_is_rejected_as_a_compiler_error() {
    // Invariant: projection row endpoints and projection evidence must agree exactly.
    assert_compiler_error(
        OriginRelations::new(
            [fresh(0), fresh(1)],
            [OriginRelation::new(
                origin(0),
                origin(1),
                OriginRelationKind::Projection {
                    projection: ProjectionElem::Field(0),
                },
                OriginRelationEvidence::Projection {
                    source: origin(0),
                    derived: origin(1),
                    projection: ProjectionElem::Field(1),
                },
            )],
        ),
        "invalid projection evidence",
    );
}

#[test]
fn contradictory_overlap_and_disjoint_rows_are_rejected_as_compiler_errors() {
    // Invariant: one origin pair cannot be both forced-overlap and positively proven disjoint.
    assert_compiler_error(
        OriginRelations::new(
            [fresh(0), fresh(1)],
            [
                OriginRelation::projection(origin(0), origin(1), ProjectionElem::Field(0)),
                OriginRelation::proven_disjoint(
                    origin(0),
                    origin(1),
                    DisjointReason::DistinctFixedFields,
                ),
            ],
        ),
        "forced overlap and proven disjoint",
    );
}

#[test]
fn contradictory_may_alias_and_disjoint_rows_are_rejected_as_compiler_errors() {
    // Invariant: MayAlias is a forced overlap fact and cannot coexist with proven disjointness.
    assert_compiler_error(
        OriginRelations::new(
            [fresh(0), fresh(1)],
            [
                OriginRelation::may_alias(origin(0), origin(1), PrecisionLossReason::PathJoin),
                OriginRelation::proven_disjoint(
                    origin(0),
                    origin(1),
                    DisjointReason::DistinctFixedFields,
                ),
            ],
        ),
        "forced overlap and proven disjoint",
    );
}

fn assert_compiler_error<T: std::fmt::Debug>(
    result: Result<T, crate::compiler_frontend::compiler_errors::CompilerError>,
    message: &str,
) {
    let error = result.expect_err("malformed relation rows must fail");
    assert_eq!(error.error_type, ErrorType::Compiler);
    assert!(
        error.msg.contains(message),
        "expected CompilerError containing {message:?}, got {:?}",
        error.msg
    );
}
