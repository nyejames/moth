//! Origin-aware loan liveness and access-conflict solving for Boracle.
//!
//! WHAT: derives source loans from normalized alias and access events, then records CFG-aware
//! liveness and typed overlap conflicts.
//! WHY: legality depends on the capability and represented value lineage, not on binding names or
//! lexical visibility alone.

// Some research-facing rows are not printed by every current dump. Keep the complete typed
// result surface warning-free as future investigation queries are added.
#![allow(dead_code)]

use super::super::problem::{
    AccessKind, BlockId, BorrowProblem, Event, EventId, EventKind, Loan, LoanId, PlaceId,
    PlaceOverlap, PointId, UseId, UseKind, ValueOriginId,
};
use super::OriginSolution;
use crate::compiler_frontend::compiler_errors::CompilerError;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// One solver-owned loan, including loans inferred from normalized source events.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LoanFact {
    pub(crate) id: LoanId,
    pub(crate) kind: AccessKind,
    pub(crate) issued_at: PointId,
    pub(crate) issue_event: Option<EventId>,
    pub(crate) place: PlaceId,
    pub(crate) origins: Box<[ValueOriginId]>,
    pub(crate) holders: Box<[PlaceId]>,
    pub(crate) uses: Box<[UseId]>,
    pub(crate) kills: Box<[PointId]>,
    pub(crate) until_event: Option<EventId>,
    pub(crate) live_points: Box<[PointId]>,
}

/// The overlap evidence for one rejected access.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConflictWitness {
    pub(crate) access_event: EventId,
    pub(crate) access_use: Option<UseId>,
    pub(crate) access_kind: AccessKind,
    pub(crate) access_place: PlaceId,
    pub(crate) access_origins: Box<[ValueOriginId]>,
    pub(crate) conflicting_loan: LoanId,
    pub(crate) loan_issue_point: PointId,
    pub(crate) loan_origins: Box<[ValueOriginId]>,
    pub(crate) keeping_use: Option<UseId>,
    pub(crate) overlap: PlaceOverlap,
    pub(crate) origin_overlap: bool,
}

/// One access decision retained even when it is legal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AccessDecision {
    pub(crate) event: EventId,
    pub(crate) use_id: Option<UseId>,
    pub(crate) place: PlaceId,
    pub(crate) origins: Box<[ValueOriginId]>,
    pub(crate) kind: AccessKind,
    pub(crate) allowed: bool,
}

/// Complete loan and conflict result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LoanSolution {
    loans: Box<[LoanFact]>,
    decisions: Box<[AccessDecision]>,
    conflicts: Box<[ConflictWitness]>,
}

impl LoanSolution {
    pub(crate) fn loans(&self) -> &[LoanFact] {
        &self.loans
    }

    pub(crate) fn decisions(&self) -> &[AccessDecision] {
        &self.decisions
    }

    pub(crate) fn conflicts(&self) -> &[ConflictWitness] {
        &self.conflicts
    }

    pub(crate) fn debug_dump(&self) -> String {
        format!("{self:#?}")
    }
}

/// Boracle loan solver with selectable exclusive-capability liveness.
pub(crate) struct LoanSolver;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExclusiveLoanLiveness {
    Conservative,
    UseDriven,
}

impl LoanSolver {
    pub(crate) fn solve(
        problem: &BorrowProblem,
        origins: &OriginSolution,
    ) -> Result<LoanSolution, CompilerError> {
        Self::solve_with_liveness(problem, origins, ExclusiveLoanLiveness::Conservative)
    }

