//! Single-path forward execution for the operational oracle.
//!
//! WHAT: dispatches every normalised event explicitly and advances one concrete path through one
//!       CFG block until its terminator decides what happens next.
//! WHY: the path layer owns the frontier and bounds spanning executions; the executor and state
//!      own the bounds within one execution.

use super::calls;
use super::conflicts;
use super::state::{
    CapabilitySource, DefinitionEventKind, DefinitionRole, DefinitionTransition, OracleState,
    PlaceIndex, RuntimeAccessTarget, RuntimePlaceState,
};
use super::traces::{RuntimeConflictWitness, TraceAccess, TraceBuilder};
use super::{OracleBounds, OracleLimitReason};
use crate::compiler_frontend::analysis::borrow_checker::problem::{
    AccessKind, AggregateField, BlockId, BorrowProblem, Call, CallId, CfgBlock, Event, EventId,
    EventKind, Loan, LoanId, OriginKind, Place, PlaceId, PointId, ProgramPoint, RebindValue,
    TerminatorEventKind, Use, UseId, ValueOrigin, ValueOriginId,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug)]
pub(super) enum EventExecutionResult<'a> {
    Continue,
    NextBlock(BlockId),
    NextBlocks(&'a [BlockId]),
    Complete,
    RuntimeConflict(RuntimeConflictWitness),
    Inconclusive(OracleLimitReason),
}

#[derive(Debug)]
pub(super) enum AccessExecutionResult {
    Continue,
    RuntimeConflict(RuntimeConflictWitness),
    Inconclusive(OracleLimitReason),
}

#[derive(Debug, Clone, Copy)]
pub(super) struct EventCursor {
    pub(super) block: BlockId,
    pub(super) event_index: usize,
}

/// Per-event execution context. `state` and `trace` belong to one path, while `explored_events`
/// counts every event this enumeration dispatched across all paths.
pub(super) struct OracleExecutionContext<'problem, 'path> {
    pub(super) problem: &'problem BorrowProblem,
    pub(super) place_index: &'problem PlaceIndex,
    pub(super) state: &'path mut OracleState,
    pub(super) trace: &'path mut TraceBuilder,
    pub(super) explored_events: &'path mut usize,
    pub(super) trace_index: usize,
    pub(super) bounds: OracleBounds,
}

pub(super) fn execute_block<'problem, 'path>(
    context: &mut OracleExecutionContext<'problem, 'path>,
    mut cursor: EventCursor,
) -> Result<EventExecutionResult<'problem>, CompilerError> {
    // The cursor never leaves this block, so resolve it once. `problem` is copied out of the
    // context so the event slice borrows the problem rather than the mutably borrowed context.
    let problem = context.problem;
    let event_ids = &*require_block(problem, cursor.block)?.events;
    if event_ids.is_empty() {
        return Err(oracle_error(format!(
            "CFG block {:?} has no executable events",
            cursor.block
        )));
    }
    if cursor.event_index >= event_ids.len() {
        return Err(oracle_error(format!(
            "event cursor is outside CFG block {:?}",
            cursor.block
        )));
    }

    loop {
        if context.state.executed_events >= context.bounds.max_executed_events {
            return Ok(EventExecutionResult::Inconclusive(
                OracleLimitReason::EventBound {
                    limit: context.bounds.max_executed_events,
                },
            ));
        }

        let event = require_event(problem, event_ids[cursor.event_index])?;
        let trace_index = context.trace.begin(event.id, event.point, cursor.block);
        context.state.executed_events += 1;
        *context.explored_events += 1;
        context.trace_index = trace_index;

        let result = dispatch_event(context, event)?;
        match result {
            EventExecutionResult::Continue => {
                cursor.event_index += 1;
                if cursor.event_index == event_ids.len() {
                    return Err(oracle_error(format!(
                        "CFG block {:?} ended without a terminator",
                        cursor.block
                    )));
                }
            }
            EventExecutionResult::NextBlock(target) => {
                if cursor.event_index + 1 != event_ids.len() {
                    return Err(oracle_error(format!(
                        "terminator event {:?} is not last in block {:?}",
                        event.id, cursor.block
                    )));
                }
                return Ok(EventExecutionResult::NextBlock(target));
            }
            EventExecutionResult::NextBlocks(targets) => {
                if cursor.event_index + 1 != event_ids.len() {
                    return Err(oracle_error(format!(
                        "terminator event {:?} is not last in block {:?}",
                        event.id, cursor.block
                    )));
                }
                return Ok(EventExecutionResult::NextBlocks(targets));
            }
            EventExecutionResult::Complete => {
                if cursor.event_index + 1 != event_ids.len() {
                    return Err(oracle_error(format!(
                        "terminal terminator event {:?} is not last in block {:?}",
                        event.id, cursor.block
                    )));
                }
                return Ok(EventExecutionResult::Complete);
            }
            EventExecutionResult::RuntimeConflict(witness) => {
                return Ok(EventExecutionResult::RuntimeConflict(witness));
            }
            EventExecutionResult::Inconclusive(reason) => {
                return Ok(EventExecutionResult::Inconclusive(reason));
            }
        }
    }
}

