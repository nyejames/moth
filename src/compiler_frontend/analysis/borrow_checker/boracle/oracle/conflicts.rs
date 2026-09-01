//! Runtime overlap and completed-trace conflict decisions.
//!
//! WHAT: compares dynamic graph targets, maps accesses to holder coverage and applies the exact
//!       capability interval rule after a forward execution completes.
//! WHY: static place and origin overlap is intentionally not the owner of operational legality.

use super::OracleLimitReason;
use super::state::{
    CapabilityEndReason, CapabilitySource, OracleState, RuntimeAccessTarget, RuntimeCapability,
    RuntimeCapabilityId,
};
use super::traces::{RuntimeConflictWitness, TraceEntry};
use crate::compiler_frontend::analysis::borrow_checker::problem::{
    AccessKind, BorrowProblem, CallId, Place, PlaceId, ProjectionElem,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use std::collections::BTreeMap;
use std::ops::Bound::{Excluded, Unbounded};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DynamicOverlap {
    Disjoint,
    Overlap,
    Undecidable,
}

pub(crate) fn exercise_capabilities(
    problem: &BorrowProblem,
    state: &mut OracleState,
    place_id: PlaceId,
    target: &RuntimeAccessTarget,
    event_index: usize,
    excluded_call: Option<CallId>,
) -> Result<Box<[RuntimeCapabilityId]>, CompilerError> {
    let place = problem.places().get(place_id.index()).ok_or_else(|| {
        CompilerError::compiler_error(format!(
            "Boracle oracle cannot locate access place {:?}",
            place_id
        ))
    })?;
    if place.id != place_id {
        return Err(CompilerError::compiler_error(format!(
            "Boracle oracle access place index {:?} names {:?}",
            place_id, place.id
        )));
    }

    let capability_ids = state.capabilities.keys().copied().collect::<Vec<_>>();
    let mut exercised = Vec::new();
    for capability_id in capability_ids {
        let (covered, covered_through_surviving_holder, source, ended, end_reason, post_call) = {
            let capability = state.capabilities.get(&capability_id).ok_or_else(|| {
                CompilerError::compiler_error("Boracle oracle lost a capability row")
            })?;
            // An open invocation withholds its own argument capabilities from sibling arguments.
            // Once its effect is recorded, `post_call` below keeps that capability out of later
            // exercises.
            let excluded = excluded_call.is_some_and(|call| {
                capability.call_effect_index.is_none()
                    && matches!(
                        capability.source,
                        CapabilitySource::CallArgument(source_call) if source_call == call
                    )
            });
            // The reference gives a call-argument loan an until-event at CallEffect. A later
            // access is legal but cannot exercise that ended loan, so do not advance
            // `last_exercised` and leave a misleading trace that the interval cap hides.
            let post_call = matches!(capability.source, CapabilitySource::CallArgument(_))
                && capability
                    .call_effect_index
                    .is_some_and(|call_effect_index| event_index > call_effect_index);
            let mut covered = false;
            let mut covered_through_surviving_holder = false;
            if !excluded {
                for holder_id in &capability.holders {
                    if !holder_covers_place(problem, *holder_id, place) {
                        continue;
                    }
                    covered = true;
                    if !capability.retired_holders.contains(holder_id) {
                        covered_through_surviving_holder = true;
                    }
                }
            }
            (
                covered,
                covered_through_surviving_holder,
                capability.source,
                capability
                    .explicit_end
                    .is_some_and(|explicit_end| event_index > explicit_end),
                capability.end_reason,
                post_call,
            )
        };
        if post_call {
            continue;
        }
        if !covered {
            continue;
        }
        if ended {
            match end_reason {
                Some(CapabilityEndReason::HolderRetired) if !covered_through_surviving_holder => {
                    continue;
                }
                Some(CapabilityEndReason::HolderRetired) => {
                    return Err(CompilerError::compiler_error(format!(
                        "Boracle oracle access at event {} exercises capability {:?} after its end",
                        event_index, capability_id
                    )));
                }
                Some(CapabilityEndReason::LoanKill)
                    if has_later_covering_capability(
                        state,
                        problem,
                        capability_id,
                        source,
                        place,
                        target,
                    ) =>
                {
                    continue;
                }
                Some(CapabilityEndReason::LoanKill) => {
                    return Err(CompilerError::compiler_error(format!(
                        "Boracle oracle access at event {} exercises capability {:?} after its end",
                        event_index, capability_id
                    )));
                }
                None => {
                    return Err(CompilerError::compiler_error(format!(
                        "Boracle oracle capability {:?} has an end without a reason",
                        capability_id
                    )));
                }
            }
        }
        let capability = state
            .capabilities
            .get_mut(&capability_id)
            .ok_or_else(|| CompilerError::compiler_error("Boracle oracle lost a capability row"))?;
        capability.last_exercised = event_index;
        exercised.push(capability_id);
    }
    Ok(exercised.into_boxed_slice())
}

fn holder_covers_place(problem: &BorrowProblem, holder_id: PlaceId, place: &Place) -> bool {
    problem
        .places()
        .get(holder_id.index())
        .is_some_and(|holder| {
            holder.id == holder_id
                && holder.root == place.root
                && holder.projections.len() <= place.projections.len()
                && holder.projections.as_ref() == &place.projections[..holder.projections.len()]
        })
}

fn capability_covers_place(
    problem: &BorrowProblem,
    capability: &RuntimeCapability,
    place: &Place,
) -> bool {
    capability
        .holders
        .iter()
        .any(|holder_id| holder_covers_place(problem, *holder_id, place))
}

fn has_later_covering_capability(
    state: &OracleState,
    problem: &BorrowProblem,
    capability_id: RuntimeCapabilityId,
    source: CapabilitySource,
    place: &Place,
    target: &RuntimeAccessTarget,
) -> bool {
    state
        .capabilities
        .range((Excluded(capability_id), Unbounded))
        .any(|(_, capability)| {
            capability.source == source
                && matches!(
                    dynamic_targets_overlap(&capability.target(), target),
                    DynamicOverlap::Overlap
                )
                && capability_covers_place(problem, capability, place)
        })
}

pub(crate) fn dynamic_targets_overlap(
    left: &RuntimeAccessTarget,
    right: &RuntimeAccessTarget,
) -> DynamicOverlap {
    if left.node != right.node {
        return DynamicOverlap::Disjoint;
    }

    let common_length = left.path.len().min(right.path.len());
    for index in 0..common_length {
        let left_element = left.path[index];
        let right_element = right.path[index];
        if left_element == right_element {
            continue;
        }
        return match (left_element, right_element) {
            (ProjectionElem::Field(left), ProjectionElem::Field(right)) if left != right => {
                DynamicOverlap::Disjoint
            }
            (ProjectionElem::FixedIndex(left), ProjectionElem::FixedIndex(right))
                if left != right =>
            {
                DynamicOverlap::Disjoint
            }
            _ => DynamicOverlap::Undecidable,
        };
    }
    DynamicOverlap::Overlap
}

pub(crate) fn find_interval_conflict(
    entries: &[TraceEntry],
    capabilities: &BTreeMap<RuntimeCapabilityId, RuntimeCapability>,
) -> Result<Option<RuntimeConflictWitness>, OracleLimitReason> {
    // A proven conflict outranks an undecidable pair, so the first undecidable reason is retained
    // and the scan continues. Returning the unknown immediately would hide a definite witness that
    // a later capability or a later access proves, and would report a real conflict as
    // inconclusive. The retained reason is the first in this deterministic iteration order.
    let mut undecidable: Option<OracleLimitReason> = None;
    for entry in entries {
        let Some(access) = entry.access.as_ref() else {
            continue;
        };
        for (capability_id, capability) in capabilities {
            if access.exercised.contains(capability_id) {
                continue;
            }
            let interval_end = capability.interval_end();
            if entry.index < capability.issue_index || entry.index > interval_end {
                continue;
            }
            if access.kind == AccessKind::Shared && capability.kind == AccessKind::Shared {
                continue;
            }
            match dynamic_targets_overlap(&access.target, &capability.target()) {
                DynamicOverlap::Disjoint => continue,
                DynamicOverlap::Undecidable => {
                    if undecidable.is_none() {
                        undecidable = Some(OracleLimitReason::UndecidableOverlap {
                            left: access.target.clone(),
                            right: capability.target(),
                        });
                    }
                }
                DynamicOverlap::Overlap => {
                    return Ok(Some(RuntimeConflictWitness {
                        access_event: entry.event,
                        access_index: entry.index,
                        capability_id: *capability_id,
                        capability_issue: capability.issue_index,
                        access_kind: access.kind,
                        capability_kind: capability.kind,
                        access_target: access.target.clone(),
                        capability_target: capability.target(),
                    }));
                }
            }
        }
    }
    match undecidable {
        Some(reason) => Err(reason),
        None => Ok(None),
    }
}