    pub(crate) fn solve_with_liveness(
        problem: &BorrowProblem,
        origins: &OriginSolution,
        exclusive_liveness: ExclusiveLoanLiveness,
    ) -> Result<LoanSolution, CompilerError> {
        problem.validate()?;
        let graph = EventGraph::new(problem, exclusive_liveness)?;

        let mut loans = problem
            .loans()
            .iter()
            .map(|loan| graph.explicit_loan(problem, origins, loan))
            .collect::<Result<Vec<_>, _>>()?;

        let next_loan_id = loans.len();
        let mut derived_alias_loans = derive_alias_loans(problem, origins, &graph, next_loan_id)?;
        loans.append(&mut derived_alias_loans);

        let next_loan_id = loans.len();
        let mut derived_provenance_loans =
            derive_provenance_loans(problem, origins, &graph, next_loan_id)?;
        loans.append(&mut derived_provenance_loans);

        let next_loan_id = loans.len();
        let mut call_argument_loans =
            derive_call_argument_loans(problem, origins, &graph, next_loan_id)?;
        loans.append(&mut call_argument_loans);

        let mut decisions = Vec::new();
        let mut conflicts = Vec::new();
        for event in problem.events() {
            let accesses = event_accesses(problem, origins, event)?;
            for access in accesses {
                let access_origins = graph.access_origins(problem, origins, event, &access);
                let mut access_conflicts = Vec::new();
                for loan in &loans {
                    if access
                        .use_id
                        .is_some_and(|use_id| loan.uses.contains(&use_id))
                    {
                        continue;
                    }
                    if !graph.loan_live_at_event(problem, loan, event.id) {
                        continue;
                    }
                    let Some((overlap, origin_overlap)) =
                        access_conflict_overlap(problem, origins, &access, &access_origins, loan)
                    else {
                        continue;
                    };
                    let witness = ConflictWitness {
                        access_event: event.id,
                        access_use: access.use_id,
                        access_kind: access.kind,
                        access_place: access.place,
                        access_origins: access_origins.clone(),
                        conflicting_loan: loan.id,
                        loan_issue_point: loan.issued_at,
                        loan_origins: loan.origins.clone(),
                        keeping_use: graph.keeping_use(problem, loan, event.id),
                        overlap,
                        origin_overlap,
                    };
                    access_conflicts.push(witness.clone());
                    conflicts.push(witness);
                }

                decisions.push(AccessDecision {
                    event: event.id,
                    use_id: access.use_id,
                    place: access.place,
                    origins: access_origins,
                    kind: access.kind,
                    allowed: access_conflicts.is_empty(),
                });
            }
        }

        decisions.sort_by_key(|decision| {
            (
                decision.event.raw(),
                decision.use_id.map(UseId::raw),
                decision.place.raw(),
            )
        });
        conflicts.sort_by_key(|conflict| {
            (
                conflict.access_event.raw(),
                conflict.access_use.map(UseId::raw),
                conflict.conflicting_loan.raw(),
            )
        });
        loans.sort_by_key(|loan| loan.id.raw());

        Ok(LoanSolution {
            loans: loans.into_boxed_slice(),
            decisions: decisions.into_boxed_slice(),
            conflicts: conflicts.into_boxed_slice(),
        })
    }
}

#[derive(Debug, Clone)]
struct AccessFact {
    use_id: Option<UseId>,
    place: PlaceId,
    kind: AccessKind,
    definition: bool,
}

fn event_accesses(
    problem: &BorrowProblem,
    origins: &OriginSolution,
    event: &Event,
) -> Result<Vec<AccessFact>, CompilerError> {
    match &event.kind {
        EventKind::Access { use_id } => {
            let use_row = problem.uses().get(use_id.index()).ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "Boracle loan solver cannot locate access use {:?}",
                    use_id
                ))
            })?;
            Ok(vec![AccessFact {
                use_id: Some(*use_id),
                place: use_row.place,
                kind: access_kind_for_use(use_row.kind),
                definition: use_row.definition && !origins.is_write_through_use(*use_id),
            }])
        }
        EventKind::CallArgument { argument, .. } => Ok(vec![AccessFact {
            use_id: Some(argument.use_id),
            place: argument.place,
            kind: argument.access,
            definition: false,
        }]),
        EventKind::ExclusiveAlias { source, .. }
        | EventKind::ExclusiveAliasFromPlace { source, .. }
            if origins.is_initial_alias_event(event.id) =>
        {
            Ok(vec![AccessFact {
                // Issuing a mutable alias is itself an exclusive access to the represented
                // source. The later alias loan describes the capability; this event checks that
                // issuance does not overlap an already-live loan.
                use_id: None,
                place: *source,
                kind: AccessKind::Exclusive,
                definition: false,
            }])
        }
        _ => Ok(Vec::new()),
    }
}

fn access_kind_for_use(kind: UseKind) -> AccessKind {
    match kind {
        UseKind::Read | UseKind::LoanObservation => AccessKind::Shared,
        UseKind::Write => AccessKind::Exclusive,
    }
}