fn dispatch_event<'problem, 'path>(
    context: &mut OracleExecutionContext<'problem, 'path>,
    event: &'problem Event,
) -> Result<EventExecutionResult<'problem>, CompilerError> {
    match &event.kind {
        EventKind::Fresh {
            destination,
            origin,
        } => execute_fresh(context, *destination, *origin),

        EventKind::Alias {
            source,
            destination,
            origins: _,
        } => execute_alias(context, event, *source, *destination, AccessKind::Shared),

        EventKind::AliasFromPlace {
            source,
            destination,
        } => execute_alias(context, event, *source, *destination, AccessKind::Shared),

        EventKind::ExclusiveAlias {
            source,
            destination,
            origins: _,
        } => execute_alias(context, event, *source, *destination, AccessKind::Exclusive),

        EventKind::ExclusiveAliasFromPlace {
            source,
            destination,
        } => execute_alias(context, event, *source, *destination, AccessKind::Exclusive),

        EventKind::Copy {
            source,
            destination,
            origin: _,
        } => execute_copy(context, event, *source, *destination),

        EventKind::Projection {
            source,
            destination,
            origin,
        } => execute_projection(context, event, *source, *destination, *origin),

        EventKind::Rebind { destination, value } => {
            execute_rebind(context, event, *destination, value)
        }

        EventKind::Aggregate {
            destination,
            origin: _,
            fields,
        } => execute_aggregate(context, event, *destination, fields),

        EventKind::ScopeExit { bindings } => {
            let ended =
                context
                    .state
                    .end_holders(context.problem, bindings, context.trace_index)?;
            for capability_id in ended.iter().copied() {
                context.trace.record_end(context.trace_index, capability_id);
            }
            Ok(EventExecutionResult::Continue)
        }

        EventKind::ReactiveObserve { place } => {
            require_place(context.problem, *place, "reactive observation")?;
            // The contract makes this event metadata-only: no capability, no access check and
            // no conflict. The observation never resolves a runtime target, so the availability
            // check must not descend the observed path either. A materialising resolution would
            // create generations for a missing Field or FixedIndex, which consumes the
            // generation bound and reidentifies every later dynamic position, so a complete safe
            // execution could turn into GenerationBound at a tight bound.
            if is_place_available_without_materialising(context.problem, context.state, *place)? {
                Ok(EventExecutionResult::Continue)
            } else {
                Err(oracle_error(format!(
                    "reactive observation reads unavailable place {:?}",
                    place
                )))
            }
        }

        EventKind::CallArgument {
            call,
            index,
            argument,
        } => calls::execute_call_argument(context, event, *call, *index, argument),

        EventKind::Terminator { kind } => execute_terminator(context, kind),

        EventKind::CallEffect(effect) => calls::execute_call_effect(context, event, effect),

        EventKind::Access { use_id } => {
            let use_row = require_use(context.problem, *use_id, "access")?;
            if use_row.point != event.point {
                return Err(oracle_error(format!(
                    "access use {:?} point {:?} does not match event point {:?}",
                    use_id, use_row.point, event.point
                )));
            }
            match execute_access(context, event, *use_id, None)? {
                AccessExecutionResult::Continue => Ok(EventExecutionResult::Continue),
                AccessExecutionResult::RuntimeConflict(witness) => {
                    Ok(EventExecutionResult::RuntimeConflict(witness))
                }
                AccessExecutionResult::Inconclusive(reason) => {
                    Ok(EventExecutionResult::Inconclusive(reason))
                }
            }
        }

        EventKind::LoanIssue { loan } => execute_loan_issue(context, event, *loan),

        EventKind::LoanKill { loan, reason: _ } => execute_loan_kill(context, *loan),
    }
}

fn execute_fresh<'problem>(
    context: &mut OracleExecutionContext<'problem, '_>,
    destination: PlaceId,
    origin: ValueOriginId,
) -> Result<EventExecutionResult<'problem>, CompilerError> {
    let problem = context.problem;
    require_place(problem, destination, "fresh destination")?;
    require_origin(problem, origin, "fresh origin")?;
    if let RuntimePlaceState::Alias { target, .. } = context.state.state(destination)? {
        let transition = context.state.apply_definition_transition(
            problem,
            destination,
            DefinitionEventKind::Value,
            DefinitionRole::Slot { current: target },
            context.trace_index,
        )?;
        finish_definition_transition(context, &transition);
        return Ok(EventExecutionResult::Continue);
    }
    let generation = match context
        .state
        .issue_generation(context.bounds.max_dynamic_generations)
    {
        Ok(generation) => generation,
        Err(reason) => return Ok(EventExecutionResult::Inconclusive(reason)),
    };
    let (event_kind, role) = if is_mutable_parameter(problem, destination, origin) {
        (
            DefinitionEventKind::MutableParameter,
            DefinitionRole::Alias {
                target: RuntimeAccessTarget {
                    node: generation,
                    path: Box::new([]),
                },
                access: AccessKind::Exclusive,
            },
        )
    } else {
        (
            DefinitionEventKind::Value,
            DefinitionRole::Slot {
                current: generation,
            },
        )
    };
    let transition = context.state.apply_definition_transition(
        problem,
        destination,
        event_kind,
        role,
        context.trace_index,
    )?;
    finish_definition_transition(context, &transition);
    Ok(EventExecutionResult::Continue)
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
        && problem
            .places()
            .get(destination.index())
            .and_then(|place| problem.bindings().get(place.root.index()))
            .is_some_and(|binding| binding.mutable)
}

pub(super) fn record_retired_capabilities(
    context: &mut OracleExecutionContext<'_, '_>,
    transition: &DefinitionTransition,
) {
    let retired_capabilities = match transition {
        DefinitionTransition::Installed {
            retired_capabilities,
            ..
        }
        | DefinitionTransition::ReplacedSlot {
            retired_capabilities,
            ..
        } => retired_capabilities,
        DefinitionTransition::WriteThroughAlias { .. } => return,
    };
    for capability_id in retired_capabilities.iter().copied() {
        context.trace.record_end(context.trace_index, capability_id);
    }
}

pub(super) fn finish_definition_transition(
    context: &mut OracleExecutionContext<'_, '_>,
    transition: &DefinitionTransition,
) {
    record_retired_capabilities(context, transition);
}

/// Checks availability without descending projection paths or materialising dynamic children.
///
/// Write-through definitions still validate their ignored inputs, but resolving those inputs
/// would create generations and could make an otherwise allocation-free event inconclusive.
pub(super) fn is_place_available_without_materialising(
    problem: &BorrowProblem,
    state: &OracleState,
    place_id: PlaceId,
) -> Result<bool, CompilerError> {
    let place = require_place(problem, place_id, "availability check")?;
    Ok(problem.places().iter().any(|candidate| {
        candidate.root == place.root
            && candidate.projections.len() <= place.projections.len()
            && place.projections.starts_with(&candidate.projections)
            && state
                .places
                .get(&candidate.id)
                .is_some_and(|candidate_state| {
                    !matches!(candidate_state, RuntimePlaceState::Unavailable)
                })
    }))
}

