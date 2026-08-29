//! Typed provenance relationships for the Boracle reference lane.
//!
//! WHAT: owns origin relation rows, typed overlap/disjoint/unknown evidence, mixed-generation
//!       sets, deterministic construction, validation and the origin-overlap query.
//! WHY:  loan conflict checking and reports consume one explicit overlap owner rather than
//!       reconstructing relatedness from traces or binding names.
//!
//! This module intentionally stops at source-semantic provenance. It does not own lifetime
//! topology, `PlaceOverlap`, retained-edge counting (REC), or experiment selection.

// Some constructors exist for focused tests and dumps that the solver does not yet emit.
#![allow(dead_code)]

use super::super::problem::{ProjectionElem, ValueOriginId};
use crate::compiler_frontend::compiler_errors::CompilerError;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

/// A Boracle-local identity for one explicit-copy correspondence graph.
///
/// This is deliberately not a normalized problem ID and does not identify an allocation family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct CopyGraphId(u32);

impl CopyGraphId {
    pub(crate) const fn new(raw: u32) -> Self {
        Self(raw)
    }

    pub(crate) const fn raw(self) -> u32 {
        self.0
    }
}

/// Why an overlap fact remains imprecise rather than proving either side independent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum PrecisionLossReason {
    UnknownCallResult,
    MissingLocalSummary,
    DynamicIndex,
    ConservativeStorageDomain,
    PathJoin,
    MixedBindingMode,
    LoopGenerationWidening,
    ExternalOpaqueValue,
}

/// Positive evidence that two origins do not observe one source-semantic generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum DisjointReason {
    DifferentFreshGenerations,
    ExplicitCopy,
    DistinctFixedFields,
    DistinctFixedIndices,
    ExperimentProof,
}

/// The semantic kind of one origin relationship row.
///
/// Projection and aggregate-child rows preserve source-to-derived direction in the row. Queries
/// over origin sets remain symmetric; direction is only evidence about how the pair was formed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum OriginRelationKind {
    Projection { projection: ProjectionElem },
    AggregateChild { projection: ProjectionElem },
    CopyCorrespondence { copy_graph: CopyGraphId },
    MayAlias { reason: PrecisionLossReason },
    ProvenDisjoint { reason: DisjointReason },
}

/// Typed fact attached to a relationship row.
///
/// Endpoint fields are repeated intentionally: validation can reject a row whose typed evidence
/// does not agree with its declared direction or pair instead of trusting a pre-rendered string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum OriginRelationEvidence {
    Projection {
        source: ValueOriginId,
        derived: ValueOriginId,
        projection: ProjectionElem,
    },
    AggregateChild {
        parent: ValueOriginId,
        child: ValueOriginId,
        projection: ProjectionElem,
    },
    CopyCorrespondence {
        source: ValueOriginId,
        result: ValueOriginId,
        copy_graph: CopyGraphId,
    },
    MayAlias {
        left: ValueOriginId,
        right: ValueOriginId,
        reason: PrecisionLossReason,
    },
    ProvenDisjoint {
        left: ValueOriginId,
        right: ValueOriginId,
        reason: DisjointReason,
    },
}

/// One typed, endpoint-oriented relationship fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct OriginRelation {
    pub(crate) left: ValueOriginId,
    pub(crate) right: ValueOriginId,
    pub(crate) kind: OriginRelationKind,
    pub(crate) evidence: OriginRelationEvidence,
}

impl OriginRelation {
    pub(crate) const fn new(
        left: ValueOriginId,
        right: ValueOriginId,
        kind: OriginRelationKind,
        evidence: OriginRelationEvidence,
    ) -> Self {
        Self {
            left,
            right,
            kind,
            evidence,
        }
    }

    pub(crate) const fn projection(
        source: ValueOriginId,
        derived: ValueOriginId,
        projection: ProjectionElem,
    ) -> Self {
        Self::new(
            source,
            derived,
            OriginRelationKind::Projection { projection },
            OriginRelationEvidence::Projection {
                source,
                derived,
                projection,
            },
        )
    }

    pub(crate) const fn aggregate_child(
        parent: ValueOriginId,
        child: ValueOriginId,
        projection: ProjectionElem,
    ) -> Self {
        Self::new(
            parent,
            child,
            OriginRelationKind::AggregateChild { projection },
            OriginRelationEvidence::AggregateChild {
                parent,
                child,
                projection,
            },
        )
    }

