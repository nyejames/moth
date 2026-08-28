//! Binding-independent origin propagation for the Boracle reference solver.
//!
//! WHAT: evaluates normalized definition, alias, copy, projection, aggregate and call-result
//! events into deterministic origin sets at CFG points.
//! WHY: a binding can be rebound while older aliases remain meaningful; provenance therefore
//! belongs to values and event flow, not to local names.

// Some provenance rows are consumed by focused tests and future differential queries rather than
// by every current dump. Keep the complete typed result surface warning-free.
#![allow(dead_code)]

use super::super::problem::{
    BindingId, BorrowProblem, CallResultProvenance, Event, EventId, EventKind, OriginKind, PlaceId,
    ProjectionElem, UseId, UseKind, ValueOrigin, ValueOriginId,
};
use super::relations::{
    CopyGraphId, OriginRegistration, OriginRelation, OriginRelations, PrecisionLossReason,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use std::collections::{BTreeMap, BTreeSet};

type OriginSet = BTreeSet<ValueOriginId>;
type OriginState = BTreeMap<PlaceId, OriginSet>;
type OriginAlternatives = BTreeMap<(PlaceId, BindingMode), OriginSet>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum BindingMode {
    Alias,
    Slot,
    /// Both alias write-through and slot replacement remain possible after a CFG join.
    Mixed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FlowState {
    origins: OriginState,
    alternatives: OriginAlternatives,
    modes: BTreeMap<BindingId, BindingMode>,
}

/// A stable reason attached to one propagated provenance fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OriginTraceRule {
    Noop,
    Fresh,
    Alias,
    /// A CFG join leaves both alias-write-through and slot-replacement paths possible.
    Mixed,
    WriteThrough,
    Copy,
    Projection,
    Rebind,
    Aggregate,
    CallResult,
    ScopeExit,
}

/// One place's possible origins after a normalized event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OriginFact {
    pub(crate) event: EventId,
    pub(crate) place: PlaceId,
    pub(crate) origins: Box<[ValueOriginId]>,
}

/// One deterministic explanation for a propagated place state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OriginTrace {
    pub(crate) event: EventId,
    pub(crate) rule: OriginTraceRule,
    pub(crate) destination: Option<PlaceId>,
    pub(crate) input_origins: Box<[ValueOriginId]>,
    pub(crate) output_origins: Box<[ValueOriginId]>,
}

/// Complete origin solution for one normalized problem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OriginSolution {
    facts: Box<[OriginFact]>,
    traces: Box<[OriginTrace]>,
    state_after_event: BTreeMap<(EventId, PlaceId), Box<[ValueOriginId]>>,
    write_through_uses: Box<[UseId]>,
    relations: OriginRelations,
}

impl OriginSolution {
    pub(crate) fn facts(&self) -> &[OriginFact] {
        &self.facts
    }

    pub(crate) fn traces(&self) -> &[OriginTrace] {
        &self.traces
    }

    pub(crate) fn origins_after_event(
        &self,
        event: EventId,
        place: PlaceId,
    ) -> Option<&[ValueOriginId]> {
        self.state_after_event.get(&(event, place)).map(Box::as_ref)
    }

    pub(crate) fn is_write_through_use(&self, use_id: UseId) -> bool {
        self.write_through_uses.contains(&use_id)
    }

    pub(crate) fn is_initial_alias_event(&self, event: EventId) -> bool {
        let event_traces = self.traces.iter().filter(|trace| trace.event == event);
        let mut saw_alias = false;
        for trace in event_traces {
            match trace.rule {
                OriginTraceRule::Alias => saw_alias = true,
                OriginTraceRule::Mixed
                | OriginTraceRule::Rebind
                | OriginTraceRule::WriteThrough => return false,
                _ => {}
            }
        }
        saw_alias
    }

    pub(crate) fn is_write_through_event(&self, event: EventId) -> bool {
        self.traces.iter().any(|trace| {
            trace.event == event
                && matches!(
                    trace.rule,
                    OriginTraceRule::Mixed | OriginTraceRule::WriteThrough
                )
        })
    }

    pub(crate) fn is_slot_rebind_event(&self, event: EventId) -> bool {
        let mut saw_rebind = false;
        for trace in self.traces.iter().filter(|trace| trace.event == event) {
            match trace.rule {
                OriginTraceRule::Mixed | OriginTraceRule::Rebind => saw_rebind = true,
                OriginTraceRule::WriteThrough => return false,
                _ => {}
            }
        }
        saw_rebind
    }

    pub(crate) fn relations(&self) -> &OriginRelations {
        &self.relations
    }

    pub(crate) fn origins_for_place_after_event(
        &self,
        problem: &BorrowProblem,
        event: EventId,
        place: PlaceId,
    ) -> Box<[ValueOriginId]> {
        origins_recovered_after_event(problem, &self.state_after_event, event, place)
    }

    pub(crate) fn debug_dump(&self) -> String {
        format!("{self:#?}")
    }
}

/// Forward fixed-point origin solver over the normalized CFG.
pub(crate) struct OriginSolver;