fn access_conflict_overlap(
    problem: &BorrowProblem,
    origins: &OriginSolution,
    access: &AccessFact,
    access_origins: &[ValueOriginId],
    loan: &LoanFact,
) -> Option<(PlaceOverlap, bool)> {
    if access.definition {
        return None;
    }
    if !access_kinds_conflict(access.kind, loan.kind) {
        return None;
    }

    let access_place = problem.places().get(access.place.index())?;
    let loan_place = problem.places().get(loan.place.index())?;
    let structural_overlap = access_place.overlap(loan_place);
    let origin_overlap = origins.origins_overlap(problem, access_origins, &loan.origins);
    if !origin_overlap && !access_origins.is_empty() && !loan.origins.is_empty() {
        // A known unrelated origin is a different value generation. This exclusion applies to
        // projected places as well: structural place overlap alone cannot reconnect an old
        // projected value after its binding has been replaced.
        return None;
    }
    if !origin_overlap && structural_overlap == PlaceOverlap::Disjoint {
        return None;
    }

    Some((structural_overlap, origin_overlap))
}

fn access_kinds_conflict(left: AccessKind, right: AccessKind) -> bool {
    left == AccessKind::Exclusive || right == AccessKind::Exclusive
}

fn origins_for_access(
    problem: &BorrowProblem,
    origins: &OriginSolution,
    event: EventId,
    place: PlaceId,
) -> Box<[ValueOriginId]> {
    origins.origins_for_place_after_event(problem, event, place)
}

fn derive_alias_loans(
    problem: &BorrowProblem,
    origins: &OriginSolution,
    graph: &EventGraph,
    first_id: usize,
) -> Result<Vec<LoanFact>, CompilerError> {
    let mut result = Vec::new();
    for event in problem.events() {
        let Some((kind, source, destination)) = alias_event(event) else {
            continue;
        };
        if !origins.is_initial_alias_event(event.id) {
            continue;
        }
        let source_origins = origins_for_access(problem, origins, event.id, source);
        let kills = holder_kills(problem, origins, graph, event.id, destination);
        let uses = holder_uses(problem, origins, graph, event.id, destination, &kills)?;
        let holder_is_compiler_temporary = problem
            .places()
            .get(destination.index())
            .and_then(|place| problem.bindings().get(place.root.index()))
            .is_some_and(|binding| binding.compiler_temporary);
        if holder_is_compiler_temporary
            && !uses.is_empty()
            && uses
                .iter()
                .all(|use_id| graph.call_argument_uses.contains(use_id))
        {
            // The call-argument loan owns this exact temporary interval. An alias loan whose
            // holder is observed only as a call argument would otherwise create a second,
            // loop-carried static loan for the same compiler temporary.
            continue;
        }
        let id = next_loan_id(first_id + result.len())?;
        result.push(LoanFact {
            id,
            kind,
            issued_at: event.point,
            issue_event: Some(event.id),
            place: source,
            origins: source_origins,
            holders: vec![destination].into_boxed_slice(),
            uses: uses.into_boxed_slice(),
            kills,
            until_event: None,
            live_points: Box::new([]),
        });
    }
    for loan in &mut result {
        loan.live_points = graph.live_points(problem, loan).into_boxed_slice();
    }
    Ok(result)
}

