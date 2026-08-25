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
    UseId, UseKind, ValueOriginId,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use std::collections::{BTreeMap, BTreeSet};

type OriginSet = BTreeSet<ValueOriginId>;
type OriginState = BTreeMap<PlaceId, OriginSet>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BindingMode {
    Alias,
    Slot,
    Mixed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FlowState {
    origins: OriginState,
    modes: BTreeMap<BindingId, BindingMode>,
}

/// A stable reason attached to one propagated provenance fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OriginTraceRule {
    Noop,
    Fresh,
    Alias,
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

    pub(crate) fn origins_for_place_after_event(
        &self,
        problem: &BorrowProblem,
        event: EventId,
        place: PlaceId,
    ) -> Box<[ValueOriginId]> {
        if let Some(origins) = self.origins_after_event(event, place) {
            return origins.to_vec().into_boxed_slice();
        }

        let Some(place_row) = problem.places().get(place.index()) else {
            return Box::new([]);
        };
        let mut origins = BTreeSet::new();
        for ((candidate_event, candidate_place), candidate_origins) in &self.state_after_event {
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
            modes: BTreeMap::new(),
        });
        let mut output_states = vec![
            FlowState {
                origins: BTreeMap::new(),
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

        Ok(OriginSolution {
            facts: facts.into_boxed_slice(),
            traces: traces.into_boxed_slice(),
            state_after_event,
            write_through_uses: write_through_uses.into_iter().collect(),
        })
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
                modes: BTreeMap::new(),
            }),
            state,
        );
    }
    joined
}

fn merge_state(destination: &mut FlowState, source: &FlowState) {
    for (place, origins) in &source.origins {
        destination
            .origins
            .entry(*place)
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
    for event_id in &block.events {
        let event = problem.events().get(event_id.index()).ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "Boracle origin solver cannot locate event {:?}",
                event_id
            ))
        })?;
        let (rule, destination, inputs) = apply_event(problem, event, &mut state)?;
        if rule == OriginTraceRule::WriteThrough
            && let Some(destination) = destination
            && let Some((place, use_id)) = pending_write
            && place == destination
        {
            local_write_through_uses.insert(use_id);
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
                return Ok(write_through_result(
                    problem,
                    state,
                    *destination,
                    Vec::new(),
                ));
            }
            let was_initialized = state.origins.contains_key(destination);
            let output = one_origin(*origin);
            replace_generation(problem, &mut state.origins, *destination);
            state.origins.insert(*destination, output);
            set_binding_mode(problem, state, *destination, BindingMode::Slot);
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
                    problem,
                    state,
                    *destination,
                    input.into_iter().collect(),
                ));
            }
            let rule = if state.origins.contains_key(destination) {
                OriginTraceRule::Rebind
            } else {
                OriginTraceRule::Alias
            };
            replace_generation(problem, &mut state.origins, *destination);
            state.origins.insert(*destination, input.clone());
            set_binding_mode(problem, state, *destination, BindingMode::Alias);
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
                    problem,
                    state,
                    *destination,
                    input.iter().copied().collect(),
                ));
            }
            let rule = if state.origins.contains_key(destination) {
                OriginTraceRule::Rebind
            } else {
                OriginTraceRule::Alias
            };
            replace_generation(problem, &mut state.origins, *destination);
            state.origins.insert(*destination, input.clone());
            set_binding_mode(problem, state, *destination, BindingMode::Alias);
            Ok((rule, Some(*destination), input.into_iter().collect()))
        }
        EventKind::Copy {
            destination,
            origin,
            ..
        } => {
            if is_alias_only(problem, state, *destination) {
                return Ok(write_through_result(
                    problem,
                    state,
                    *destination,
                    Vec::new(),
                ));
            }
            replace_generation(problem, &mut state.origins, *destination);
            state.origins.insert(*destination, one_origin(*origin));
            set_binding_mode(problem, state, *destination, BindingMode::Slot);
            Ok((OriginTraceRule::Copy, Some(*destination), Vec::new()))
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
                    problem,
                    state,
                    *destination,
                    input.into_iter().collect(),
                ));
            }
            replace_generation(problem, &mut state.origins, *destination);
            state.origins.insert(*destination, output);
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
                return Ok(write_through_result(
                    problem,
                    state,
                    *destination,
                    input_origins,
                ));
            }
            replace_generation(problem, &mut state.origins, *destination);
            state.origins.insert(*destination, input);
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
                    problem,
                    state,
                    *destination,
                    input.iter().copied().collect(),
                ));
            }
            replace_generation(problem, &mut state.origins, *destination);
            state.origins.insert(*destination, one_origin(*origin));
            set_binding_mode(problem, state, *destination, BindingMode::Slot);
            for (projection, field_origins) in field_states {
                if let Some(projected_place) = projected_place(problem, *destination, projection) {
                    state
                        .origins
                        .entry(projected_place)
                        .or_default()
                        .extend(field_origins);
                }
            }
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
                    problem,
                    state,
                    result.place,
                    input.iter().copied().collect(),
                ));
            }
            replace_generation(problem, &mut state.origins, result.place);
            state.origins.insert(result.place, output);
            set_binding_mode(problem, state, result.place, BindingMode::Slot);
            Ok((
                OriginTraceRule::CallResult,
                Some(result.place),
                input.into_iter().collect(),
            ))
        }
        EventKind::ScopeExit { bindings } => {
            let roots = bindings.iter().copied().collect::<BTreeSet<_>>();
            let removed = state
                .origins
                .keys()
                .copied()
                .filter(|place| {
                    problem
                        .places()
                        .get(place.index())
                        .is_some_and(|row| roots.contains(&row.root))
                })
                .collect::<Vec<_>>();
            for place in removed {
                state.origins.remove(&place);
            }
            for binding in bindings {
                state.modes.remove(binding);
            }
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
    destination_binding(problem, destination).and_then(|binding| state.modes.get(&binding).copied())
        == Some(BindingMode::Alias)
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

fn write_through_result(
    problem: &BorrowProblem,
    state: &FlowState,
    destination: PlaceId,
    inputs: Vec<ValueOriginId>,
) -> (OriginTraceRule, Option<PlaceId>, Vec<ValueOriginId>) {
    let _ = problem;
    let _ = state;
    (OriginTraceRule::WriteThrough, Some(destination), inputs)
}

fn replace_generation(problem: &BorrowProblem, state: &mut OriginState, destination: PlaceId) {
    let Some(destination_row) = problem.places().get(destination.index()) else {
        return;
    };
    let stale_places = state
        .keys()
        .copied()
        .filter(|place| {
            let Some(place_row) = problem.places().get(place.index()) else {
                return false;
            };
            place_row.root == destination_row.root
                && place_row.projections.len() >= destination_row.projections.len()
                && place_row
                    .projections
                    .starts_with(&destination_row.projections)
        })
        .collect::<Vec<_>>();
    for place in stale_places {
        state.remove(&place);
    }
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