impl OriginSolver {
    pub(crate) fn solve(problem: &BorrowProblem) -> Result<OriginSolution, CompilerError> {
        problem.validate()?;

        let block_count = problem.control_flow().blocks.len();
        let mut predecessors = vec![Vec::<usize>::new(); block_count];
        for edge in &problem.control_flow().edges {
            predecessors[edge.to.index()].push(edge.from.index());
        }
        for predecessor_list in &mut predecessors {
            predecessor_list.sort_unstable();
            predecessor_list.dedup();
        }

        let mut entry_states = vec![None::<FlowState>; block_count];
        entry_states[problem.control_flow().entry.index()] = Some(FlowState {
            origins: BTreeMap::new(),
            alternatives: BTreeMap::new(),
            modes: BTreeMap::new(),
        });
        let mut output_states = vec![
            FlowState {
                origins: BTreeMap::new(),
                alternatives: BTreeMap::new(),
                modes: BTreeMap::new(),
            };
            block_count
        ];

        let mut changed = true;
        while changed {
            changed = false;
            for block_index in 0..block_count {
                let block_id = problem.control_flow().blocks[block_index].id;
                let entry = if block_id == problem.control_flow().entry {
                    join_predecessors(
                        &entry_states,
                        &output_states,
                        &predecessors[block_index],
                        Some(FlowState {
                            origins: BTreeMap::new(),
                            alternatives: BTreeMap::new(),
                            modes: BTreeMap::new(),
                        }),
                    )
                } else {
                    join_predecessors(
                        &entry_states,
                        &output_states,
                        &predecessors[block_index],
                        None,
                    )
                };
                let Some(entry) = entry else {
                    continue;
                };
                if entry_states[block_index].as_ref() != Some(&entry) {
                    entry_states[block_index] = Some(entry.clone());
                    changed = true;
                }
                let output = apply_block(problem, block_index, entry, None)?.0;
                if output_states[block_index] != output {
                    output_states[block_index] = output;
                    changed = true;
                }
            }
        }

        let mut facts = Vec::new();
        let mut traces = Vec::new();
        let mut state_after_event = BTreeMap::new();
        let mut write_through_uses = BTreeSet::new();
        for (block_index, _) in problem.control_flow().blocks.iter().enumerate() {
            let Some(entry) = entry_states[block_index].clone() else {
                continue;
            };
            let (_, _, _, block_states, block_write_through_uses) =
                apply_block(problem, block_index, entry, Some((&mut facts, &mut traces)))?;
            for ((event, place), origins) in block_states {
                state_after_event.insert((event, place), origins);
            }
            write_through_uses.extend(block_write_through_uses);
        }

        facts.sort_by_key(|fact| (fact.event.raw(), fact.place.raw()));
        traces.sort_by_key(|trace| trace.event.raw());

        let relations = origin_relations(problem, &traces, &state_after_event)?;

        Ok(OriginSolution {
            facts: facts.into_boxed_slice(),
            traces: traces.into_boxed_slice(),
            state_after_event,
            write_through_uses: write_through_uses.into_iter().collect(),
            relations,
        })
    }
}

// ------------------------
//  Origin relation construction
// ------------------------

/// Build the typed relation table for one solved origin problem.
///
/// Rows are extracted from the normalized origin rows, the solved traces and the per-event
/// state: identity stays `ValueOriginId` equality, projection rows keep their actual
/// source-to-derived direction, same-event place ancestry owns base/child containment, and
/// explicit copies publish positive disjointness. Every other coexistence between two solved
/// generations becomes a may-alias precision loss. Mixed traces contribute no rows at all
/// because their output sets union independent generations that only identity may relate.
fn origin_relations(
    problem: &BorrowProblem,
    traces: &[OriginTrace],
    state_after_event: &BTreeMap<(EventId, PlaceId), Box<[ValueOriginId]>>,
) -> Result<OriginRelations, CompilerError> {
    let top_reasons = top_like_reasons(problem);
    let mut registrations = Vec::new();
    let mut rows = Vec::new();
    let mut mixed_generation_sets = Vec::new();

    for origin in problem.origins() {
        registrations.push(match top_reasons.get(&origin.id).copied().flatten() {
            Some(reason) => OriginRegistration::unknown(origin.id, reason),
            None => independent_registration(origin),
        });

        // A projection row is only honest when its source is a real generation. Builder rows
        // share one Unknown placeholder as their source and stay conservative through their
        // unknown registration instead of inventing a row from it.
        if let OriginKind::Projection { source, projection } = &origin.kind
            && source != &origin.id
            && top_reasons.get(source).copied().flatten().is_none()
        {
            rows.push(OriginRelation::projection(*source, origin.id, *projection));
        }

        // A joined origin represents one of the joined path generations. Fixture rows are the
        // only producer, so the possible-overlap fact keeps its explicit path reason.
        if let OriginKind::Join(members) = &origin.kind {
            for member in members.iter().copied() {
                if member != origin.id {
                    rows.push(OriginRelation::may_alias(
                        origin.id,
                        member,
                        PrecisionLossReason::PathJoin,
                    ));
                }
            }
        }
    }

    for trace in traces {
        if trace.rule == OriginTraceRule::Mixed {
            mixed_generation_sets.push(trace.output_origins.clone());
            continue;
        }

        let Some(event) = problem.events().get(trace.event.index()) else {
            continue;
        };
        match trace.rule {
            OriginTraceRule::Aggregate => {
                emit_aggregate_child_rows(problem, state_after_event, trace, event, &mut rows);
            }
            OriginTraceRule::Copy | OriginTraceRule::Noop | OriginTraceRule::ScopeExit => {}
            OriginTraceRule::Projection => {
                emit_projection_trace_rows(problem, event, state_after_event, &mut rows);
            }
            OriginTraceRule::Mixed => {}
            OriginTraceRule::Alias
            | OriginTraceRule::Rebind
            | OriginTraceRule::WriteThrough
            | OriginTraceRule::Fresh
            | OriginTraceRule::CallResult => {
                // A write through an alias-only binding preserves the referent generation while
                // the written generation flows through the same access path, so such a pair stays
                // a may-alias precision loss. Flow-preserving rules only restate identity pairs.
                for output in &trace.output_origins {
                    for input in &trace.input_origins {
                        if output != input {
                            rows.push(OriginRelation::may_alias(
                                *output,
                                *input,
                                PrecisionLossReason::PathJoin,
                            ));
                        }
                    }
                }
            }
        }
    }

    // Base/child containment at one event. The directional rows keep sibling fields
    // unrelated: a transitive closure would reconnect generations the solver keeps apart.
    for ((event, ancestor_place), ancestor_origins) in state_after_event {
        for ((candidate_event, descendant_place), descendant_origins) in state_after_event {
            if event != candidate_event
                || !is_strict_place_ancestor(problem, *ancestor_place, *descendant_place)
            {
                continue;
            }
            let Some(step) = ancestry_step(problem, *ancestor_place, *descendant_place) else {
                continue;
            };
            for ancestor_origin in ancestor_origins {
                for descendant_origin in descendant_origins {
                    if ancestor_origin != descendant_origin {
                        rows.push(OriginRelation::aggregate_child(
                            *ancestor_origin,
                            *descendant_origin,
                            step,
                        ));
                    }
                }
            }
        }
    }

    emit_copy_correspondence_rows(problem, traces, state_after_event, &top_reasons, &mut rows);

    Ok(
        OriginRelations::new(registrations, rows)?
            .with_mixed_generation_sets(mixed_generation_sets),
    )
}