fn derive_provenance_loans(
    problem: &BorrowProblem,
    origins: &OriginSolution,
    graph: &EventGraph,
    first_id: usize,
) -> Result<Vec<LoanFact>, CompilerError> {
    let mut result = Vec::new();
    for event in problem.events() {
        match &event.kind {
            EventKind::AliasFromPlace {
                source,
                destination,
            }
            | EventKind::ExclusiveAliasFromPlace {
                source,
                destination,
            }
            | EventKind::Alias {
                source,
                destination,
                ..
            }
            | EventKind::ExclusiveAlias {
                source,
                destination,
                ..
            } if origins.is_slot_rebind_event(event.id) => {
                // A mutable destination can remain slot-backed after an alias-valued rebind.
                // Preserve the represented value relationship without turning the raw HIR
                // event into a new write-through exclusive capability.
                // A slot-backed alias-valued rebind creates its new provenance relationship
                // from the value being assigned. The destination may also retain an
                // alias-backed alternative after a CFG join, so using its union here would
                // incorrectly attach stale origins to the new loan.
                let source_origins = provenance_source_origins(problem, origins, event, *source);
                push_provenance_loan(
                    problem,
                    origins,
                    graph,
                    &mut result,
                    first_id,
                    ProvenanceLoanSpec {
                        event,
                        kind: AccessKind::Shared,
                        source: *source,
                        holder: *destination,
                        loan_origins: source_origins,
                        fallback_origin: None,
                    },
                )?;
            }
            EventKind::Projection {
                destination,
                origin,
                source,
                ..
            } => {
                let output_origins = origins
                    .origins_after_event(event.id, *destination)
                    .map(|origins| origins.to_vec())
                    .unwrap_or_else(|| {
                        origins_for_access(problem, origins, event.id, *source).to_vec()
                    });
                push_provenance_loan(
                    problem,
                    origins,
                    graph,
                    &mut result,
                    first_id,
                    ProvenanceLoanSpec {
                        event,
                        kind: AccessKind::Shared,
                        source: *source,
                        holder: *destination,
                        loan_origins: if output_origins.is_empty() {
                            origins_for_access(problem, origins, event.id, *source)
                        } else {
                            output_origins.into_boxed_slice()
                        },
                        fallback_origin: Some(*origin),
                    },
                )?;
            }
            EventKind::Aggregate {
                destination,
                fields,
                ..
            } => {
                for field in fields {
                    let holder = projection_place(problem, *destination, field.projection)
                        .unwrap_or(*destination);
                    let field_origins =
                        origins_for_access(problem, origins, event.id, field.source);
                    push_provenance_loan(
                        problem,
                        origins,
                        graph,
                        &mut result,
                        first_id,
                        ProvenanceLoanSpec {
                            event,
                            kind: AccessKind::Shared,
                            source: field.source,
                            holder,
                            loan_origins: field_origins,
                            fallback_origin: None,
                        },
                    )?;
                }
            }
            EventKind::CallEffect(effect) => {
                let Some(result_row) = effect.result else {
                    continue;
                };
                let Some(origin_row) = problem.origins().get(result_row.origin.index()) else {
                    return Err(CompilerError::compiler_error(format!(
                        "Boracle loan solver cannot locate call-result origin {:?}",
                        result_row.origin
                    )));
                };
                let super::super::problem::OriginKind::CallResult { provenance, .. } =
                    &origin_row.kind
                else {
                    continue;
                };
                match provenance {
                    super::super::problem::CallResultProvenance::Fresh => {}
                    super::super::problem::CallResultProvenance::AliasParams(indices) => {
                        for index in indices {
                            let Some(argument) = effect.arguments.get(*index) else {
                                continue;
                            };
                            push_provenance_loan(
                                problem,
                                origins,
                                graph,
                                &mut result,
                                first_id,
                                ProvenanceLoanSpec {
                                    event,
                                    kind: AccessKind::Shared,
                                    source: argument.place,
                                    holder: result_row.place,
                                    loan_origins: origins_for_access(
                                        problem,
                                        origins,
                                        event.id,
                                        argument.place,
                                    ),
                                    fallback_origin: None,
                                },
                            )?;
                        }
                    }
                    super::super::problem::CallResultProvenance::Alias(_)
                    | super::super::problem::CallResultProvenance::Unknown => {
                        let result_origins =
                            origins_for_access(problem, origins, event.id, result_row.place);
                        let arguments = if effect.arguments.is_empty() {
                            vec![None]
                        } else {
                            effect
                                .arguments
                                .iter()
                                .map(|argument| Some(argument.place))
                                .collect()
                        };
                        for source in arguments {
                            let source_origins = source
                                .map(|place| origins_for_access(problem, origins, event.id, place))
                                .unwrap_or_else(|| result_origins.clone());
                            let loan_origins = if matches!(
                                provenance,
                                super::super::problem::CallResultProvenance::Unknown
                            ) || source_origins.is_empty()
                            {
                                result_origins.clone()
                            } else {
                                source_origins
                            };
                            push_provenance_loan(
                                problem,
                                origins,
                                graph,
                                &mut result,
                                first_id,
                                ProvenanceLoanSpec {
                                    event,
                                    kind: AccessKind::Shared,
                                    source: source.unwrap_or(result_row.place),
                                    holder: result_row.place,
                                    loan_origins,
                                    fallback_origin: None,
                                },
                            )?;
                        }
                    }
                }
            }
            _ => {}
        }
    }
    for loan in &mut result {
        loan.live_points = graph.live_points(problem, loan).into_boxed_slice();
    }
    Ok(result)
}

fn provenance_source_origins(
    problem: &BorrowProblem,
    origins: &OriginSolution,
    event: &Event,
    source: PlaceId,
) -> Box<[ValueOriginId]> {
    let explicit_origins = match &event.kind {
        EventKind::Alias { origins, .. } | EventKind::ExclusiveAlias { origins, .. } => {
            Some(origins.as_ref())
        }
        _ => None,
    };
    if let Some(explicit_origins) = explicit_origins
        && !explicit_origins.is_empty()
    {
        return explicit_origins.to_vec().into_boxed_slice();
    }
    origins_for_access(problem, origins, event.id, source)
}