fn execute_alias<'problem>(
    context: &mut OracleExecutionContext<'problem, '_>,
    event: &Event,
    source: PlaceId,
    destination: PlaceId,
    access: AccessKind,
) -> Result<EventExecutionResult<'problem>, CompilerError> {
    let problem = context.problem;
    let place_index = context.place_index;
    let trace_index = context.trace_index;
    let bounds = context.bounds;
    require_place(problem, source, "alias source")?;
    require_place(problem, destination, "alias destination")?;

    // The transition owner decides whether this source is consumed. A write-through keeps the
    // existing referent, so only validate source availability without materialising its target.
    if let RuntimePlaceState::Alias { target, .. } = context.state.state(destination)? {
        if !is_place_available_without_materialising(problem, context.state, source)? {
            return Err(oracle_error(format!(
                "alias source {:?} is unavailable at event {:?}",
                source, event.id
            )));
        }
        let transition = context.state.apply_definition_transition(
            problem,
            destination,
            DefinitionEventKind::DirectAlias,
            DefinitionRole::Slot { current: target },
            trace_index,
        )?;
        finish_definition_transition(context, &transition);
        return Ok(EventExecutionResult::Continue);
    }

    let resolved = match context.state.resolve_place(
        problem,
        place_index,
        source,
        bounds.max_dynamic_generations,
    )? {
        Ok(Some(resolved)) => resolved,
        Ok(None) => {
            return Err(oracle_error(format!(
                "alias source {:?} is unavailable at event {:?}",
                source, event.id
            )));
        }
        Err(reason) => return Ok(EventExecutionResult::Inconclusive(reason)),
    };
    // An unavailable destination installs the alias with its full residual path, so the alias
    // state stays faithful. A slot-backed destination cannot: its replacement collapses through
    // `DefinitionRole::slot_target` to a bare generation, which would make the destination
    // compare equal to the whole base node and manufacture the definite overlap the contract
    // classifies as UNDECIDABLE. Refuse before the transition, in the shape the Copy and
    // Aggregate arms use.
    if !resolved.target.path.is_empty()
        && matches!(
            context.state.state(destination)?,
            RuntimePlaceState::Slot { .. }
        )
    {
        return Ok(EventExecutionResult::Inconclusive(
            OracleLimitReason::UndecidableOverlap {
                left: resolved.target.clone(),
                right: RuntimeAccessTarget {
                    node: resolved.target.node,
                    path: Box::new([]),
                },
            },
        ));
    }
    let transition = context.state.apply_definition_transition(
        problem,
        destination,
        DefinitionEventKind::DirectAlias,
        DefinitionRole::Alias {
            target: resolved.target.clone(),
            access,
        },
        trace_index,
    )?;
    finish_definition_transition(context, &transition);
    // A slot replacement is a value rebind even when the syntax is exclusive. The reference
    // carries that relationship as a shared provenance capability rather than a direct alias.
    match transition {
        DefinitionTransition::WriteThroughAlias { .. } => {}
        DefinitionTransition::Installed { .. } => {
            let capability_id = context.state.issue_capability(
                access,
                resolved.target,
                BTreeSet::from([destination]),
                trace_index,
                event.id,
                CapabilitySource::Alias,
            )?;
            context.trace.record_issue(trace_index, capability_id);
        }
        DefinitionTransition::ReplacedSlot { .. } => {
            let capability_id = context.state.issue_capability(
                AccessKind::Shared,
                resolved.target,
                BTreeSet::from([destination]),
                trace_index,
                event.id,
                CapabilitySource::Provenance,
            )?;
            context.trace.record_issue(trace_index, capability_id);
        }
    }
    Ok(EventExecutionResult::Continue)
}

fn execute_copy<'problem>(
    context: &mut OracleExecutionContext<'problem, '_>,
    _event: &Event,
    source: PlaceId,
    destination: PlaceId,
) -> Result<EventExecutionResult<'problem>, CompilerError> {
    let problem = context.problem;
    let place_index = context.place_index;
    let state = &mut *context.state;
    let bounds = context.bounds;
    require_place(problem, source, "copy source")?;
    require_place(problem, destination, "copy destination")?;
    // The destination action is decided before any graph copy: an alias-backed destination
    // writes through, so allocating copied generations here would both consume the generation
    // bound and insert dynamic nodes for a value the event does not create. The source still
    // needs a non-materialising availability check because the reference validates every input.
    if let RuntimePlaceState::Alias { target, .. } = state.state(destination)? {
        if !is_place_available_without_materialising(problem, state, source)? {
            return Err(oracle_error(format!(
                "copy source {:?} is unavailable",
                source
            )));
        }
        let transition = context.state.apply_definition_transition(
            problem,
            destination,
            DefinitionEventKind::Value,
            DefinitionRole::Slot { current: target },
            context.trace_index,
        )?;
        finish_definition_transition(context, &transition);
        return Ok(EventExecutionResult::Continue);
    }

    let source_target =
        match state.resolve_place(problem, place_index, source, bounds.max_dynamic_generations)? {
            Ok(Some(source_target)) => source_target,
            Ok(None) => {
                return Err(oracle_error(format!(
                    "copy source {:?} is unavailable",
                    source
                )));
            }
            Err(reason) => return Ok(EventExecutionResult::Inconclusive(reason)),
        };
    let source_target =
        match state.resolve_target(source_target.target, bounds.max_dynamic_generations)? {
            Ok(source_target) => source_target,
            Err(reason) => return Ok(EventExecutionResult::Inconclusive(reason)),
        };
    if !source_target.path.is_empty() {
        return Ok(EventExecutionResult::Inconclusive(
            OracleLimitReason::UndecidableOverlap {
                left: source_target.clone(),
                right: RuntimeAccessTarget {
                    node: source_target.node,
                    path: Box::new([]),
                },
            },
        ));
    }

    let mut reachable = BTreeSet::new();
    let mut pending = vec![source_target.node];
    while let Some(node) = pending.pop() {
        if !reachable.insert(node) {
            continue;
        }
        if let Some(aggregate) = state.aggregates.get(&node) {
            for child in aggregate.children.values().copied() {
                pending.push(child);
            }
        }
    }

    let mut correspondence = BTreeMap::new();
    for source_node in reachable.iter().copied() {
        let copied = match state.issue_generation(bounds.max_dynamic_generations) {
            Ok(copied) => copied,
            Err(reason) => return Ok(EventExecutionResult::Inconclusive(reason)),
        };
        correspondence.insert(source_node, copied);
    }

    let mut copied_aggregates = Vec::new();
    for source_node in &reachable {
        let Some(source_aggregate) = state.aggregates.get(source_node) else {
            continue;
        };
        let copied_node = correspondence
            .get(source_node)
            .copied()
            .ok_or_else(|| oracle_error("copy correspondence is missing an aggregate node"))?;
        let mut children = BTreeMap::new();
        for (projection, child) in &source_aggregate.children {
            let copied_child = correspondence
                .get(child)
                .copied()
                .ok_or_else(|| oracle_error("copy correspondence is missing a child node"))?;
            children.insert(*projection, copied_child);
        }
        copied_aggregates.push((copied_node, children));
    }
    for (copied_node, children) in copied_aggregates {
        state
            .aggregates
            .insert(copied_node, super::state::RuntimeAggregate { children });
    }
    let copied_root = correspondence
        .get(&source_target.node)
        .copied()
        .ok_or_else(|| oracle_error("copy correspondence is missing its root node"))?;
    let transition = state.apply_definition_transition(
        problem,
        destination,
        DefinitionEventKind::Value,
        DefinitionRole::Slot {
            current: copied_root,
        },
        context.trace_index,
    )?;
    finish_definition_transition(context, &transition);
    Ok(EventExecutionResult::Continue)
}