    pub(crate) const fn copy_correspondence(
        source: ValueOriginId,
        result: ValueOriginId,
        copy_graph: CopyGraphId,
    ) -> Self {
        Self::new(
            source,
            result,
            OriginRelationKind::CopyCorrespondence { copy_graph },
            OriginRelationEvidence::CopyCorrespondence {
                source,
                result,
                copy_graph,
            },
        )
    }

    pub(crate) const fn may_alias(
        left: ValueOriginId,
        right: ValueOriginId,
        reason: PrecisionLossReason,
    ) -> Self {
        Self::new(
            left,
            right,
            OriginRelationKind::MayAlias { reason },
            OriginRelationEvidence::MayAlias {
                left,
                right,
                reason,
            },
        )
    }

    pub(crate) const fn proven_disjoint(
        left: ValueOriginId,
        right: ValueOriginId,
        reason: DisjointReason,
    ) -> Self {
        Self::new(
            left,
            right,
            OriginRelationKind::ProvenDisjoint { reason },
            OriginRelationEvidence::ProvenDisjoint {
                left,
                right,
                reason,
            },
        )
    }
}

/// Typed evidence for an overlap decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum OriginOverlapEvidence {
    /// Identity is sufficient; no relation row is needed for one origin against itself.
    Identity { origin: ValueOriginId },
    /// A directional row proved that this pair can observe one generation.
    Relation {
        left: ValueOriginId,
        right: ValueOriginId,
        kind: OriginRelationKind,
        evidence: OriginRelationEvidence,
    },
}

/// Typed evidence for a disjoint decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct OriginDisjointEvidence {
    pub(crate) left: ValueOriginId,
    pub(crate) right: ValueOriginId,
    pub(crate) reason: DisjointReason,
    pub(crate) relation: Option<OriginRelationEvidence>,
}

/// Typed evidence for an unknown decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OriginUnknownEvidence {
    pub(crate) left: Box<[ValueOriginId]>,
    pub(crate) right: Box<[ValueOriginId]>,
    pub(crate) reason: PrecisionLossReason,
    pub(crate) relation: Option<OriginRelationEvidence>,
}

/// Result of asking whether two origin sets can observe one source-semantic generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum OriginOverlapDecision {
    Overlap(OriginOverlapEvidence),
    Disjoint(OriginDisjointEvidence),
    Unknown(OriginUnknownEvidence),
}

/// Registration metadata needed to distinguish independent fresh generations from unknown data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum OriginRegistration {
    Fresh(ValueOriginId),
    Derived(ValueOriginId),
    Unknown {
        id: ValueOriginId,
        reason: PrecisionLossReason,
    },
}

impl OriginRegistration {
    pub(crate) const fn fresh(id: ValueOriginId) -> Self {
        Self::Fresh(id)
    }

    pub(crate) const fn derived(id: ValueOriginId) -> Self {
        Self::Derived(id)
    }

    pub(crate) const fn unknown(id: ValueOriginId, reason: PrecisionLossReason) -> Self {
        Self::Unknown { id, reason }
    }

    pub(crate) const fn unknown_call_result(id: ValueOriginId) -> Self {
        Self::unknown(id, PrecisionLossReason::UnknownCallResult)
    }

    const fn id(self) -> ValueOriginId {
        match self {
            Self::Fresh(id) | Self::Derived(id) => id,
            Self::Unknown { id, .. } => id,
        }
    }

    const fn unknown_reason(self) -> Option<PrecisionLossReason> {
        match self {
            Self::Unknown { reason, .. } => Some(reason),
            Self::Fresh(_) | Self::Derived(_) => None,
        }
    }

    const fn is_fresh(self) -> bool {
        matches!(self, Self::Fresh(_))
    }
}

/// Deterministic relation owner for one set of registered origin facts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OriginRelations {
    registrations: BTreeMap<ValueOriginId, OriginRegistration>,
    rows: Box<[OriginRelation]>,
    rows_by_pair: BTreeMap<(ValueOriginId, ValueOriginId), Box<[OriginRelation]>>,
    mixed_generation_sets: Box<[Box<[ValueOriginId]>]>,
}