struct ProvenanceLoanSpec<'a> {
    event: &'a Event,
    kind: AccessKind,
    source: PlaceId,
    holder: PlaceId,
    loan_origins: Box<[ValueOriginId]>,
    fallback_origin: Option<ValueOriginId>,
}

fn push_provenance_loan(
    problem: &BorrowProblem,
    origins: &OriginSolution,
    graph: &EventGraph,
    result: &mut Vec<LoanFact>,
    first_id: usize,
    spec: ProvenanceLoanSpec<'_>,
) -> Result<(), CompilerError> {
    let ProvenanceLoanSpec {
        event,
        kind,
        source,
        holder,
        mut loan_origins,
        fallback_origin,
    } = spec;
    if loan_origins.is_empty()
        && let Some(origin) = fallback_origin
    {
        loan_origins = vec![origin].into_boxed_slice();
    }
    if result.iter().any(|loan| {
        loan.issue_event == Some(event.id)
            && loan.kind == kind
            && loan.holders.as_ref() == [holder]
            && loan.origins.as_ref() == loan_origins.as_ref()
    }) {
        return Ok(());
    }
    let kills = holder_kills(problem, origins, graph, event.id, holder);
    let uses = holder_uses(problem, origins, graph, event.id, holder, &kills)?;
    let id = next_loan_id(first_id + result.len())?;
    result.push(LoanFact {
        id,
        kind,
        issued_at: event.point,
        issue_event: Some(event.id),
        place: source,
        origins: loan_origins,
        holders: vec![holder].into_boxed_slice(),
        uses: uses.into_boxed_slice(),
        kills,
        until_event: None,
        live_points: Box::new([]),
    });
    let loan = result
        .last_mut()
        .ok_or_else(|| CompilerError::compiler_error("Boracle provenance loan was not inserted"))?;
    loan.live_points = graph.live_points(problem, loan).into_boxed_slice();
    Ok(())
}

fn projection_place(
    problem: &BorrowProblem,
    base: PlaceId,
    projection: super::super::problem::ProjectionElem,
) -> Option<PlaceId> {
    let base = problem.places().get(base.index())?;
    problem.places().iter().find_map(|place| {
        (place.root == base.root
            && place.projections.len() == base.projections.len() + 1
            && place.projections.starts_with(&base.projections)
            && place.projections.last().copied() == Some(projection))
        .then_some(place.id)
    })
}

fn alias_event(event: &Event) -> Option<(AccessKind, PlaceId, PlaceId)> {
    match event.kind {
        EventKind::Alias {
            source,
            destination,
            ..
        }
        | EventKind::AliasFromPlace {
            source,
            destination,
        } => Some((AccessKind::Shared, source, destination)),
        EventKind::ExclusiveAlias {
            source,
            destination,
            ..
        }
        | EventKind::ExclusiveAliasFromPlace {
            source,
            destination,
        } => Some((AccessKind::Exclusive, source, destination)),
        _ => None,
    }
}

fn derive_call_argument_loans(
    problem: &BorrowProblem,
    origins: &OriginSolution,
    graph: &EventGraph,
    first_id: usize,
) -> Result<Vec<LoanFact>, CompilerError> {
    let mut result = Vec::new();
    for event in problem.events() {
        if let EventKind::CallArgument { argument, call, .. } = &event.kind {
            let until_event = graph.call_end_event(*call).or(Some(event.id));
            let id = next_loan_id(first_id + result.len())?;
            result.push(call_argument_loan(
                event,
                argument,
                until_event,
                origins_for_access(problem, origins, event.id, argument.place),
                id,
            ));
        }
    }
    for loan in &mut result {
        loan.live_points = graph.live_points(problem, loan).into_boxed_slice();
    }
    Ok(result)
}

fn call_argument_loan(
    event: &Event,
    argument: &super::super::problem::CallArgument,
    until_event: Option<EventId>,
    origins: Box<[ValueOriginId]>,
    id: LoanId,
) -> LoanFact {
    LoanFact {
        id,
        kind: argument.access,
        issued_at: event.point,
        issue_event: Some(event.id),
        place: argument.place,
        origins,
        holders: vec![argument.place].into_boxed_slice(),
        uses: vec![argument.use_id].into_boxed_slice(),
        kills: Box::new([]),
        until_event,
        live_points: Box::new([]),
    }
}