fn execute_projection<'problem>(
    context: &mut OracleExecutionContext<'problem, '_>,
    event: &Event,
    source: PlaceId,
    destination: PlaceId,
    origin: ValueOriginId,
) -> Result<EventExecutionResult<'problem>, CompilerError> {
    let problem = context.problem;
    let place_index = context.place_index;
    let trace_index = context.trace_index;
    let bounds = context.bounds;
    require_place(problem, source, "projection source")?;
    require_place(problem, destination, "projection destination")?;
    let origin_row = require_origin(problem, origin, "projection origin")?;
    let projection = match &origin_row.kind {
        OriginKind::Projection { projection, .. } => *projection,
        _ => {
            return Err(oracle_error(format!(
                "projection event {:?} refers to non-projection origin {:?}",
                event.id, origin
            )));
        }
    };
    let source_target = match context.state.resolve_place(
        problem,
        place_index,
        source,
        bounds.max_dynamic_generations,
    )? {
        Ok(Some(source_target)) => source_target,
        Ok(None) => {
            return Err(oracle_error(format!(
                "projection source {:?} is unavailable at event {:?}",
                source, event.id
            )));
        }
        Err(reason) => return Ok(EventExecutionResult::Inconclusive(reason)),
    };
    let mut target_path = source_target.target.path.to_vec();
    target_path.push(projection);
    let target = match context.state.resolve_target(
        RuntimeAccessTarget {
            node: source_target.target.node,
            path: target_path.into_boxed_slice(),
        },
        bounds.max_dynamic_generations,
    )? {
        Ok(target) => target,
        Err(reason) => return Ok(EventExecutionResult::Inconclusive(reason)),
    };
    // A residual undecidable path left by the descent cannot be carried by a slot: the slot
    // state stores only a generation node, so installing `target.node` would make the
    // destination compare equal to the whole base node and manufacture a definite overlap the
    // contract classifies as UNDECIDABLE. The reference's Copy and Aggregate arms refuse the
    // same shape with `UndecidableOverlap` before installing anything, and the refusal carries
    // the residual target against the same node with an empty path, so the projection uses the
    // identical shape. This must fire before the transition and the capability, because both
    // would record state for a value the slot cannot represent.
    if !target.path.is_empty() {
        return Ok(EventExecutionResult::Inconclusive(
            OracleLimitReason::UndecidableOverlap {
                left: target.clone(),
                right: RuntimeAccessTarget {
                    node: target.node,
                    path: Box::new([]),
                },
            },
        ));
    }
    // A projection result is value-producing, so an unavailable destination installs a slot and
    // an established slot is replaced: the reference's projection arm calls `replace_generation`
    // with `BindingMode::Slot` for destinations that are neither alias-only nor mixed
    // (`origins.rs:1206-1213`). An alias-installing exception here kept a stale
    // aliasing relationship alive and manufactured the definition conflicts the transition table
    // must never produce.
    let transition = context.state.apply_definition_transition(
        problem,
        destination,
        DefinitionEventKind::Value,
        DefinitionRole::Slot {
            current: target.node,
        },
        trace_index,
    )?;
    finish_definition_transition(context, &transition);
    let capability_id = context.state.issue_capability(
        AccessKind::Shared,
        target,
        BTreeSet::from([destination]),
        trace_index,
        event.id,
        CapabilitySource::Provenance,
    )?;
    context.trace.record_issue(trace_index, capability_id);
    Ok(EventExecutionResult::Continue)
}