impl OriginRelations {
    /// Construct and validate a relation table from hand-authored or later extracted facts.
    pub(crate) fn new(
        registrations: impl IntoIterator<Item = OriginRegistration>,
        relations: impl IntoIterator<Item = OriginRelation>,
    ) -> Result<Self, CompilerError> {
        let mut registration_map = BTreeMap::new();
        for registration in registrations {
            let id = registration.id();
            if registration_map.insert(id, registration).is_some() {
                return Err(relation_error(format!(
                    "duplicate registered origin {:?}",
                    id
                )));
            }
        }

        let mut sorted_rows = BTreeSet::new();
        for relation in relations {
            validate_relation(&registration_map, relation)?;
            sorted_rows.insert(relation);
        }

        let rows: Box<[OriginRelation]> = sorted_rows.into_iter().collect();
        validate_contradictions(&rows)?;

        let mut grouped_rows: BTreeMap<(ValueOriginId, ValueOriginId), Vec<OriginRelation>> =
            BTreeMap::new();
        for relation in rows.iter().copied() {
            grouped_rows
                .entry(normalized_pair(relation.left, relation.right))
                .or_default()
                .push(relation);
        }
        let rows_by_pair = grouped_rows
            .into_iter()
            .map(|(pair, rows)| (pair, rows.into_boxed_slice()))
            .collect();

        Ok(Self {
            registrations: registration_map,
            rows,
            rows_by_pair,
            mixed_generation_sets: Box::new([]),
        })
    }

    /// Construct a table whose registered IDs are all independent fresh generations.
    pub(crate) fn from_fresh_origins(
        origins: impl IntoIterator<Item = ValueOriginId>,
        relations: impl IntoIterator<Item = OriginRelation>,
    ) -> Result<Self, CompilerError> {
        Self::new(
            origins.into_iter().map(OriginRegistration::fresh),
            relations,
        )
    }

    pub(crate) fn rows(&self) -> &[OriginRelation] {
        &self.rows
    }

    /// Record mixed alias/slot unions without relating their independent members.
    pub(crate) fn with_mixed_generation_sets(
        mut self,
        sets: impl IntoIterator<Item = Box<[ValueOriginId]>>,
    ) -> Self {
        let mut unique = BTreeSet::new();
        for set in sets {
            let mut origins = set.into_vec();
            origins.sort_by_key(|origin| origin.raw());
            origins.dedup();
            if origins.len() >= 2 {
                unique.insert(origins.into_boxed_slice());
            }
        }
        self.mixed_generation_sets = unique.into_iter().collect();
        self
    }

    pub(crate) fn mixed_generation_sets(&self) -> &[Box<[ValueOriginId]>] {
        &self.mixed_generation_sets
    }

    /// Ask the one relation-owned origin-set overlap question.
    pub(crate) fn query_overlap(
        &self,
        left: &[ValueOriginId],
        right: &[ValueOriginId],
    ) -> Result<OriginOverlapDecision, CompilerError> {
        let mut left = canonical_query_set(&self.registrations, left, "left")?;
        let mut right = canonical_query_set(&self.registrations, right, "right")?;

        // The question is symmetric. Canonicalise the two sets before selecting a witness so
        // swapping query arguments cannot change either the decision or its typed evidence.
        if left > right {
            std::mem::swap(&mut left, &mut right);
        }

        if left.is_empty() || right.is_empty() {
            return Ok(OriginOverlapDecision::Unknown(OriginUnknownEvidence {
                left: left.iter().copied().collect::<Vec<_>>().into_boxed_slice(),
                right: right.iter().copied().collect::<Vec<_>>().into_boxed_slice(),
                reason: PrecisionLossReason::MissingLocalSummary,
                relation: None,
            }));
        }

        let mut first_unknown = None;
        let mut first_disjoint = None;
        for left_origin in left.iter().copied() {
            for right_origin in right.iter().copied() {
                match self.query_pair(left_origin, right_origin) {
                    OriginOverlapDecision::Overlap(evidence) => {
                        return Ok(OriginOverlapDecision::Overlap(evidence));
                    }
                    OriginOverlapDecision::Disjoint(evidence) => {
                        if first_disjoint.is_none() {
                            first_disjoint = Some(evidence);
                        }
                    }
                    OriginOverlapDecision::Unknown(evidence) => {
                        if first_unknown.is_none() {
                            first_unknown = Some(evidence);
                        }
                    }
                }
            }
        }

        if let Some(evidence) = first_unknown {
            Ok(OriginOverlapDecision::Unknown(evidence))
        } else {
            Ok(OriginOverlapDecision::Disjoint(first_disjoint.expect(
                "non-empty origin sets always produce a pair decision",
            )))
        }
    }

    /// Render registrations and rows in a stable order for focused reports and tests.
    pub(crate) fn debug_dump(&self) -> String {
        let mut dump = String::new();
        dump.push_str("origin-registrations:\n");
        for registration in self.registrations.values() {
            writeln!(&mut dump, "  {registration:?}").expect("writing to String cannot fail");
        }
        dump.push_str("mixed-generation-sets:\n");
        for set in self.mixed_generation_sets.iter() {
            writeln!(&mut dump, "  {set:?}").expect("writing to String cannot fail");
        }
        dump.push_str("relations:\n");
        for relation in self.rows.iter() {
            writeln!(&mut dump, "  {relation:?}").expect("writing to String cannot fail");
        }
        dump
    }