#[derive(Debug, Clone, Copy)]
enum TopLikeWalk {
    InProgress,
    Done(Option<PrecisionLossReason>),
}

/// Classify every origin's top-like uncertainty together with the reason that caused it.
///
/// This keeps the previous overlap helper's walk as registration evidence: Unknown rows,
/// unknown call results and every alias, join, projection or alias-provenance derivation that
/// reaches one are conservative. Memoized walking replaces the previous unguarded recursion.
fn top_like_reasons(
    problem: &BorrowProblem,
) -> BTreeMap<ValueOriginId, Option<PrecisionLossReason>> {
    let mut memo: BTreeMap<ValueOriginId, TopLikeWalk> = BTreeMap::new();
    for origin in problem.origins() {
        top_like_reason(problem, origin.id, &mut memo);
    }

    memo.into_iter()
        .map(|(origin, walk)| (origin, finished_walk(walk)))
        .collect()
}

fn finished_walk(walk: TopLikeWalk) -> Option<PrecisionLossReason> {
    match walk {
        TopLikeWalk::Done(reason) => reason,
        TopLikeWalk::InProgress => Some(PrecisionLossReason::MissingLocalSummary),
    }
}

fn top_like_reason(
    problem: &BorrowProblem,
    origin_id: ValueOriginId,
    memo: &mut BTreeMap<ValueOriginId, TopLikeWalk>,
) -> Option<PrecisionLossReason> {
    match memo.get(&origin_id) {
        Some(TopLikeWalk::Done(reason)) => return *reason,

        // A derivation cycle cannot be proven independent. Conservatively treat the back edge
        // as unknown so an unknown member cannot be forgotten because of visit order.
        Some(TopLikeWalk::InProgress) => {
            return Some(PrecisionLossReason::MissingLocalSummary);
        }
        None => {}
    }

    memo.insert(origin_id, TopLikeWalk::InProgress);
    let reason = match problem.origins().get(origin_id.index()) {
        None => Some(PrecisionLossReason::MissingLocalSummary),

        Some(origin) => match &origin.kind {
            OriginKind::Unknown => Some(PrecisionLossReason::MissingLocalSummary),

            OriginKind::CallResult {
                provenance: CallResultProvenance::Unknown,
                ..
            } => Some(PrecisionLossReason::UnknownCallResult),

            OriginKind::Projection { source, .. } => top_like_reason(problem, *source, memo),

            OriginKind::Alias(members)
            | OriginKind::ExclusiveAlias(members)
            | OriginKind::Join(members) => derivation_member_reason(problem, members, memo),

            OriginKind::CallResult {
                provenance: CallResultProvenance::Alias(members),
                ..
            } => derivation_member_reason(problem, members, memo),

            OriginKind::Copy(_)
            | OriginKind::Parameter { .. }
            | OriginKind::Fresh
            | OriginKind::CallResult { .. } => None,
        },
    };

    memo.insert(origin_id, TopLikeWalk::Done(reason));
    reason
}

fn derivation_member_reason(
    problem: &BorrowProblem,
    members: &[ValueOriginId],
    memo: &mut BTreeMap<ValueOriginId, TopLikeWalk>,
) -> Option<PrecisionLossReason> {
    members
        .iter()
        .filter_map(|member| top_like_reason(problem, *member, memo))
        .min()
}

/// Registration for one origin that is not top-like.
///
/// Alias, exclusive-alias and alias-provenance call-result rows have no current producer.
/// They stay derived rather than claiming a fresh independent generation their member list
/// cannot prove, so any pair without an explicit row remains conservatively unknown.
fn independent_registration(origin: &ValueOrigin) -> OriginRegistration {
    match &origin.kind {
        OriginKind::Alias(_)
        | OriginKind::ExclusiveAlias(_)
        | OriginKind::CallResult {
            provenance: CallResultProvenance::Alias(_),
            ..
        } => OriginRegistration::derived(origin.id),

        _ => OriginRegistration::fresh(origin.id),
    }
}