fn execute_rebind<'problem>(
    context: &mut OracleExecutionContext<'problem, '_>,
    event: &Event,
    destination: PlaceId,
    value: &RebindValue,
) -> Result<EventExecutionResult<'problem>, CompilerError> {
    let problem = context.problem;
    let place_index = context.place_index;
    let trace_index = context.trace_index;
    let bounds = context.bounds;
    require_place(problem, destination, "rebind destination")?;
    match value {
        RebindValue::Fresh(_) => {
            // Like every value-producing writer the destination action is decided before the
            // generation is allocated, so a write-through cannot consume generation bound.
            if let RuntimePlaceState::Alias { target, .. } = context.state.state(destination)? {
                let transition = context.state.apply_definition_transition(
                    problem,
                    destination,
                    DefinitionEventKind::Value,
                    DefinitionRole::Slot { current: target },
                    trace_index,
                )?;
                finish_definition_transition(context, &transition);
                return Ok(EventExecutionResult::Continue);
            }
            let generation = match context
                .state
                .issue_generation(bounds.max_dynamic_generations)
            {
                Ok(generation) => generation,
                Err(reason) => return Ok(EventExecutionResult::Inconclusive(reason)),
            };
            let transition = context.state.apply_definition_transition(
                problem,
                destination,
                DefinitionEventKind::Value,
                DefinitionRole::Slot {
                    current: generation,
                },
                trace_index,
            )?;
            finish_definition_transition(context, &transition);
            Ok(EventExecutionResult::Continue)
        }
        RebindValue::Alias(origins) => Ok(EventExecutionResult::Inconclusive(
            OracleLimitReason::RebindAliasOrigins {
                origins: origins.clone(),
            },
        )),
        RebindValue::AliasFromPlace(source) => {
            require_place(problem, *source, "place rebind source")?;

            // The value is ignored by a write-through. Check its availability without descending
            // projections, then let the transition owner preserve the existing alias.
            if let RuntimePlaceState::Alias { target, .. } = context.state.state(destination)? {
                if !is_place_available_without_materialising(problem, context.state, *source)? {
                    return Err(oracle_error(format!(
                        "rebind source {:?} is unavailable at event {:?}",
                        source, event.id
                    )));
                }
                let transition = context.state.apply_definition_transition(
                    problem,
                    destination,
                    DefinitionEventKind::Value,
                    DefinitionRole::Slot { current: target },
                    trace_index,
                )?;
                finish_definition_transition(context, &transition);
                return Ok(EventExecutionResult::Continue);
            }

            let resolved = match context.state.resolve_place(
                problem,
                place_index,
                *source,
                bounds.max_dynamic_generations,
            )? {
                Ok(Some(resolved)) => resolved,
                Ok(None) => {
                    return Err(oracle_error(format!(
                        "rebind source {:?} is unavailable at event {:?}",
                        source, event.id
                    )));
                }
                Err(reason) => return Ok(EventExecutionResult::Inconclusive(reason)),
            };
            // The slot can only carry a generation node, so a source resolution that leaves a
            // residual undecidable path would collapse the destination onto the whole base node
            // and manufacture a definite overlap the contract classifies as UNDECIDABLE. Refuse
            // before installing, in the same shape the Copy and Aggregate arms use.
            if !resolved.target.path.is_empty() {
                return Ok(EventExecutionResult::Inconclusive(
                    OracleLimitReason::UndecidableOverlap {
                        left: resolved.target.clone(),
                        right: RuntimeAccessTarget {
                            node: resolved.target.node,
                            path: Box::new([]),
                        },
                    },
                ));
            }
            // An alias-from-place rebind produces a value and a slot: it is routed through the
            // slot-producing event category so that a slot-backed destination has its slot
            // replaced and its holder retired, which an alias row would never do.
            let transition = context.state.apply_definition_transition(
                problem,
                destination,
                DefinitionEventKind::Value,
                DefinitionRole::Slot {
                    current: resolved.target.node,
                },
                trace_index,
            )?;
            finish_definition_transition(context, &transition);
            Ok(EventExecutionResult::Continue)
        }
    }
}

fn execute_aggregate<'problem>(
    context: &mut OracleExecutionContext<'problem, '_>,
    event: &Event,
    destination: PlaceId,
    fields: &[AggregateField],
) -> Result<EventExecutionResult<'problem>, CompilerError> {
    let problem = context.problem;
    require_place(problem, destination, "aggregate destination")?;
    // The destination action is decided before the aggregate graph is built: a write-through
    // onto an alias must not allocate an outer node or register its children. Each field still
    // needs a non-materialising availability check because the reference validates every input.
    if let RuntimePlaceState::Alias { target, .. } = context.state.state(destination)? {
        for field in fields {
            require_place(problem, field.source, "aggregate child")?;
            if !is_place_available_without_materialising(problem, context.state, field.source)? {
                return Err(oracle_error(format!(
                    "aggregate child {:?} is unavailable",
                    field.source
                )));
            }
        }
        let transition = context.state.apply_definition_transition(
            problem,
            destination,
            DefinitionEventKind::Value,
            DefinitionRole::Slot { current: target },
            context.trace_index,
        )?;
        finish_definition_transition(context, &transition);
        let source_targets = match resolve_aggregate_field_targets(context, fields)? {
            Ok(source_targets) => source_targets,
            Err(reason) => return Ok(EventExecutionResult::Inconclusive(reason)),
        };
        issue_aggregate_provenance(context, event, destination, fields, &source_targets)?;
        return Ok(EventExecutionResult::Continue);
    }

    let source_targets = match resolve_aggregate_field_targets(context, fields)? {
        Ok(source_targets) => source_targets,
        Err(reason) => return Ok(EventExecutionResult::Inconclusive(reason)),
    };
    // Aggregate children enter the graph only here, in `execute_copy` (which copies one guarded
    // map one for one) and in `descend` (which materialises only `Field` and `FixedIndex` edges),
    // so this check guards every insertion site. One child position cannot hold two distinct
    // nodes: either repeat would silently keep only the later one and detach the forgotten child
    // from every later observation of that position, which can turn a real conflict into a
    // reported safe. The reference still gives the shape semantics by extending the projected
    // place's alternatives with every repeated field's origins (`origins.rs:1308-1317`), so the
    // runtime graph is merely too small to represent the union, and the shape belongs in the
    // inconclusive lane.
    let mut children = BTreeMap::new();
    for (field, source_target) in fields.iter().zip(source_targets.iter()) {
        if let Some(&existing) = children.get(&field.projection)
            && existing != source_target.node
        {
            return Ok(EventExecutionResult::Inconclusive(
                OracleLimitReason::RepeatedProjectionChild {
                    destination,
                    projection: field.projection,
                    surviving: source_target.node,
                    forgotten: existing,
                },
            ));
        }
        children.insert(field.projection, source_target.node);
    }
    let outer = match context
        .state
        .issue_generation(context.bounds.max_dynamic_generations)
    {
        Ok(outer) => outer,
        Err(reason) => return Ok(EventExecutionResult::Inconclusive(reason)),
    };
    context
        .state
        .aggregates
        .insert(outer, super::state::RuntimeAggregate { children });
    let transition = context.state.apply_definition_transition(
        problem,
        destination,
        DefinitionEventKind::Value,
        DefinitionRole::Slot { current: outer },
        context.trace_index,
    )?;
    finish_definition_transition(context, &transition);
    issue_aggregate_provenance(context, event, destination, fields, &source_targets)?;
    Ok(EventExecutionResult::Continue)
}

