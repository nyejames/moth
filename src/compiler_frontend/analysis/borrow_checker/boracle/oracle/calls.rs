//! Normalized call argument and result execution.
//!
//! WHAT: checks granular argument accesses, keeps their capabilities open through CallEffect and
//!       binds call results to dynamic storage while preserving source provenance relationships.
//! WHY: calls have no reservation object in the normalized IR, so the oracle models their current
//!      interval semantics directly.

use super::OracleLimitReason;
use super::execute::{
    AccessExecutionResult, EventExecutionResult, OracleExecutionContext, execute_access,
    finish_definition_transition, is_place_available_without_materialising, oracle_error,
    require_call, require_origin, require_place, require_use,
};
use super::state::{
    CapabilitySource, DefinitionEventKind, DefinitionRole, RuntimeAccessTarget, RuntimePlaceState,
};
use crate::compiler_frontend::analysis::borrow_checker::problem::{
    AccessKind, BorrowProblem, CallArgument, CallEffect, CallId, CallResultProvenance, Event,
    EventKind, OriginKind, PlaceId,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use std::collections::BTreeSet;

pub(super) fn execute_call_argument<'a>(
    context: &mut OracleExecutionContext<'a, '_>,
    event: &Event,
    call: CallId,
    index: u32,
    argument: &CallArgument,
) -> Result<EventExecutionResult<'a>, CompilerError> {
    require_place(context.problem, argument.place, "call argument place")?;
    let use_row = require_use(context.problem, argument.use_id, "call argument")?;
    if use_row.point != event.point || use_row.place != argument.place {
        return Err(oracle_error(format!(
            "call argument use {:?} does not match event {:?}",
            argument.use_id, event.id
        )));
    }
    let expected_access = use_row.kind.access_kind();
    if expected_access != argument.access {
        return Err(oracle_error(format!(
            "call argument use {:?} has access {:?}, expected {:?}",
            argument.use_id, argument.access, expected_access
        )));
    }
    let effect = find_call_effect(context.problem, call).ok_or_else(|| {
        oracle_error(format!(
            "call argument {:?} has no matching CallEffect for call {:?}",
            argument.use_id, call
        ))
    })?;
    let expected_argument = effect.arguments.get(index as usize).ok_or_else(|| {
        oracle_error(format!(
            "call argument index {} is outside call {:?}",
            index, call
        ))
    })?;
    if expected_argument != argument {
        return Err(oracle_error(format!(
            "call argument {} does not match its CallEffect row for call {:?}",
            index, call
        )));
    }

    let result = execute_access(context, event, argument.use_id, Some(call))?;
    let problem = context.problem;
    let state = &mut *context.state;
    let trace = &mut *context.trace;
    let trace_index = context.trace_index;
    let bounds = context.bounds;
    match result {
        AccessExecutionResult::RuntimeConflict(witness) => {
            Ok(EventExecutionResult::RuntimeConflict(witness))
        }
        AccessExecutionResult::Inconclusive(reason) => {
            Ok(EventExecutionResult::Inconclusive(reason))
        }
        AccessExecutionResult::Continue => {
            let resolved = match state.resolve_place(
                problem,
                context.place_index,
                argument.place,
                bounds.max_dynamic_generations,
            )? {
                Ok(Some(resolved)) => resolved,
                Ok(None) => {
                    return Err(oracle_error(format!(
                        "call argument place {:?} became unavailable after access",
                        argument.place
                    )));
                }
                Err(reason) => return Ok(EventExecutionResult::Inconclusive(reason)),
            };
            let capability_id = state.issue_capability(
                argument.access,
                resolved.target,
                BTreeSet::from([argument.place]),
                trace_index,
                event.id,
                CapabilitySource::CallArgument(call),
            )?;
            trace.record_issue(trace_index, capability_id);
            trace.record_exercise(trace_index, capability_id);
            Ok(EventExecutionResult::Continue)
        }
    }
}

