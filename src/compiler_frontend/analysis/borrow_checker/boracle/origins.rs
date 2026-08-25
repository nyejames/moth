//! Binding-independent origin propagation for the Boracle reference solver.
//!
//! WHAT: evaluates normalized definition, alias, copy, projection, aggregate and call-result
//! events into deterministic origin sets at CFG points.
//! WHY: a binding can be rebound while older aliases remain meaningful; provenance therefore
//! belongs to values and event flow, not to local names.

// The feature lane grows the reference report incrementally. Keep the origin vocabulary warning
// free while the loan/conflict and service consumers are added in later phases.
#![allow(dead_code)]

use super::super::problem::{
    BorrowProblem, CallResultProvenance, Event, EventId, EventKind, OriginKind, PlaceId,
    ValueOriginId,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use std::collections::{BTreeMap, BTreeSet};

type OriginSet = BTreeSet<ValueOriginId>;
type OriginState = BTreeMap<PlaceId, OriginSet>;

/// A stable reason attached to one propagated provenance fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OriginTraceRule {
    Noop,
    Fresh,
    Alias,
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

        let mut entry_states = vec![None::<OriginState>; block_count];
        entry_states[problem.control_flow().entry.index()] = Some(BTreeMap::new());
        let mut output_states = vec![OriginState::new(); block_count];

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
                        Some(BTreeMap::new()),
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
        for (block_index, _) in problem.control_flow().blocks.iter().enumerate() {
            let Some(entry) = entry_states[block_index].clone() else {
                continue;
            };
            let (_, _, _, block_states) =
                apply_block(problem, block_index, entry, Some((&mut facts, &mut traces)))?;
            for ((event, place), origins) in block_states {
                state_after_event.insert((event, place), origins);
            }
        }

        facts.sort_by_key(|fact| (fact.event.raw(), fact.place.raw()));
        traces.sort_by_key(|trace| trace.event.raw());

        Ok(OriginSolution {
            facts: facts.into_boxed_slice(),
            traces: traces.into_boxed_slice(),
            state_after_event,
        })
    }
}

fn join_predecessors(
    entry_states: &[Option<OriginState>],
    output_states: &[OriginState],
    predecessors: &[usize],
    initial: Option<OriginState>,
) -> Option<OriginState> {
    let mut joined = initial;
    for predecessor in predecessors {
        let Some(state) = entry_states[*predecessor]
            .as_ref()
            .map(|_| &output_states[*predecessor])
        else {
            continue;
        };
        merge_state(joined.get_or_insert_with(BTreeMap::new), state);
    }
    joined
}

fn merge_state(destination: &mut OriginState, source: &OriginState) {
    for (place, origins) in source {
        destination
            .entry(*place)
            .or_default()
            .extend(origins.iter().copied());
    }
}

type ApplyCapture<'a> = Option<(&'a mut Vec<OriginFact>, &'a mut Vec<OriginTrace>)>;
type AppliedBlock = (
    OriginState,
    Vec<OriginFact>,
    Vec<OriginTrace>,
    BTreeMap<(EventId, PlaceId), Box<[ValueOriginId]>>,
);

fn apply_block(
    problem: &BorrowProblem,
    block_index: usize,
    mut state: OriginState,
    mut capture: ApplyCapture<'_>,
) -> Result<AppliedBlock, CompilerError> {
    let block = &problem.control_flow().blocks[block_index];
    let mut local_facts = Vec::new();
    let mut local_traces = Vec::new();
    let mut local_states = BTreeMap::new();
    for event_id in &block.events {
        let event = problem.events().get(event_id.index()).ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "Boracle origin solver cannot locate event {:?}",
                event_id
            ))
        })?;
        let (rule, destination, inputs) = apply_event(problem, event, &mut state)?;
        for (place, origins) in &state {
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
            let origins = sorted_origins(state.get(&destination));
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
    Ok((state, local_facts, local_traces, local_states))
}

fn apply_event(
    problem: &BorrowProblem,
    event: &Event,
    state: &mut OriginState,
) -> Result<(OriginTraceRule, Option<PlaceId>, Vec<ValueOriginId>), CompilerError> {
    match &event.kind {
        EventKind::Fresh {
            destination,
            origin,
        } => {
            let output = one_origin(*origin);
            state.insert(*destination, output);
            Ok((OriginTraceRule::Fresh, Some(*destination), Vec::new()))
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
                state.get(source).cloned().unwrap_or_default()
            } else {
                origins.iter().copied().collect()
            };
            state.insert(*destination, input.clone());
            Ok((
                OriginTraceRule::Alias,
                Some(*destination),
                input.into_iter().collect(),
            ))
        }
        EventKind::AliasFromPlace {
            source,
            destination,
        }
        | EventKind::ExclusiveAliasFromPlace {
            source,
            destination,
        } => {
            let input = state.get(source).cloned().unwrap_or_default();
            state.insert(*destination, input.clone());
            Ok((
                OriginTraceRule::Alias,
                Some(*destination),
                input.into_iter().collect(),
            ))
        }
        EventKind::Copy {
            destination,
            origin,
            ..
        } => {
            state.insert(*destination, one_origin(*origin));
            Ok((OriginTraceRule::Copy, Some(*destination), Vec::new()))
        }
        EventKind::Projection {
            source,
            destination,
            origin,
        } => {
            let input = state.get(source).cloned().unwrap_or_default();
            state.insert(*destination, one_origin(*origin));
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
                    state.get(source).cloned().unwrap_or_default()
                }
            };
            let input_origins = input.iter().copied().collect();
            state.insert(*destination, input);
            Ok((OriginTraceRule::Rebind, Some(*destination), input_origins))
        }
        EventKind::Aggregate {
            destination,
            origin,
            fields,
        } => {
            let mut input = BTreeSet::new();
            for field in fields {
                if let Some(origins) = state.get(&field.source) {
                    input.extend(origins.iter().copied());
                }
            }
            state.insert(*destination, one_origin(*origin));
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
                    call_result_origins(provenance, effect, state)
                }
                _ => (BTreeSet::new(), one_origin(result.origin)),
            };
            state.insert(result.place, output);
            Ok((
                OriginTraceRule::CallResult,
                Some(result.place),
                input.into_iter().collect(),
            ))
        }
        EventKind::ScopeExit { bindings } => {
            let roots = bindings.iter().copied().collect::<BTreeSet<_>>();
            let removed = state
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
                state.remove(&place);
            }
            Ok((OriginTraceRule::ScopeExit, None, Vec::new()))
        }
        EventKind::Access { .. }
        | EventKind::ReactiveObserve { .. }
        | EventKind::Terminator { .. }
        | EventKind::LoanIssue { .. }
        | EventKind::LoanKill { .. } => Ok((OriginTraceRule::Noop, None, Vec::new())),
    }
}

fn call_result_origins(
    provenance: &CallResultProvenance,
    effect: &super::super::problem::CallEffect,
    state: &OriginState,
) -> (OriginSet, OriginSet) {
    match provenance {
        CallResultProvenance::Fresh | CallResultProvenance::Unknown => {
            (BTreeSet::new(), one_origin_from_result(effect))
        }
        CallResultProvenance::Alias(origins) => {
            let origins = origins.iter().copied().collect::<BTreeSet<_>>();
            (origins.clone(), origins)
        }
        CallResultProvenance::AliasParams(indices) => {
            let mut origins = BTreeSet::new();
            for index in indices {
                if let Some(argument) = effect.arguments.get(*index)
                    && let Some(argument_origins) = state.get(&argument.place)
                {
                    origins.extend(argument_origins.iter().copied());
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