/// Emit typed containment rows for one aggregate event.
///
/// The event's fields carry the actual per-child projection path, so each stored child
/// generation gets one directional row. Input generations the field rows cannot explain are
/// still related as a may-alias precision loss so trace flow never silently disappears.
fn emit_aggregate_child_rows(
    problem: &BorrowProblem,
    state_after_event: &BTreeMap<(EventId, PlaceId), Box<[ValueOriginId]>>,
    trace: &OriginTrace,
    event: &Event,
    rows: &mut Vec<OriginRelation>,
) {
    let EventKind::Aggregate {
        destination,
        fields,
        ..
    } = &event.kind
    else {
        return;
    };

    let mut explained_pairs = BTreeSet::new();
    for field in fields.iter() {
        let Some(child_place) = projected_place(problem, *destination, field.projection) else {
            continue;
        };
        let field_origins =
            origins_recovered_after_event(problem, state_after_event, trace.event, child_place);
        for field_origin in field_origins {
            for output in &trace.output_origins {
                if field_origin != *output {
                    rows.push(OriginRelation::aggregate_child(
                        *output,
                        field_origin,
                        field.projection,
                    ));
                    explained_pairs.insert(normalized_origin_pair(*output, field_origin));
                }
            }
        }
    }

    for output in &trace.output_origins {
        for input in &trace.input_origins {
            if output != input
                && !explained_pairs.contains(&normalized_origin_pair(*output, *input))
            {
                rows.push(OriginRelation::may_alias(
                    *output,
                    *input,
                    PrecisionLossReason::PathJoin,
                ));
            }
        }
    }
}

/// Emit positive copy-disjointness rows.
///
/// Each copy event owns one correspondence graph between the read source generation and the
/// independent result generation. The row is only published when the copy actually replaced
/// the destination generation: a write-through copy keeps the old referent generation, so the
/// copied result never became observable. Pairs that another rule already forces to overlap
/// keep that stronger fact, and unknown-provenance origins never receive a disjointness row
/// because top-like uncertainty must stay conservative.
fn emit_copy_correspondence_rows(
    problem: &BorrowProblem,
    traces: &[OriginTrace],
    state_after_event: &BTreeMap<(EventId, PlaceId), Box<[ValueOriginId]>>,
    top_reasons: &BTreeMap<ValueOriginId, Option<PrecisionLossReason>>,
    rows: &mut Vec<OriginRelation>,
) {
    let forced_pairs: BTreeSet<(ValueOriginId, ValueOriginId)> = rows
        .iter()
        .map(|row| normalized_origin_pair(row.left, row.right))
        .collect();

    let mut copy_graphs = 0u32;
    for trace in traces {
        if trace.rule != OriginTraceRule::Copy {
            continue;
        }
        let Some(event) = problem.events().get(trace.event.index()) else {
            continue;
        };
        let EventKind::Copy {
            destination,
            origin,
            ..
        } = &event.kind
        else {
            continue;
        };
        let copy_graph = CopyGraphId::new(copy_graphs);
        copy_graphs += 1;

        let Some(destination_origins) = state_after_event.get(&(event.id, *destination)) else {
            continue;
        };
        if !destination_origins.contains(origin) {
            continue;
        }

        for source_origin in &trace.input_origins {
            if source_origin == origin
                || top_reasons.get(source_origin).copied().flatten().is_some()
                || top_reasons.get(origin).copied().flatten().is_some()
                || forced_pairs.contains(&normalized_origin_pair(*source_origin, *origin))
            {
                continue;
            }
            rows.push(OriginRelation::copy_correspondence(
                *source_origin,
                *origin,
                copy_graph,
            ));
        }
    }
}

fn emit_projection_trace_rows(
    problem: &BorrowProblem,
    event: &Event,
    state_after_event: &BTreeMap<(EventId, PlaceId), Box<[ValueOriginId]>>,
    rows: &mut Vec<OriginRelation>,
) {
    let EventKind::Projection { source, origin, .. } = &event.kind else {
        return;
    };
    let Some(projection) = problem
        .origins()
        .get(origin.index())
        .and_then(|row| match &row.kind {
            OriginKind::Projection { projection, .. } => Some(*projection),
            _ => None,
        })
    else {
        return;
    };

    // Use the source place's own generation, not identity-preserving child recovery.
    // Projecting an aggregate must not claim the stored field generation as the derived
    // origin's source.
    let Some(source_origins) = state_after_event.get(&(event.id, *source)) else {
        return;
    };
    for source_origin in source_origins {
        if source_origin != origin {
            rows.push(OriginRelation::projection(
                *source_origin,
                *origin,
                projection,
            ));
        }
    }
}
fn is_strict_place_ancestor(
    problem: &BorrowProblem,
    ancestor: PlaceId,
    descendant: PlaceId,
) -> bool {
    let Some(ancestor) = problem.places().get(ancestor.index()) else {
        return false;
    };
    let Some(descendant) = problem.places().get(descendant.index()) else {
        return false;
    };
    ancestor.root == descendant.root
        && ancestor.projections.len() < descendant.projections.len()
        && descendant.projections.starts_with(&ancestor.projections)
}

/// The single projection step that leads from one place to its strict descendant.
fn ancestry_step(
    problem: &BorrowProblem,
    ancestor: PlaceId,
    descendant: PlaceId,
) -> Option<ProjectionElem> {
    let ancestor_row = problem.places().get(ancestor.index())?;
    let descendant_row = problem.places().get(descendant.index())?;

    descendant_row
        .projections
        .get(ancestor_row.projections.len())
        .copied()
}

/// Recover one place's origin state after an event, unioning same-root ancestor states when
/// the exact place has no recorded state.
fn origins_recovered_after_event(
    problem: &BorrowProblem,
    state_after_event: &BTreeMap<(EventId, PlaceId), Box<[ValueOriginId]>>,
    event: EventId,
    place: PlaceId,
) -> Box<[ValueOriginId]> {
    if let Some(origins) = state_after_event.get(&(event, place)) {
        return origins.clone();
    }

    let Some(place_row) = problem.places().get(place.index()) else {
        return Box::new([]);
    };
    let mut origins = BTreeSet::new();
    for ((candidate_event, candidate_place), candidate_origins) in state_after_event {
        if *candidate_event != event {
            continue;
        }
        let Some(candidate_row) = problem.places().get(candidate_place.index()) else {
            continue;
        };
        if candidate_row.root == place_row.root
            && candidate_row.projections.len() <= place_row.projections.len()
            && place_row
                .projections
                .starts_with(&candidate_row.projections)
        {
            origins.extend(candidate_origins.iter().copied());
        }
    }
    origins.into_iter().collect()
}