fn next_loan_id(index: usize) -> Result<LoanId, CompilerError> {
    u32::try_from(index)
        .map(LoanId::new)
        .map_err(|_| CompilerError::compiler_error("Boracle loan table is larger than u32::MAX"))
}

fn holder_uses(
    problem: &BorrowProblem,
    origins: &OriginSolution,
    graph: &EventGraph,
    issue_event: EventId,
    holder: PlaceId,
    kills: &[PointId],
) -> Result<Vec<UseId>, CompilerError> {
    let mut uses = Vec::new();
    for use_row in problem.uses() {
        let Some(event_id) = graph.event_by_use.get(&use_row.id).copied() else {
            return Err(CompilerError::compiler_error(format!(
                "Boracle loan solver cannot locate owner of holder use {:?}",
                use_row.id
            )));
        };
        if event_id == issue_event || !places_cover(problem, holder, use_row.place) {
            continue;
        }
        if use_row.definition && !origins.is_write_through_use(use_row.id) {
            continue;
        }
        if graph.reaches_without_kill(problem, issue_event, event_id, kills) {
            uses.push(use_row.id);
        }
    }
    uses.sort_by_key(|use_id| use_id.raw());
    uses.dedup();
    Ok(uses)
}

fn holder_kills(
    problem: &BorrowProblem,
    origins: &OriginSolution,
    graph: &EventGraph,
    issue_event: EventId,
    holder: PlaceId,
) -> Box<[PointId]> {
    let mut kills = Vec::new();
    for event in problem.events() {
        if event.id == issue_event
            || !graph.reaches_without_kill(problem, issue_event, event.id, &[])
        {
            continue;
        }
        let kills_holder = match &event.kind {
            EventKind::ScopeExit { bindings } => problem
                .places()
                .get(holder.index())
                .is_some_and(|place| bindings.contains(&place.root)),
            EventKind::Access { use_id } => {
                problem.uses().get(use_id.index()).is_some_and(|use_row| {
                    use_row.kind == UseKind::Write
                        && use_row.definition
                        && !origins.is_write_through_use(*use_id)
                        && places_overlap(problem, holder, use_row.place) != PlaceOverlap::Disjoint
                })
            }
            _ if origins.is_write_through_event(event.id) => false,
            _ => event_destination(event).is_some_and(|destination| {
                places_overlap(problem, holder, destination) != PlaceOverlap::Disjoint
            }),
        };
        if kills_holder {
            kills.push(event.point);
        }
    }
    kills.sort_by_key(|point| point.raw());
    kills.dedup();
    kills.into_boxed_slice()
}

fn event_destination(event: &Event) -> Option<PlaceId> {
    match &event.kind {
        EventKind::Fresh { destination, .. }
        | EventKind::Copy { destination, .. }
        | EventKind::Projection { destination, .. }
        | EventKind::Rebind { destination, .. }
        | EventKind::Aggregate { destination, .. }
        | EventKind::Alias { destination, .. }
        | EventKind::AliasFromPlace { destination, .. }
        | EventKind::ExclusiveAlias { destination, .. }
        | EventKind::ExclusiveAliasFromPlace { destination, .. } => Some(*destination),
        EventKind::CallEffect(effect) => effect.result.map(|result| result.place),
        _ => None,
    }
}

fn places_cover(problem: &BorrowProblem, holder: PlaceId, observed: PlaceId) -> bool {
    let Some(holder) = problem.places().get(holder.index()) else {
        return false;
    };
    let Some(observed) = problem.places().get(observed.index()) else {
        return false;
    };
    holder.root == observed.root
        && holder.projections.len() <= observed.projections.len()
        && observed.projections.starts_with(&holder.projections)
}

fn places_overlap(problem: &BorrowProblem, left: PlaceId, right: PlaceId) -> PlaceOverlap {
    let Some(left) = problem.places().get(left.index()) else {
        return PlaceOverlap::Conservative;
    };
    let Some(right) = problem.places().get(right.index()) else {
        return PlaceOverlap::Conservative;
    };
    left.overlap(right)
}