pub(super) fn execute_call_effect<'a>(
    context: &mut OracleExecutionContext<'a, '_>,
    event: &Event,
    effect: &CallEffect,
) -> Result<EventExecutionResult<'a>, CompilerError> {
    let problem = context.problem;
    let trace_index = context.trace_index;
    let bounds = context.bounds;
    require_call(problem, effect.call, "call effect")?;
    verify_granular_arguments(problem, effect)?;
    context
        .state
        .extend_call_capabilities(effect.call, trace_index);

    let Some(result) = effect.result else {
        return Ok(EventExecutionResult::Continue);
    };
    let place_row = require_place(problem, result.place, "call result place")?;
    let origin = require_origin(problem, result.origin, "call result origin")?;
    let OriginKind::CallResult {
        call: origin_call,
        provenance,
    } = &origin.kind
    else {
        return Err(oracle_error(format!(
            "call result origin {:?} is not a CallResult origin",
            result.origin
        )));
    };
    if *origin_call != effect.call {
        return Err(oracle_error(format!(
            "call result origin {:?} belongs to {:?}, not {:?}",
            result.origin, origin_call, effect.call
        )));
    }

    let target = match provenance {
        CallResultProvenance::Fresh => {
            // As with the value-producing events the write-through check precedes the
            // generation allocation, so an alias-backed result cannot consume the bound.
            if let RuntimePlaceState::Alias { target, .. } = context.state.state(result.place)? {
                let transition = context.state.apply_definition_transition(
                    problem,
                    result.place,
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
                result.place,
                DefinitionEventKind::Value,
                DefinitionRole::Slot {
                    current: generation,
                },
                trace_index,
            )?;
            finish_definition_transition(context, &transition);
            transition.target().clone()
        }
        CallResultProvenance::AliasParams(parameter_indices) => {
            // A write-through result ignores its incoming value, but every argument remains a
            // validated input and its provenance relationship is still emitted by the reference.
            if let RuntimePlaceState::Alias { target, .. } = context.state.state(result.place)? {
                for parameter_index in parameter_indices {
                    let argument = effect.arguments.get(*parameter_index).ok_or_else(|| {
                        oracle_error(format!(
                            "call result origin {:?} references argument {} outside call {:?}",
                            result.origin, parameter_index, effect.call
                        ))
                    })?;
                    require_place(problem, argument.place, "call result argument place")?;
                    if !is_place_available_without_materialising(
                        problem,
                        context.state,
                        argument.place,
                    )? {
                        return Err(oracle_error(format!(
                            "call result argument place {:?} is unavailable",
                            argument.place
                        )));
                    }
                }
                let transition = context.state.apply_definition_transition(
                    problem,
                    result.place,
                    DefinitionEventKind::Value,
                    DefinitionRole::Slot { current: target },
                    trace_index,
                )?;
                finish_definition_transition(context, &transition);
                let source_targets =
                    match resolve_alias_params_arguments(context, effect, parameter_indices)? {
                        Ok(source_targets) => source_targets,
                        Err(reason) => return Ok(EventExecutionResult::Inconclusive(reason)),
                    };
                issue_call_result_provenance(context, event, result.place, &source_targets)?;
                return Ok(EventExecutionResult::Continue);
            }

            let source_targets =
                match resolve_alias_params_arguments(context, effect, parameter_indices)? {
                    Ok(source_targets) => source_targets,
                    Err(reason) => return Ok(EventExecutionResult::Inconclusive(reason)),
                };
            let generation = match context
                .state
                .issue_generation(bounds.max_dynamic_generations)
            {
                Ok(generation) => generation,
                Err(reason) => return Ok(EventExecutionResult::Inconclusive(reason)),
            };
            let transition = context.state.apply_definition_transition(
                problem,
                result.place,
                DefinitionEventKind::Value,
                DefinitionRole::Slot {
                    current: generation,
                },
                trace_index,
            )?;
            finish_definition_transition(context, &transition);
            issue_call_result_provenance(context, event, result.place, &source_targets)?;
            transition.target().clone()
        }
        CallResultProvenance::Alias(origins) => {
            return Ok(EventExecutionResult::Inconclusive(
                OracleLimitReason::CallResultAliasOrigins {
                    call: effect.call,
                    origins: origins.clone(),
                },
            ));
        }
        CallResultProvenance::Unknown(reason) => {
            return Ok(EventExecutionResult::Inconclusive(
                OracleLimitReason::CallResultUnknown {
                    call: effect.call,
                    reason: *reason,
                },
            ));
        }
    };
    // The pending entry is bound to the exact generation the transition above just installed:
    // only a defining access to the result place consumes it, and the state layer rejects any
    // other event that retires or replaces that place first (`apply_definition_transition`,
    // `end_holders`). Keeping that binding is what keeps the confirming write's exemptions in
    // `execute_access` from suppressing detection through a stale entry.
    //
    // The registration itself only happens where a confirmation can exist at all. The builder
    // emits every call result into a local's root place, and validation rejects projected
    // definitions, so a projected result place has no defining write that could confirm it and
    // must not register a pending entry that would otherwise only dangle.
    if place_row.projections.is_empty() {
        context
            .state
            .pending_call_results
            .insert(result.place, target);
    }
    Ok(EventExecutionResult::Continue)
}