fn normalized_origin_pair(
    left: ValueOriginId,
    right: ValueOriginId,
) -> (ValueOriginId, ValueOriginId) {
    if left <= right {
        (left, right)
    } else {
        (right, left)
    }
}

fn join_predecessors(
    entry_states: &[Option<FlowState>],
    output_states: &[FlowState],
    predecessors: &[usize],
    initial: Option<FlowState>,
) -> Option<FlowState> {
    let mut joined = initial;
    for predecessor in predecessors {
        let Some(state) = entry_states[*predecessor]
            .as_ref()
            .map(|_| &output_states[*predecessor])
        else {
            continue;
        };
        merge_state(
            joined.get_or_insert_with(|| FlowState {
                origins: BTreeMap::new(),
                alternatives: BTreeMap::new(),
                modes: BTreeMap::new(),
            }),
            state,
        );
    }
    joined
}

fn merge_state(destination: &mut FlowState, source: &FlowState) {
    for ((place, mode), origins) in &source.alternatives {
        destination
            .alternatives
            .entry((*place, *mode))
            .or_default()
            .extend(origins.iter().copied());
    }
    for (binding, mode) in &source.modes {
        destination
            .modes
            .entry(*binding)
            .and_modify(|existing| *existing = join_mode(*existing, *mode))
            .or_insert(*mode);
    }
    rebuild_origins(destination);
}

type ApplyCapture<'a> = Option<(&'a mut Vec<OriginFact>, &'a mut Vec<OriginTrace>)>;
type AppliedBlock = (
    FlowState,
    Vec<OriginFact>,
    Vec<OriginTrace>,
    BTreeMap<(EventId, PlaceId), Box<[ValueOriginId]>>,
    BTreeSet<UseId>,
);

fn apply_block(
    problem: &BorrowProblem,
    block_index: usize,
    mut state: FlowState,
    mut capture: ApplyCapture<'_>,
) -> Result<AppliedBlock, CompilerError> {
    let block = &problem.control_flow().blocks[block_index];
    let mut local_facts = Vec::new();
    let mut local_traces = Vec::new();
    let mut local_states = BTreeMap::new();
    let mut local_write_through_uses = BTreeSet::new();
    let mut pending_write = None;
    for (event_index, event_id) in block.events.iter().enumerate() {
        let event = problem.events().get(event_id.index()).ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "Boracle origin solver cannot locate event {:?}",
                event_id
            ))
        })?;
        let (rule, destination, inputs) = apply_event(problem, event, &mut state)?;
        if matches!(rule, OriginTraceRule::Mixed | OriginTraceRule::WriteThrough)
            && let Some(destination) = destination
            && let Some((place, use_id)) = pending_write
            && place == destination
        {
            local_write_through_uses.insert(use_id);
        }
        if matches!(rule, OriginTraceRule::Mixed | OriginTraceRule::WriteThrough)
            && let Some(destination) = destination
            && matches!(&event.kind, EventKind::CallEffect(_))
            && let Some(next_event_id) = block.events.get(event_index + 1)
            && let Some(Event {
                kind: EventKind::Access { use_id },
                ..
            }) = problem.events().get(next_event_id.index())
            && problem.uses().get(use_id.index()).is_some_and(|use_row| {
                use_row.kind == UseKind::Write && use_row.place == destination
            })
        {
            // Call results publish their write-through origin at the CallEffect event, while
            // the normalized destination write is the immediately following Access event.
            local_write_through_uses.insert(*use_id);
        }
        pending_write = match &event.kind {
            EventKind::Access { use_id } => problem
                .uses()
                .get(use_id.index())
                .filter(|use_row| use_row.kind == UseKind::Write)
                .map(|use_row| (use_row.place, *use_id)),
            _ => None,
        };
        for (place, origins) in &state.origins {
            local_states.insert(
                (event.id, *place),
                origins
                    .iter()
                    .copied()
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
            );
        }
        if let Some(destination) = destination {
            let origins = sorted_origins(state.origins.get(&destination));
            local_facts.push(OriginFact {
                event: event.id,
                place: destination,
                origins: origins.clone(),
            });
            local_traces.push(OriginTrace {
                event: event.id,
                rule,
                destination: Some(destination),
                input_origins: inputs.into_boxed_slice(),
                output_origins: origins.clone(),
            });
            local_states.insert((event.id, destination), origins);
        } else {
            local_traces.push(OriginTrace {
                event: event.id,
                rule,
                destination: None,
                input_origins: inputs.into_boxed_slice(),
                output_origins: Box::new([]),
            });
        }
    }

    if let Some((facts, traces)) = capture.as_mut() {
        facts.extend(local_facts.iter().cloned());
        traces.extend(local_traces.iter().cloned());
    }
    Ok((
        state,
        local_facts,
        local_traces,
        local_states,
        local_write_through_uses,
    ))
}