struct EventGraph {
    events_by_block: BTreeMap<BlockId, Vec<EventId>>,
    successors: BTreeMap<BlockId, Vec<BlockId>>,
    event_location: BTreeMap<EventId, (BlockId, usize)>,
    event_by_use: BTreeMap<UseId, EventId>,
    call_effect_events: BTreeMap<super::super::problem::CallId, EventId>,
    call_argument_uses: BTreeSet<UseId>,
    exclusive_liveness: ExclusiveLoanLiveness,
}

impl EventGraph {
    fn new(
        problem: &BorrowProblem,
        exclusive_liveness: ExclusiveLoanLiveness,
    ) -> Result<Self, CompilerError> {
        let mut events_by_block = BTreeMap::new();
        let mut event_location = BTreeMap::new();
        for block in &problem.control_flow().blocks {
            events_by_block.insert(block.id, block.events.to_vec());
            for (index, event) in block.events.iter().enumerate() {
                event_location.insert(*event, (block.id, index));
            }
        }

        let mut event_by_use = BTreeMap::new();
        let mut call_effect_events = BTreeMap::new();
        let mut call_argument_uses = BTreeSet::new();
        for event in problem.events() {
            match &event.kind {
                EventKind::Access { use_id } => {
                    event_by_use.insert(*use_id, event.id);
                }
                EventKind::CallArgument { call, argument, .. } => {
                    event_by_use.insert(argument.use_id, event.id);
                    call_argument_uses.insert(argument.use_id);
                    let _ = call;
                }
                EventKind::CallEffect(effect) => {
                    call_effect_events.insert(effect.call, event.id);
                }
                _ => {}
            }
        }

        let mut successors = BTreeMap::new();
        for block in &problem.control_flow().blocks {
            successors.insert(block.id, Vec::new());
        }
        for edge in &problem.control_flow().edges {
            successors.entry(edge.from).or_default().push(edge.to);
        }
        for targets in successors.values_mut() {
            targets.sort_by_key(|block| block.raw());
            targets.dedup();
        }

        if event_location.len() != problem.events().len() {
            return Err(CompilerError::compiler_error(
                "Boracle loan solver found an event without CFG ownership",
            ));
        }
        Ok(Self {
            events_by_block,
            successors,
            event_location,
            event_by_use,
            call_effect_events,
            call_argument_uses,
            exclusive_liveness,
        })
    }

    fn explicit_loan(
        &self,
        problem: &BorrowProblem,
        origins: &OriginSolution,
        loan: &Loan,
    ) -> Result<LoanFact, CompilerError> {
        let issue_event = problem.events().iter().find_map(|event| {
            matches!(event.kind, EventKind::LoanIssue { loan: id } if id == loan.id)
                .then_some(event.id)
        });
        let Some(issue_event) = issue_event else {
            return Err(CompilerError::compiler_error(format!(
                "Boracle loan solver cannot locate issue event for loan {:?}",
                loan.id
            )));
        };
        let loan_origins = if loan.origins.is_empty() {
            origins_for_access(problem, origins, issue_event, loan.place)
        } else {
            loan.origins.clone()
        };
        let mut fact = LoanFact {
            id: loan.id,
            kind: loan.kind,
            issued_at: loan.issued_at,
            issue_event: Some(issue_event),
            place: loan.place,
            origins: loan_origins,
            holders: loan.holders.clone(),
            uses: loan.uses.clone(),
            kills: loan.kills.clone(),
            until_event: None,
            live_points: Box::new([]),
        };
        fact.live_points = self.live_points(problem, &fact).into_boxed_slice();
        Ok(fact)
    }

    fn call_end_event(&self, call: super::super::problem::CallId) -> Option<EventId> {
        self.call_effect_events.get(&call).copied()
    }

    fn access_origins(
        &self,
        problem: &BorrowProblem,
        origins: &OriginSolution,
        event: &Event,
        access: &AccessFact,
    ) -> Box<[ValueOriginId]> {
        origins_for_access(problem, origins, event.id, access.place)
    }

    fn next_event(&self, event: EventId) -> Option<EventId> {
        let (block, index) = self.event_location.get(&event).copied()?;
        self.events_by_block
            .get(&block)
            .and_then(|events| events.get(index + 1))
            .copied()
    }