fn resolve_aggregate_field_targets<'problem>(
    context: &mut OracleExecutionContext<'problem, '_>,
    fields: &[AggregateField],
) -> Result<Result<Vec<RuntimeAccessTarget>, OracleLimitReason>, CompilerError> {
    let problem = context.problem;
    let place_index = context.place_index;
    let bounds = context.bounds;
    let mut source_targets = Vec::with_capacity(fields.len());
    for field in fields {
        require_place(problem, field.source, "aggregate child")?;
        let source_target = match context.state.resolve_place(
            problem,
            place_index,
            field.source,
            bounds.max_dynamic_generations,
        )? {
            Ok(Some(source_target)) => source_target,
            Ok(None) => {
                return Err(oracle_error(format!(
                    "aggregate child {:?} is unavailable",
                    field.source
                )));
            }
            Err(reason) => return Ok(Err(reason)),
        };
        let source_target = match context
            .state
            .resolve_target(source_target.target, bounds.max_dynamic_generations)?
        {
            Ok(source_target) => source_target,
            Err(reason) => return Ok(Err(reason)),
        };
        if !source_target.path.is_empty() {
            return Ok(Err(OracleLimitReason::UndecidableOverlap {
                left: source_target.clone(),
                right: RuntimeAccessTarget {
                    node: source_target.node,
                    path: Box::new([]),
                },
            }));
        }
        source_targets.push(source_target);
    }
    Ok(Ok(source_targets))
}

fn issue_aggregate_provenance(
    context: &mut OracleExecutionContext<'_, '_>,
    event: &Event,
    destination: PlaceId,
    fields: &[AggregateField],
    source_targets: &[RuntimeAccessTarget],
) -> Result<(), CompilerError> {
    // A normalised projected place is the field holder. Synthetic inputs can omit that child,
    // so the destination is the same fallback used by the reference.
    debug_assert_eq!(fields.len(), source_targets.len());
    let problem = context.problem;
    let place_index = context.place_index;
    for (field, source_target) in fields.iter().zip(source_targets) {
        let holder = place_index
            .projected_place(problem, destination, field.projection)
            .unwrap_or(destination);
        let capability_id = context.state.issue_capability(
            AccessKind::Shared,
            source_target.clone(),
            BTreeSet::from([holder]),
            context.trace_index,
            event.id,
            CapabilitySource::Provenance,
        )?;
        context
            .trace
            .record_issue(context.trace_index, capability_id);
    }
    Ok(())
}