fn apply_event(
    problem: &BorrowProblem,
    event: &Event,
    state: &mut FlowState,
) -> Result<(OriginTraceRule, Option<PlaceId>, Vec<ValueOriginId>), CompilerError> {
    match &event.kind {
        EventKind::Fresh {
            destination,
            origin,
        } => {
            if is_alias_only(problem, state, *destination) {
                return Ok(write_through_result(*destination, Vec::new()));
            }
            if is_mixed(problem, state, *destination) {
                return Ok(mixed_write_result(
                    problem,
                    state,
                    *destination,
                    one_origin(*origin),
                    Vec::new(),
                ));
            }
            let was_initialized = state.origins.contains_key(destination);
            let output = one_origin(*origin);
            let mode = if is_mutable_parameter(problem, *destination, *origin) {
                BindingMode::Alias
            } else {
                BindingMode::Slot
            };
            replace_generation(problem, state, *destination, mode, output);
            set_binding_mode(problem, state, *destination, mode);
            let rule = if was_initialized {
                OriginTraceRule::Rebind
            } else {
                OriginTraceRule::Fresh
            };
            Ok((rule, Some(*destination), Vec::new()))
        }
        EventKind::Alias {
            source,
            destination,
            origins,
        }
        | EventKind::ExclusiveAlias {
            source,
            destination,
            origins,
        } => {
            let input = if origins.is_empty() {
                origins_for_place(problem, &state.origins, *source)
            } else {
                origins.iter().copied().collect()
            };
            if is_alias_only(problem, state, *destination) {
                return Ok(write_through_result(
                    *destination,
                    input.into_iter().collect(),
                ));
            }
            if is_mixed(problem, state, *destination) {
                return Ok(mixed_write_result(
                    problem,
                    state,
                    *destination,
                    input.clone(),
                    input.iter().copied().collect(),
                ));
            }
            let rule = if state.origins.contains_key(destination) {
                OriginTraceRule::Rebind
            } else {
                OriginTraceRule::Alias
            };
            let mode = if binding_mode(problem, state, *destination) == Some(BindingMode::Slot) {
                BindingMode::Slot
            } else {
                BindingMode::Alias
            };
            replace_generation(problem, state, *destination, mode, input.clone());
            if mode == BindingMode::Alias {
                set_alias_mode_if_unclassified(problem, state, *destination);
            } else {
                set_binding_mode(problem, state, *destination, mode);
            }
            Ok((rule, Some(*destination), input.into_iter().collect()))
        }
        EventKind::AliasFromPlace {
            source,
            destination,
        }
        | EventKind::ExclusiveAliasFromPlace {
            source,
            destination,
        } => {
            let input = origins_for_place(problem, &state.origins, *source);
            if is_alias_only(problem, state, *destination) {
                return Ok(write_through_result(
                    *destination,
                    input.iter().copied().collect(),
                ));
            }
            if is_mixed(problem, state, *destination) {
                return Ok(mixed_write_result(
                    problem,
                    state,
                    *destination,
                    input.clone(),
                    input.iter().copied().collect(),
                ));
            }
            let rule = if state.origins.contains_key(destination) {
                OriginTraceRule::Rebind
            } else {
                OriginTraceRule::Alias
            };
            let mode = if binding_mode(problem, state, *destination) == Some(BindingMode::Slot) {
                BindingMode::Slot
            } else {
                BindingMode::Alias
            };
            replace_generation(problem, state, *destination, mode, input.clone());
            if mode == BindingMode::Alias {
                set_alias_mode_if_unclassified(problem, state, *destination);
            } else {
                set_binding_mode(problem, state, *destination, mode);
            }
            Ok((rule, Some(*destination), input.into_iter().collect()))
        }
        EventKind::Copy {
            source,
            destination,
            origin,
        } => {
            let source_origins = origins_for_place(problem, &state.origins, *source)
                .into_iter()
                .collect::<Vec<_>>();
            if is_alias_only(problem, state, *destination) {
                return Ok(write_through_result(*destination, source_origins));
            }
            if is_mixed(problem, state, *destination) {
                return Ok(mixed_write_result(
                    problem,
                    state,
                    *destination,
                    one_origin(*origin),
                    source_origins,
                ));
            }
            replace_generation(
                problem,
                state,
                *destination,
                BindingMode::Slot,
                one_origin(*origin),
            );
            set_binding_mode(problem, state, *destination, BindingMode::Slot);
            Ok((OriginTraceRule::Copy, Some(*destination), source_origins))
        }
        EventKind::Projection {
            source,
            destination,
            origin,
        } => {
            let input = projection_child_origins(problem, &state.origins, *source, *origin)
                .unwrap_or_else(|| origins_for_place(problem, &state.origins, *source));
            let output = if input.is_empty() {
                one_origin(*origin)
            } else {
                input.clone()
            };
            if is_alias_only(problem, state, *destination) {
                return Ok(write_through_result(
                    *destination,
                    input.into_iter().collect(),
                ));
            }
            if is_mixed(problem, state, *destination) {
                return Ok(mixed_write_result(
                    problem,
                    state,
                    *destination,
                    output,
                    input.into_iter().collect(),
                ));
            }
            replace_generation(
                problem,
                state,
                *destination,
                BindingMode::Slot,
                output.clone(),
            );
            set_binding_mode(problem, state, *destination, BindingMode::Slot);
            Ok((
                OriginTraceRule::Projection,
                Some(*destination),
                input.into_iter().collect(),
            ))
        }
        EventKind::Rebind { destination, value } => {
            let input = match value {
                super::super::problem::RebindValue::Fresh(origin) => one_origin(*origin),
                super::super::problem::RebindValue::Alias(origins) => {
                    origins.iter().copied().collect()
                }
                super::super::problem::RebindValue::AliasFromPlace(source) => {
                    origins_for_place(problem, &state.origins, *source)
                }
            };
            let input_origins = input.iter().copied().collect();
            if is_alias_only(problem, state, *destination) {
                return Ok(write_through_result(*destination, input_origins));
            }
            if is_mixed(problem, state, *destination) {
                return Ok(mixed_write_result(
                    problem,
                    state,
                    *destination,
                    input.clone(),
                    input_origins,
                ));
            }
            replace_generation(
                problem,
                state,
                *destination,
                BindingMode::Slot,
                input.clone(),
            );
            set_binding_mode(problem, state, *destination, BindingMode::Slot);
            Ok((OriginTraceRule::Rebind, Some(*destination), input_origins))
        }
        EventKind::Aggregate {
            destination,
            origin,
            fields,
        } => {
            let mut input = BTreeSet::new();
            let mut field_states = Vec::new();
            for field in fields {
                let field_origins = origins_for_place(problem, &state.origins, field.source);
                input.extend(field_origins.iter().copied());
                field_states.push((field.projection, field_origins));
            }
            if is_alias_only(problem, state, *destination) {
                return Ok(write_through_result(
                    *destination,
                    input.iter().copied().collect(),
                ));
            }
            if is_mixed(problem, state, *destination) {
                let result = mixed_write_result(
                    problem,
                    state,
                    *destination,
                    one_origin(*origin),
                    input.iter().copied().collect(),
                );
                for (projection, field_origins) in &field_states {
                    if let Some(projected_place) =
                        projected_place(problem, *destination, *projection)
                    {
                        state
                            .alternatives
                            .entry((projected_place, BindingMode::Slot))
                            .or_default()
                            .extend(field_origins.iter().copied());
                    }
                }
                rebuild_origins(state);
                return Ok(result);
            }
            replace_generation(
                problem,
                state,
                *destination,
                BindingMode::Slot,
                one_origin(*origin),
            );
            set_binding_mode(problem, state, *destination, BindingMode::Slot);
            for (projection, field_origins) in field_states {
                if let Some(projected_place) = projected_place(problem, *destination, projection) {
                    state
                        .alternatives
                        .entry((projected_place, BindingMode::Slot))
                        .or_default()
                        .extend(field_origins);
                }
            }
            rebuild_origins(state);
            Ok((
                OriginTraceRule::Aggregate,
                Some(*destination),
                input.into_iter().collect(),
            ))
        }
        EventKind::CallEffect(effect) => {
            let Some(result) = effect.result else {
                return Ok((OriginTraceRule::CallResult, None, Vec::new()));
            };
            let call = problem.calls().get(effect.call.index()).ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "Boracle origin solver cannot locate call {:?}",
                    effect.call
                ))
            })?;
            let _ = call;
            let origin = problem
                .origins()
                .get(result.origin.index())
                .ok_or_else(|| {
                    CompilerError::compiler_error(format!(
                        "Boracle origin solver cannot locate call-result origin {:?}",
                        result.origin
                    ))
                })?;
            let (input, output) = match &origin.kind {
                OriginKind::CallResult { provenance, .. } => {
                    call_result_origins(problem, provenance, effect, &state.origins)
                }
                _ => (BTreeSet::new(), one_origin(result.origin)),
            };
            if is_alias_only(problem, state, result.place) {
                return Ok(write_through_result(
                    result.place,
                    input.iter().copied().collect(),
                ));
            }
            if is_mixed(problem, state, result.place) {
                return Ok(mixed_write_result(
                    problem,
                    state,
                    result.place,
                    output,
                    input.iter().copied().collect(),
                ));
            }
            replace_generation(problem, state, result.place, BindingMode::Slot, output);
            set_binding_mode(problem, state, result.place, BindingMode::Slot);
            Ok((
                OriginTraceRule::CallResult,
                Some(result.place),
                input.into_iter().collect(),
            ))
        }
        EventKind::ScopeExit { bindings } => {
            let roots = bindings.iter().copied().collect::<BTreeSet<_>>();
            state.alternatives.retain(|(place, _), _| {
                problem
                    .places()
                    .get(place.index())
                    .is_none_or(|row| !roots.contains(&row.root))
            });
            for binding in bindings {
                state.modes.remove(binding);
            }
            rebuild_origins(state);
            Ok((OriginTraceRule::ScopeExit, None, Vec::new()))
        }
        EventKind::CallArgument { .. }
        | EventKind::Access { .. }
        | EventKind::ReactiveObserve { .. }
        | EventKind::Terminator { .. }
        | EventKind::LoanIssue { .. }
        | EventKind::LoanKill { .. } => Ok((OriginTraceRule::Noop, None, Vec::new())),
    }
}
fn join_mode(left: BindingMode, right: BindingMode) -> BindingMode {
    if left == right {
        left
    } else {
        BindingMode::Mixed
    }
}