fn resolve_alias_params_arguments<'a>(
    context: &mut OracleExecutionContext<'a, '_>,
    effect: &CallEffect,
    parameter_indices: &[usize],
) -> Result<Result<Vec<RuntimeAccessTarget>, OracleLimitReason>, CompilerError> {
    let problem = context.problem;
    let place_index = context.place_index;
    let bounds = context.bounds;
    let mut source_targets = Vec::with_capacity(parameter_indices.len());
    for parameter_index in parameter_indices {
        let argument = effect.arguments.get(*parameter_index).ok_or_else(|| {
            oracle_error(format!(
                "call result references argument {} outside call {:?}",
                parameter_index, effect.call
            ))
        })?;
        require_place(problem, argument.place, "call result argument place")?;
        let source = match context.state.resolve_place(
            problem,
            place_index,
            argument.place,
            bounds.max_dynamic_generations,
        )? {
            Ok(Some(source)) => source,
            Ok(None) => {
                return Err(oracle_error(format!(
                    "call result argument place {:?} is unavailable",
                    argument.place
                )));
            }
            Err(reason) => return Ok(Err(reason)),
        };
        source_targets.push(source.target);
    }
    Ok(Ok(source_targets))
}

fn issue_call_result_provenance(
    context: &mut OracleExecutionContext<'_, '_>,
    event: &Event,
    result_place: PlaceId,
    source_targets: &[RuntimeAccessTarget],
) -> Result<(), CompilerError> {
    for source_target in source_targets {
        let capability_id = context.state.issue_capability(
            AccessKind::Shared,
            source_target.clone(),
            BTreeSet::from([result_place]),
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

fn verify_granular_arguments(
    problem: &BorrowProblem,
    effect: &CallEffect,
) -> Result<(), CompilerError> {
    let mut found = vec![false; effect.arguments.len()];
    for event in problem.events() {
        let EventKind::CallArgument {
            call,
            index,
            argument,
        } = &event.kind
        else {
            continue;
        };
        if *call != effect.call {
            continue;
        }
        let Some(expected) = effect.arguments.get(*index as usize) else {
            return Err(oracle_error(format!(
                "call argument index {} is outside CallEffect {:?}",
                index, effect.call
            )));
        };
        if expected != argument {
            return Err(oracle_error(format!(
                "call argument {} does not match CallEffect {:?}",
                index, effect.call
            )));
        }
        found[*index as usize] = true;
    }
    if found.iter().any(|found| !found) {
        return Err(oracle_error(format!(
            "call {:?} has a missing granular argument event",
            effect.call
        )));
    }
    Ok(())
}

struct CallEffectRow<'a> {
    arguments: &'a [CallArgument],
}

fn find_call_effect<'a>(problem: &'a BorrowProblem, call: CallId) -> Option<CallEffectRow<'a>> {
    problem.events().iter().find_map(|event| {
        let EventKind::CallEffect(effect) = &event.kind else {
            return None;
        };
        (effect.call == call).then_some(CallEffectRow {
            arguments: &effect.arguments,
        })
    })
}