    /// Render only the precision-loss views owned by the relation table.
    pub(crate) fn precision_debug_dump(&self) -> String {
        let mut dump = String::new();
        dump.push_str("unknown-origins:\n");
        for registration in self.registrations.values() {
            if matches!(registration, OriginRegistration::Unknown { .. }) {
                writeln!(&mut dump, "  {registration:?}").expect("writing to String cannot fail");
            }
        }
        dump.push_str("may-alias-relations:\n");
        for relation in self.rows.iter() {
            if matches!(relation.kind, OriginRelationKind::MayAlias { .. }) {
                writeln!(&mut dump, "  {relation:?}").expect("writing to String cannot fail");
            }
        }
        dump.push_str("mixed-generation-sets:\n");
        for set in self.mixed_generation_sets.iter() {
            writeln!(&mut dump, "  {set:?}").expect("writing to String cannot fail");
        }
        dump
    }

    fn query_pair(&self, left: ValueOriginId, right: ValueOriginId) -> OriginOverlapDecision {
        if left == right {
            return OriginOverlapDecision::Overlap(OriginOverlapEvidence::Identity {
                origin: left,
            });
        }

        let (left, right) = normalized_pair(left, right);

        if let Some(relations) = self.rows_by_pair.get(&normalized_pair(left, right)) {
            let mut first_disjoint = None;
            for relation in relations.iter() {
                match relation.kind {
                    OriginRelationKind::Projection { .. }
                    | OriginRelationKind::AggregateChild { .. } => {
                        return OriginOverlapDecision::Overlap(OriginOverlapEvidence::Relation {
                            left: relation.left,
                            right: relation.right,
                            kind: relation.kind,
                            evidence: relation.evidence,
                        });
                    }
                    OriginRelationKind::MayAlias { .. } => {
                        return OriginOverlapDecision::Overlap(OriginOverlapEvidence::Relation {
                            left: relation.left,
                            right: relation.right,
                            kind: relation.kind,
                            evidence: relation.evidence,
                        });
                    }
                    OriginRelationKind::CopyCorrespondence { copy_graph } => {
                        if first_disjoint.is_none() {
                            first_disjoint =
                                Some(OriginOverlapDecision::Disjoint(OriginDisjointEvidence {
                                    left,
                                    right,
                                    reason: DisjointReason::ExplicitCopy,
                                    relation: Some(OriginRelationEvidence::CopyCorrespondence {
                                        source: relation.left,
                                        result: relation.right,
                                        copy_graph,
                                    }),
                                }));
                        }
                    }
                    OriginRelationKind::ProvenDisjoint { reason } => {
                        if first_disjoint.is_none() {
                            first_disjoint =
                                Some(OriginOverlapDecision::Disjoint(OriginDisjointEvidence {
                                    left,
                                    right,
                                    reason,
                                    relation: Some(relation.evidence),
                                }));
                        }
                    }
                }
            }

            if let Some(evidence) = first_disjoint {
                return evidence;
            }
        }

        let left_registration = self
            .registrations
            .get(&left)
            .copied()
            .expect("query IDs are validated before pair evaluation");
        let right_registration = self
            .registrations
            .get(&right)
            .copied()
            .expect("query IDs are validated before pair evaluation");

        if left_registration.is_fresh() && right_registration.is_fresh() {
            return OriginOverlapDecision::Disjoint(OriginDisjointEvidence {
                left,
                right,
                reason: DisjointReason::DifferentFreshGenerations,
                relation: None,
            });
        }

        let reason = left_registration
            .unknown_reason()
            .into_iter()
            .chain(right_registration.unknown_reason())
            .min()
            .unwrap_or(PrecisionLossReason::MissingLocalSummary);
        OriginOverlapDecision::Unknown(OriginUnknownEvidence {
            left: vec![left].into_boxed_slice(),
            right: vec![right].into_boxed_slice(),
            reason,
            relation: None,
        })
    }
}

fn normalized_pair(left: ValueOriginId, right: ValueOriginId) -> (ValueOriginId, ValueOriginId) {
    if left <= right {
        (left, right)
    } else {
        (right, left)
    }
}