fn destination_binding(problem: &BorrowProblem, destination: PlaceId) -> Option<BindingId> {
    problem
        .places()
        .get(destination.index())
        .map(|place| place.root)
}

fn is_alias_only(problem: &BorrowProblem, state: &FlowState, destination: PlaceId) -> bool {
    binding_mode(problem, state, destination) == Some(BindingMode::Alias)
}

fn is_mixed(problem: &BorrowProblem, state: &FlowState, destination: PlaceId) -> bool {
    binding_mode(problem, state, destination) == Some(BindingMode::Mixed)
}

fn binding_mode(
    problem: &BorrowProblem,
    state: &FlowState,
    destination: PlaceId,
) -> Option<BindingMode> {
    destination_binding(problem, destination).and_then(|binding| state.modes.get(&binding).copied())
}

fn is_mutable_parameter(
    problem: &BorrowProblem,
    destination: PlaceId,
    origin: ValueOriginId,
) -> bool {
    problem
        .origins()
        .get(origin.index())
        .is_some_and(|origin| matches!(origin.kind, OriginKind::Parameter { .. }))
        && destination_binding(problem, destination)
            .and_then(|binding| problem.bindings().get(binding.index()))
            .is_some_and(|binding| binding.mutable)
}

fn set_binding_mode(
    problem: &BorrowProblem,
    state: &mut FlowState,
    destination: PlaceId,
    mode: BindingMode,
) {
    let Some(place) = problem.places().get(destination.index()) else {
        return;
    };
    if place.projections.is_empty() {
        state.modes.insert(place.root, mode);
    }
}