    fn loan_live_at_event(
        &self,
        problem: &BorrowProblem,
        loan: &LoanFact,
        target_event: EventId,
    ) -> bool {
        let Some(issue_event) = loan.issue_event else {
            return false;
        };
        if issue_event == target_event {
            return loan.until_event.is_some();
        }
        if !self.reaches_without_kill(problem, issue_event, target_event, &loan.kills) {
            return false;
        }
        if problem
            .events()
            .get(target_event.index())
            .is_some_and(|event| loan.kills.contains(&event.point))
        {
            // The access that replaces a holder is still protected by the old loan. The kill
            // takes effect only after that event has been checked for a conflict.
            return true;
        }
        if let Some(until_event) = loan.until_event {
            return target_event == until_event
                || self.reaches_before_barrier(
                    problem,
                    issue_event,
                    target_event,
                    until_event,
                    &loan.kills,
                );
        }
        if loan.kind == AccessKind::Exclusive
            && loan.uses.is_empty()
            && self.exclusive_liveness == ExclusiveLoanLiveness::Conservative
        {
            // The reference rule keeps a dead exclusive capability conservative. The named
            // dead-exclusive experiment opts into use-driven liveness instead.
            return true;
        }
        self.keeping_use(problem, loan, target_event).is_some()
    }

    fn live_points(&self, problem: &BorrowProblem, loan: &LoanFact) -> Vec<PointId> {
        let mut points = problem
            .points()
            .iter()
            .filter(|point| {
                self.events_by_block
                    .get(&point.block)
                    .into_iter()
                    .flatten()
                    .any(|event_id| {
                        problem.events().get(event_id.index()).is_some_and(|event| {
                            event.point == point.id
                                && self.loan_live_at_event(problem, loan, *event_id)
                        })
                    })
            })
            .map(|point| point.id)
            .collect::<Vec<_>>();
        points.sort_by_key(|point| point.raw());
        points.dedup();
        points
    }

    fn keeping_use(
        &self,
        problem: &BorrowProblem,
        loan: &LoanFact,
        target_event: EventId,
    ) -> Option<UseId> {
        let mut candidates = loan
            .uses
            .iter()
            .copied()
            .filter(|use_id| {
                self.event_by_use
                    .get(use_id)
                    .is_some_and(|event| *event != target_event)
            })
            .filter(|use_id| {
                self.reaches_without_kill(
                    problem,
                    target_event,
                    self.event_by_use[use_id],
                    &loan.kills,
                )
            })
            .collect::<Vec<_>>();
        candidates.sort_by_key(|use_id| use_id.raw());
        candidates.into_iter().next()
    }

    fn reaches_before_barrier(
        &self,
        problem: &BorrowProblem,
        from_event: EventId,
        target_event: EventId,
        barrier_event: EventId,
        kills: &[PointId],
    ) -> bool {
        let Some(&(from_block, from_index)) = self.event_location.get(&from_event) else {
            return false;
        };
        let mut queue = VecDeque::from([(from_block, from_index + 1)]);
        let mut visited = BTreeSet::new();
        while let Some((block, index)) = queue.pop_front() {
            if !visited.insert((block, index)) {
                continue;
            }
            let Some(events) = self.events_by_block.get(&block) else {
                continue;
            };
            if index >= events.len() {
                queue.extend(
                    self.successors
                        .get(&block)
                        .into_iter()
                        .flatten()
                        .map(|successor| (*successor, 0)),
                );
                continue;
            }
            let event_id = events[index];
            if event_id == barrier_event {
                continue;
            }
            if event_id == target_event {
                return true;
            }
            let Some(event) = problem.events().get(event_id.index()) else {
                continue;
            };
            if kills.contains(&event.point) {
                continue;
            }
            queue.push_back((block, index + 1));
        }
        false
    }

    fn reaches_without_kill(
        &self,
        problem: &BorrowProblem,
        from_event: EventId,
        target_event: EventId,
        kills: &[PointId],
    ) -> bool {
        let Some(&(from_block, from_index)) = self.event_location.get(&from_event) else {
            return false;
        };
        let mut queue = VecDeque::from([(from_block, from_index + 1)]);
        let mut visited = BTreeSet::new();
        while let Some((block, index)) = queue.pop_front() {
            if !visited.insert((block, index)) {
                continue;
            }
            let Some(events) = self.events_by_block.get(&block) else {
                continue;
            };
            if index >= events.len() {
                queue.extend(
                    self.successors
                        .get(&block)
                        .into_iter()
                        .flatten()
                        .map(|successor| (*successor, 0)),
                );
                continue;
            }
            let event_id = events[index];
            if event_id == target_event {
                return true;
            }
            let Some(event) = problem.events().get(event_id.index()) else {
                continue;
            };
            if kills.contains(&event.point) {
                continue;
            }
            queue.push_back((block, index + 1));
        }
        false
    }
}