fn canonical_query_set(
    registrations: &BTreeMap<ValueOriginId, OriginRegistration>,
    origins: &[ValueOriginId],
    side: &str,
) -> Result<BTreeSet<ValueOriginId>, CompilerError> {
    let mut canonical = BTreeSet::new();
    for origin in origins.iter().copied() {
        if !registrations.contains_key(&origin) {
            return Err(relation_error(format!(
                "origin overlap query {side} set references unknown origin {:?}",
                origin
            )));
        }
        canonical.insert(origin);
    }
    Ok(canonical)
}

fn validate_relation(
    registrations: &BTreeMap<ValueOriginId, OriginRegistration>,
    relation: OriginRelation,
) -> Result<(), CompilerError> {
    if !registrations.contains_key(&relation.left) {
        return Err(relation_error(format!(
            "relation {:?} references unknown left origin {:?}",
            relation.kind, relation.left
        )));
    }
    if !registrations.contains_key(&relation.right) {
        return Err(relation_error(format!(
            "relation {:?} references unknown right origin {:?}",
            relation.kind, relation.right
        )));
    }
    if relation.left == relation.right {
        if matches!(relation.kind, OriginRelationKind::CopyCorrespondence { .. }) {
            return Err(relation_error(
                "copy correspondence source and result must be different origins",
            ));
        }
        return Err(relation_error(format!(
            "relation {:?} is a forbidden self-relation for origin {:?}",
            relation.kind, relation.left
        )));
    }

    match (relation.kind, relation.evidence) {
        (
            OriginRelationKind::Projection { projection },
            OriginRelationEvidence::Projection {
                source,
                derived,
                projection: evidence_projection,
            },
        ) if relation.left == source
            && relation.right == derived
            && projection == evidence_projection => {}
        (
            OriginRelationKind::AggregateChild { projection },
            OriginRelationEvidence::AggregateChild {
                parent,
                child,
                projection: evidence_projection,
            },
        ) if relation.left == parent
            && relation.right == child
            && projection == evidence_projection => {}
        (
            OriginRelationKind::CopyCorrespondence { copy_graph },
            OriginRelationEvidence::CopyCorrespondence {
                source,
                result,
                copy_graph: evidence_graph,
            },
        ) if relation.left == source
            && relation.right == result
            && copy_graph == evidence_graph => {}
        (
            OriginRelationKind::MayAlias { reason },
            OriginRelationEvidence::MayAlias {
                left,
                right,
                reason: evidence_reason,
            },
        ) if relation.left == left && relation.right == right && reason == evidence_reason => {
            if reason == PrecisionLossReason::UnknownCallResult {
                return Err(relation_error(
                    "UnknownCallResult must be registered as unknown provenance, not a MayAlias row",
                ));
            }
        }
        (
            OriginRelationKind::ProvenDisjoint { reason },
            OriginRelationEvidence::ProvenDisjoint {
                left,
                right,
                reason: evidence_reason,
            },
        ) if relation.left == left && relation.right == right && reason == evidence_reason => {}
        (OriginRelationKind::Projection { .. }, _)
        | (OriginRelationKind::AggregateChild { .. }, _) => {
            return Err(relation_error(format!(
                "invalid projection evidence for relation {:?} between {:?} and {:?}",
                relation.kind, relation.left, relation.right
            )));
        }
        _ => {
            return Err(relation_error(format!(
                "relation kind {:?} has incompatible typed evidence {:?}",
                relation.kind, relation.evidence
            )));
        }
    }

    Ok(())
}

fn validate_contradictions(rows: &[OriginRelation]) -> Result<(), CompilerError> {
    let mut facts: BTreeMap<(ValueOriginId, ValueOriginId), (bool, bool)> = BTreeMap::new();
    for relation in rows.iter().copied() {
        let fact = facts
            .entry(normalized_pair(relation.left, relation.right))
            .or_default();
        match relation.kind {
            OriginRelationKind::Projection { .. }
            | OriginRelationKind::AggregateChild { .. }
            | OriginRelationKind::MayAlias { .. } => fact.0 = true,
            OriginRelationKind::CopyCorrespondence { .. }
            | OriginRelationKind::ProvenDisjoint { .. } => fact.1 = true,
        }
    }

    for (pair, (forced_overlap, proven_disjoint)) in facts {
        if forced_overlap && proven_disjoint {
            return Err(relation_error(format!(
                "contradictory origin relation facts for {:?} and {:?}: forced overlap and proven disjoint",
                pair.0, pair.1
            )));
        }
    }
    Ok(())
}

fn relation_error(message: impl Into<String>) -> CompilerError {
    CompilerError::compiler_error(format!("Boracle origin relations: {}", message.into()))
}