fn set_alias_mode_if_unclassified(
    problem: &BorrowProblem,
    state: &mut FlowState,
    destination: PlaceId,
) {
    let Some(binding) = destination_binding(problem, destination) else {
        return;
    };
    state.modes.entry(binding).or_insert(BindingMode::Alias);
}

fn write_through_result(
    destination: PlaceId,
    inputs: Vec<ValueOriginId>,
) -> (OriginTraceRule, Option<PlaceId>, Vec<ValueOriginId>) {
    (OriginTraceRule::WriteThrough, Some(destination), inputs)
}

fn mixed_write_result(
    problem: &BorrowProblem,
    state: &mut FlowState,
    destination: PlaceId,
    output: OriginSet,
    inputs: Vec<ValueOriginId>,
) -> (OriginTraceRule, Option<PlaceId>, Vec<ValueOriginId>) {
    // A mixed binding represents two correlated possibilities. A write can replace the
    // slot-backed possibility while the alias-backed possibility writes through to its
    // referent. Keep the alternatives separate so the replaced slot generation does not
    // retain stale origins from the other possibility.
    replace_mode_generation(problem, state, destination, BindingMode::Slot, output);
    set_binding_mode(problem, state, destination, BindingMode::Mixed);
    (OriginTraceRule::Mixed, Some(destination), inputs)
}

fn rebuild_origins(state: &mut FlowState) {
    let mut origins = BTreeMap::new();
    for ((place, _mode), origin_set) in &state.alternatives {
        origins
            .entry(*place)
            .or_insert_with(BTreeSet::new)
            .extend(origin_set.iter().copied());
    }
    state.origins = origins;
}

fn place_is_in_generation(
    problem: &BorrowProblem,
    generation: PlaceId,
    candidate: PlaceId,
) -> bool {
    let Some(generation_row) = problem.places().get(generation.index()) else {
        return false;
    };
    let Some(candidate_row) = problem.places().get(candidate.index()) else {
        return false;
    };
    candidate_row.root == generation_row.root
        && candidate_row.projections.len() >= generation_row.projections.len()
        && candidate_row
            .projections
            .starts_with(&generation_row.projections)
}

fn replace_generation(
    problem: &BorrowProblem,
    state: &mut FlowState,
    destination: PlaceId,
    mode: BindingMode,
    output: OriginSet,
) {
    state
        .alternatives
        .retain(|(place, _), _| !place_is_in_generation(problem, destination, *place));
    state.alternatives.insert((destination, mode), output);
    rebuild_origins(state);
}

fn replace_mode_generation(
    problem: &BorrowProblem,
    state: &mut FlowState,
    destination: PlaceId,
    mode: BindingMode,
    output: OriginSet,
) {
    state.alternatives.retain(|(place, existing_mode), _| {
        *existing_mode != mode || !place_is_in_generation(problem, destination, *place)
    });
    state.alternatives.insert((destination, mode), output);
    rebuild_origins(state);
}

fn call_result_origins(
    problem: &BorrowProblem,
    provenance: &CallResultProvenance,
    effect: &super::super::problem::CallEffect,
    state: &OriginState,
) -> (OriginSet, OriginSet) {
    match provenance {
        CallResultProvenance::Fresh => (BTreeSet::new(), one_origin_from_result(effect)),
        CallResultProvenance::Unknown => {
            let output = one_origin_from_result(effect);
            (output.clone(), output)
        }
        CallResultProvenance::Alias(origins) => {
            let origins = origins.iter().copied().collect::<BTreeSet<_>>();
            (origins.clone(), origins)
        }
        CallResultProvenance::AliasParams(indices) => {
            let mut origins = BTreeSet::new();
            for index in indices {
                if let Some(argument) = effect.arguments.get(*index) {
                    origins.extend(origins_for_place(problem, state, argument.place));
                }
            }
            if origins.is_empty() {
                (origins, one_origin_from_result(effect))
            } else {
                (origins.clone(), origins)
            }
        }
    }
}

fn origins_for_place(problem: &BorrowProblem, state: &OriginState, place: PlaceId) -> OriginSet {
    if let Some(origins) = state.get(&place) {
        return origins.clone();
    }

    let Some(place_row) = problem.places().get(place.index()) else {
        return BTreeSet::new();
    };
    let mut origins = BTreeSet::new();
    for (candidate, candidate_origins) in state {
        let Some(candidate_row) = problem.places().get(candidate.index()) else {
            continue;
        };
        if candidate_row.root == place_row.root
            && candidate_row.projections.len() <= place_row.projections.len()
            && place_row
                .projections
                .starts_with(&candidate_row.projections)
        {
            origins.extend(candidate_origins.iter().copied());
        }
    }
    origins
}

fn projected_place(
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

fn projection_child_origins(
    problem: &BorrowProblem,
    state: &OriginState,
    source: PlaceId,
    origin: ValueOriginId,
) -> Option<OriginSet> {
    let projection = problem
        .origins()
        .get(origin.index())
        .and_then(|row| match &row.kind {
            OriginKind::Projection { projection, .. } => Some(*projection),
            _ => None,
        })?;
    let child = projected_place(problem, source, projection)?;
    state.get(&child).cloned()
}

fn one_origin_from_result(effect: &super::super::problem::CallEffect) -> OriginSet {
    effect
        .result
        .map(|result| one_origin(result.origin))
        .unwrap_or_default()
}

fn one_origin(origin: ValueOriginId) -> OriginSet {
    BTreeSet::from([origin])
}

fn sorted_origins(origins: Option<&OriginSet>) -> Box<[ValueOriginId]> {
    origins
        .into_iter()
        .flatten()
        .copied()
        .collect::<Vec<_>>()
        .into_boxed_slice()
}