pub(super) fn execute_access(
    context: &mut OracleExecutionContext<'_, '_>,
    event: &Event,
    use_id: UseId,
    call: Option<CallId>,
) -> Result<AccessExecutionResult, CompilerError> {
    let problem = context.problem;
    let place_index = context.place_index;
    let state = &mut *context.state;
    let trace = &mut *context.trace;
    let trace_index = context.trace_index;
    let bounds = context.bounds;
    let use_row = require_use(problem, use_id, "access")?;
    require_place(problem, use_row.place, "access place")?;
    let access_kind = use_row.kind.access_kind();
    let pending_result = state.pending_call_results.contains_key(&use_row.place);
    let resolved = match state.resolve_place(
        problem,
        place_index,
        use_row.place,
        bounds.max_dynamic_generations,
    )? {
        Ok(resolved) => resolved,
        Err(reason) => return Ok(AccessExecutionResult::Inconclusive(reason)),
    };
    let (target, existing_state) = match resolved {
        // A defining write never installs anything: it resolves through the destination's state
        // as it was before the paired provenance event consumes that state, and the writer owns
        // the role transition alone. The reference still kills holders for a defining write
        // independently of that writer (`holder_kills`, `loans.rs:794-800`), so this write ends
        // every capability held by a structurally overlapping place before deferring. An
        // unavailable destination resolves to nothing, but a covered holder can still exist, so
        // the kill runs even on the deferred path.
        Some(resolved) => (resolved.target, Some(resolved.state)),
        None if use_row.definition => {
            let retired = state.retire_overlapping_holders(problem, use_row.place, trace_index)?;
            for capability_id in retired.iter().copied() {
                trace.record_end(trace_index, capability_id);
            }
            return Ok(AccessExecutionResult::Continue);
        }
        None => {
            return Err(oracle_error(format!(
                "non-defining access {:?} reads unavailable place {:?}",
                use_row.id, use_row.place
            )));
        }
    };

    let mut exercised = conflicts::exercise_capabilities(
        problem,
        state,
        use_row.place,
        &target,
        trace_index,
        call,
    )?
    .into_vec();
    // A definition is never conflict-checked: the reference's access_conflict_overlap returns
    // Ok(None) whenever access.definition. The post-hoc interval scan mirrors that by skipping
    // any capability the entry covers, so a non-write-through defining write must cover every
    // capability its target could overlap, not just the ones its place already holds. A write
    // through an alias-backed destination is different: the reference reclassifies such a use as
    // an ordinary access (`event_accesses`, `loans.rs:233`), so its conflicts are real and the
    // cover must not be installed for it. This is bookkeeping only: it marks the scanned entry,
    // it does not extend any capability interval.
    let write_through = matches!(existing_state, Some(RuntimePlaceState::Alias { .. }));
    if use_row.definition && !write_through {
        for capability_id in state.capabilities.keys().copied() {
            let overlapping = match state.capabilities.get(&capability_id) {
                Some(capability) => !matches!(
                    conflicts::dynamic_targets_overlap(&target, &capability.target()),
                    conflicts::DynamicOverlap::Disjoint
                ),
                None => false,
            };
            if overlapping && !exercised.contains(&capability_id) {
                exercised.push(capability_id);
            }
        }
        exercised.sort_unstable();
    }
    // The direct rule conflicts an exclusive access with a shared-alias candidate state. The
    // contract reclassifies the paired access of a write-through as an ordinary mutation
    // (`loans.rs:227-234`), so a defining write through a shared alias stays conflict-checked
    // exactly like a non-defining one. A pending call result never reaches this rule: its
    // entry is registered for a slot-backed result, and any event that would give the place
    // a different state before the confirming write is rejected as malformed, so a live
    // pending entry and an alias-backed candidate state are mutually exclusive.
    //
    // The candidate state alone cannot assert the conflict, because holder retirement ends
    // capabilities without touching place state, so a stale `Alias { access: Shared }` can
    // outlive the only capability it names. Exercise owns the oracle's one liveness decision
    // and has just skipped that ended capability above, so the rule requires its witness among
    // the capabilities this access exercised. A covered projection makes exact target equality
    // lose the conflict: the access resolves through the alias's residual path plus its own
    // projection, the capability still names the alias's base target, and the completed
    // interval scan would then skip that capability precisely because this access exercised
    // it. The witness is therefore the first exercised shared capability the same non-disjoint
    // target relation the interval scan uses selects, in deterministic exercise order:
    // exercise membership means the capability already passed holder coverage for this access,
    // so any non-disjoint target genuinely applies to it. Without one the access is legal and
    // falls through.
    let shared_alias_witness = if access_kind == AccessKind::Exclusive
        && matches!(
            existing_state,
            Some(RuntimePlaceState::Alias {
                access: AccessKind::Shared,
                ..
            })
        ) {
        exercised.iter().copied().find(|capability_id| {
            state
                .capabilities
                .get(capability_id)
                .is_some_and(|capability| {
                    capability.kind == AccessKind::Shared
                        && !matches!(
                            conflicts::dynamic_targets_overlap(&capability.target(), &target),
                            conflicts::DynamicOverlap::Disjoint
                        )
                })
        })
    } else {
        None
    };

    trace.record_access(
        trace_index,
        TraceAccess {
            place: use_row.place,
            kind: access_kind,
            target: target.clone(),
            definition: use_row.definition,
            exercised: exercised.into_boxed_slice(),
        },
    );
    // The reference's Access kill is separate from the writer's slot replacement: a defining
    // non-write-through write ends every capability held by a structurally overlapping holder
    // (`loans.rs:794-800`), which keeps the kill alive for a write whose provenance writer never
    // arrives. Retirement changes no destination role and allocates no generation.
    //
    // A defining access that still finds the pending entry for its own place live is the
    // builder's confirmation and is exempt: it never replaces the generation the CallEffect
    // defined, and retiring the overlapping holders here would end the provenance capabilities
    // the call just issued, so every later use of the result degrades into a retired-holder
    // exercise. The entry is bound to the generation that effect installed and any earlier
    // event that retires or replaces the result place is rejected at its own event, so only
    // this defining access can hold the exemption and a stale entry can never outlive the
    // generation it was registered against. Both the bound generation and the provenance
    // capabilities must survive their confirmation.
    if use_row.definition && !write_through && !pending_result {
        let retired = state.retire_overlapping_holders(problem, use_row.place, trace_index)?;
        for capability_id in retired.iter().copied() {
            trace.record_end(trace_index, capability_id);
        }
    }

    if let Some(capability_id) = shared_alias_witness {
        let capability = state
            .capabilities
            .get(&capability_id)
            .ok_or_else(|| oracle_error("lost a witness capability row on this execution"))?;
        return Ok(AccessExecutionResult::RuntimeConflict(
            RuntimeConflictWitness {
                access_event: event.id,
                access_index: trace_index,
                capability_id,
                capability_issue: capability.issue_index,
                access_kind,
                capability_kind: AccessKind::Shared,
                access_target: target.clone(),
                // The witness names the capability's own storage position, which a prefix
                // overlap no longer equals the access target, so the trace stays replayable
                // against the capability row it records.
                capability_target: capability.target(),
            },
        ));
    }

    if pending_result && use_row.definition {
        let expected = state.pending_call_results.remove(&use_row.place);
        if expected.as_ref() != Some(&target) {
            return Err(oracle_error(format!(
                "call-result confirmation for place {:?} changed its bound target",
                use_row.place
            )));
        }
    }
    Ok(AccessExecutionResult::Continue)
}
fn execute_loan_issue<'problem>(
    context: &mut OracleExecutionContext<'problem, '_>,
    event: &Event,
    loan_id: LoanId,
) -> Result<EventExecutionResult<'problem>, CompilerError> {
    let problem = context.problem;
    let place_index = context.place_index;
    let state = &mut *context.state;
    let trace = &mut *context.trace;
    let trace_index = context.trace_index;
    let bounds = context.bounds;
    let loan = require_loan(problem, loan_id, "loan issue")?;
    // Availability is checked without materialising graph state and before the cardinality
    // refusal: the authority classifies an unresolvable loan place or holder as malformed, and
    // validation accepts the event ordering that reaches here, so the checks must keep their
    // error lane even though the row is refused afterwards anyway. The refusal itself counts
    // DISTINCT holders, because validation does not require uniqueness and the capability set
    // below collapses a repeated place into one holder with nothing to retire twice.
    let holders: BTreeSet<PlaceId> = loan.holders.iter().copied().collect();
    if !is_place_available_without_materialising(problem, state, loan.place)? {
        return Err(oracle_error(format!(
            "loan {:?} place {:?} is unavailable",
            loan.id, loan.place
        )));
    }
    for holder in &holders {
        require_place(problem, *holder, "loan holder")?;
        if !is_place_available_without_materialising(problem, state, *holder)? {
            return Err(oracle_error(format!(
                "loan {:?} holder {:?} is unavailable",
                loan.id, holder
            )));
        }
    }
    // No producer emits a multi-holder loan row: HIR extraction publishes an empty explicit
    // loan table, every derived loan is single-holder, the deterministic generator emits one
    // holder and the reducer remaps holders one for one. The static solver applies a row's uses
    // and kills capability-wide with no per-holder model, so there is no per-holder reference
    // semantics to mirror, and the first retirement here would end the whole capability while
    // the surviving holders keep exercising it. Refuse the shape instead of inventing semantics.
    if holders.len() > 1 {
        return Ok(EventExecutionResult::Inconclusive(
            OracleLimitReason::MultiHolderLoan {
                loan: loan_id,
                holders: holders.len(),
            },
        ));
    }
    let target = match state.resolve_place(
        problem,
        place_index,
        loan.place,
        bounds.max_dynamic_generations,
    )? {
        Ok(Some(target)) => target,
        Ok(None) => {
            return Err(oracle_error(format!(
                "loan {:?} place {:?} is unavailable",
                loan.id, loan.place
            )));
        }
        Err(reason) => return Ok(EventExecutionResult::Inconclusive(reason)),
    };
    for holder in &holders {
        match state.resolve_place(
            problem,
            place_index,
            *holder,
            bounds.max_dynamic_generations,
        )? {
            Ok(Some(_)) => {}
            Ok(None) => {
                return Err(oracle_error(format!(
                    "loan {:?} holder {:?} is unavailable",
                    loan.id, holder
                )));
            }
            Err(reason) => return Ok(EventExecutionResult::Inconclusive(reason)),
        }
    }
    let capability_id = state.issue_capability(
        loan.kind,
        target.target,
        holders,
        trace_index,
        event.id,
        CapabilitySource::Loan(loan_id),
    )?;
    trace.record_issue(trace_index, capability_id);
    Ok(EventExecutionResult::Continue)
}

fn execute_loan_kill<'problem>(
    context: &mut OracleExecutionContext<'problem, '_>,
    loan_id: LoanId,
) -> Result<EventExecutionResult<'problem>, CompilerError> {
    let problem = context.problem;
    let state = &mut *context.state;
    let trace = &mut *context.trace;
    let trace_index = context.trace_index;
    require_loan(problem, loan_id, "loan kill")?;
    let capability_id = state.active_capability_for_loan(loan_id).ok_or_else(|| {
        oracle_error(format!(
            "loan kill {:?} has no active capability on this execution",
            loan_id
        ))
    })?;
    state.end_capability(capability_id, trace_index)?;
    trace.record_end(trace_index, capability_id);
    Ok(EventExecutionResult::Continue)
}
fn execute_terminator<'problem>(
    context: &OracleExecutionContext<'problem, '_>,
    kind: &'problem TerminatorEventKind,
) -> Result<EventExecutionResult<'problem>, CompilerError> {
    let problem = context.problem;
    match kind {
        TerminatorEventKind::Jump { target }
        | TerminatorEventKind::Break { target }
        | TerminatorEventKind::Continue { target } => {
            require_block(problem, *target)?;
            Ok(EventExecutionResult::NextBlock(*target))
        }
        TerminatorEventKind::Branch { targets } => {
            for target in targets.iter().copied() {
                require_block(problem, target)?;
            }
            Ok(EventExecutionResult::NextBlocks(targets))
        }
        TerminatorEventKind::Return
        | TerminatorEventKind::ReturnSuccess
        | TerminatorEventKind::ReturnError
        | TerminatorEventKind::RuntimeFailure
        | TerminatorEventKind::AssertFailure => Ok(EventExecutionResult::Complete),
    }
}

pub(super) fn validate_branch_targets(problem: &BorrowProblem) -> Result<(), CompilerError> {
    for event in problem.events() {
        let EventKind::Terminator {
            kind: TerminatorEventKind::Branch { targets },
        } = &event.kind
        else {
            continue;
        };
        if targets.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(oracle_error(format!(
                "branch event {:?} has targets outside ascending unique order",
                event.id
            )));
        }
    }
    Ok(())
}

pub(super) fn require_block(
    problem: &BorrowProblem,
    block: BlockId,
) -> Result<&CfgBlock, CompilerError> {
    let row = problem
        .control_flow()
        .blocks
        .get(block.index())
        .ok_or_else(|| {
            oracle_error(format!(
                "Boracle oracle cannot locate CFG block {:?}",
                block
            ))
        })?;
    if row.id != block {
        return Err(oracle_error(format!(
            "CFG block index {:?} names {:?}",
            block, row.id
        )));
    }
    Ok(row)
}

fn require_event(problem: &BorrowProblem, event: EventId) -> Result<&Event, CompilerError> {
    let row = problem
        .events()
        .get(event.index())
        .ok_or_else(|| oracle_error(format!("Boracle oracle cannot locate event {:?}", event)))?;
    if row.id != event {
        return Err(oracle_error(format!(
            "event index {:?} names {:?}",
            event, row.id
        )));
    }
    Ok(row)
}

pub(super) fn require_point(
    problem: &BorrowProblem,
    point: PointId,
) -> Result<&ProgramPoint, CompilerError> {
    let row = problem
        .points()
        .get(point.index())
        .ok_or_else(|| oracle_error(format!("Boracle oracle cannot locate point {:?}", point)))?;
    if row.id != point {
        return Err(oracle_error(format!(
            "point index {:?} names {:?}",
            point, row.id
        )));
    }
    Ok(row)
}

pub(super) fn require_place<'a>(
    problem: &'a BorrowProblem,
    place: PlaceId,
    owner: &str,
) -> Result<&'a Place, CompilerError> {
    let row = problem
        .places()
        .get(place.index())
        .ok_or_else(|| oracle_error(format!("{owner} references missing place {:?}", place)))?;
    if row.id != place {
        return Err(oracle_error(format!(
            "{owner} place index {:?} names {:?}",
            place, row.id
        )));
    }
    Ok(row)
}

pub(super) fn require_origin<'a>(
    problem: &'a BorrowProblem,
    origin: ValueOriginId,
    owner: &str,
) -> Result<&'a ValueOrigin, CompilerError> {
    let row = problem
        .origins()
        .get(origin.index())
        .ok_or_else(|| oracle_error(format!("{owner} references missing origin {:?}", origin)))?;
    if row.id != origin {
        return Err(oracle_error(format!(
            "{owner} origin index {:?} names {:?}",
            origin, row.id
        )));
    }
    Ok(row)
}

pub(super) fn require_use<'a>(
    problem: &'a BorrowProblem,
    use_id: UseId,
    owner: &str,
) -> Result<&'a Use, CompilerError> {
    let row = problem
        .uses()
        .get(use_id.index())
        .ok_or_else(|| oracle_error(format!("{owner} references missing use {:?}", use_id)))?;
    if row.id != use_id {
        return Err(oracle_error(format!(
            "{owner} use index {:?} names {:?}",
            use_id, row.id
        )));
    }
    Ok(row)
}

pub(super) fn require_loan<'a>(
    problem: &'a BorrowProblem,
    loan: LoanId,
    owner: &str,
) -> Result<&'a Loan, CompilerError> {
    let row = problem
        .loans()
        .get(loan.index())
        .ok_or_else(|| oracle_error(format!("{owner} references missing loan {:?}", loan)))?;
    if row.id != loan {
        return Err(oracle_error(format!(
            "{owner} loan index {:?} names {:?}",
            loan, row.id
        )));
    }
    Ok(row)
}

pub(super) fn require_call<'a>(
    problem: &'a BorrowProblem,
    call: CallId,
    owner: &str,
) -> Result<&'a Call, CompilerError> {
    let row = problem
        .calls()
        .get(call.index())
        .ok_or_else(|| oracle_error(format!("{owner} references missing call {:?}", call)))?;
    if row.id != call {
        return Err(oracle_error(format!(
            "{owner} call index {:?} names {:?}",
            call, row.id
        )));
    }
    Ok(row)
}

pub(super) fn oracle_error(message: impl Into<String>) -> CompilerError {
    CompilerError::compiler_error(format!("Boracle operational oracle: {}", message.into()))
}
