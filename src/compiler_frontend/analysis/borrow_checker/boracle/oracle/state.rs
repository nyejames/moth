//! Dynamic execution state owned by the operational oracle.
//!
//! WHAT: stores concrete value generations, aggregate child edges, place states and live
//!       capabilities for one execution.
//! WHY: the operational oracle must make runtime aliasing decisions without reconstructing the
//!      reference solver's origin or loan state.

use super::OracleLimitReason;
use crate::compiler_frontend::analysis::borrow_checker::problem::{
    AccessKind, BindingId, BlockId, BorrowProblem, CallId, EventId, LoanId, PlaceId, PlaceOverlap,
    ProjectionElem,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct DynamicOriginId(u32);

impl DynamicOriginId {
    pub(crate) const fn raw(self) -> u32 {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RuntimePlaceState {
    Unavailable,
    Slot {
        current: DynamicOriginId,
    },
    Alias {
        target: DynamicOriginId,
        path: Box<[ProjectionElem]>,
        access: AccessKind,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DefinitionEventKind {
    Value,
    DirectAlias,
    MutableParameter,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DefinitionRole {
    Slot {
        current: DynamicOriginId,
    },
    Alias {
        target: RuntimeAccessTarget,
        access: AccessKind,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DefinitionTransition {
    Installed {
        target: RuntimeAccessTarget,
        retired_capabilities: Box<[RuntimeCapabilityId]>,
    },
    ReplacedSlot {
        target: RuntimeAccessTarget,
        retired_capabilities: Box<[RuntimeCapabilityId]>,
    },
    /// The alias row covers shared and exclusive destinations identically: both write through
    /// to the referent, keep the alias state and allocate nothing. The reference solver never
    /// checks a definition against a loan, and its alias arm tests one unqualified
    /// `BindingMode::Alias`, so there is no conflict row and no access-kind split here.
    WriteThroughAlias { target: RuntimeAccessTarget },
}

impl DefinitionTransition {
    pub(crate) fn target(&self) -> &RuntimeAccessTarget {
        match self {
            Self::Installed { target, .. }
            | Self::ReplacedSlot { target, .. }
            | Self::WriteThroughAlias { target } => target,
        }
    }
}

impl DefinitionRole {
    fn slot_target(&self) -> RuntimeAccessTarget {
        match self {
            Self::Slot { current } => RuntimeAccessTarget {
                node: *current,
                path: Box::new([]),
            },
            Self::Alias { target, .. } => RuntimeAccessTarget {
                node: target.node,
                path: Box::new([]),
            },
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct RuntimeAggregate {
    pub(crate) children: BTreeMap<ProjectionElem, DynamicOriginId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RuntimeAccessTarget {
    pub(crate) node: DynamicOriginId,
    pub(crate) path: Box<[ProjectionElem]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct RuntimeCapabilityId(u32);

impl RuntimeCapabilityId {
    pub(crate) const fn raw(self) -> u32 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CapabilitySource {
    Alias,
    /// A value relationship derived from a source place rather than a direct alias declaration.
    Provenance,
    CallArgument(CallId),
    Loan(LoanId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CapabilityEndReason {
    HolderRetired,
    LoanKill,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RuntimeCapability {
    pub(crate) kind: AccessKind,
    pub(crate) target: DynamicOriginId,
    pub(crate) path: Box<[ProjectionElem]>,
    pub(crate) holders: BTreeSet<PlaceId>,
    pub(crate) issue_index: usize,
    pub(crate) issue_event: EventId,
    pub(crate) last_exercised: usize,
    pub(crate) explicit_end: Option<usize>,
    pub(crate) end_reason: Option<CapabilityEndReason>,
    pub(crate) retired_holders: BTreeSet<PlaceId>,
    pub(crate) call_effect_index: Option<usize>,
    pub(crate) source: CapabilitySource,
}

impl RuntimeCapability {
    pub(crate) fn interval_end(&self) -> usize {
        // A call argument remains live through its CallEffect but not after it, so the effect is
        // both floor and ceiling. `extend_call_capabilities` raises the interval to the effect and
        // this caps it there.
        //
        // The cap is a backstop, not the primary rule. `exercise_capabilities` already refuses to
        // advance `last_exercised` past a recorded effect, so while that guard holds no execution
        // reaches this `min` with a larger value. It stays because the invariant is then structural:
        // a later exercise site that forgets the guard still cannot turn a legal post-call access
        // into a conflict. Its test drives this method directly, because no end-to-end path can
        // build the state it defends against.
        let mut end = self.last_exercised;
        if let Some(call_effect_index) = self.call_effect_index {
            end = end.min(call_effect_index);
        }
        if let Some(explicit_end) = self.explicit_end {
            end = end.min(explicit_end);
        }
        end
    }

    pub(crate) fn target(&self) -> RuntimeAccessTarget {
        RuntimeAccessTarget {
            node: self.target,
            path: self.path.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedPlace {
    pub(crate) candidate: PlaceId,
    pub(crate) state: RuntimePlaceState,
    pub(crate) target: RuntimeAccessTarget,
}

/// Immutable lookup of the places interned by one normalised borrow problem.
///
/// The index is shared by every path. Keeping it outside [`OracleState`] means a branch fork only
/// clones dynamic execution data.
#[derive(Debug, Clone)]
pub(super) struct PlaceIndex {
    place_lookup: BTreeMap<(BindingId, Box<[ProjectionElem]>), PlaceId>,
}

impl PlaceIndex {
    pub(super) fn new(problem: &BorrowProblem) -> Self {
        let place_lookup = problem
            .places()
            .iter()
            .map(|place| ((place.root, place.projections.clone()), place.id))
            .collect();
        Self { place_lookup }
    }

    fn lookup(&self, root: BindingId, projections: &[ProjectionElem]) -> Option<PlaceId> {
        self.place_lookup
            .get(&(root, projections.to_vec().into_boxed_slice()))
            .copied()
    }

    pub(super) fn projected_place(
        &self,
        problem: &BorrowProblem,
        base: PlaceId,
        projection: ProjectionElem,
    ) -> Option<PlaceId> {
        let base = problem.places().get(base.index())?;
        let mut projections = base.projections.to_vec();
        projections.push(projection);
        self.lookup(base.root, &projections)
    }
}

/// A cycle comparison includes all runtime data and identity counters, but not path accounting or
/// block-entry bookkeeping.
#[derive(Debug, Clone, PartialEq, Eq)]
struct DynamicStateSnapshot {
    places: BTreeMap<PlaceId, RuntimePlaceState>,
    aggregates: BTreeMap<DynamicOriginId, RuntimeAggregate>,
    capabilities: BTreeMap<RuntimeCapabilityId, RuntimeCapability>,
    pending_call_results: BTreeMap<PlaceId, RuntimeAccessTarget>,
    generation_count: usize,
    next_generation: u32,
    next_capability: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OracleState {
    pub(crate) places: BTreeMap<PlaceId, RuntimePlaceState>,
    pub(crate) aggregates: BTreeMap<DynamicOriginId, RuntimeAggregate>,
    pub(crate) capabilities: BTreeMap<RuntimeCapabilityId, RuntimeCapability>,
    pub(crate) pending_call_results: BTreeMap<PlaceId, RuntimeAccessTarget>,
    block_entry_states: BTreeMap<BlockId, Vec<DynamicStateSnapshot>>,
    pub(crate) generation_count: usize,
    pub(crate) next_generation: u32,
    pub(crate) next_capability: u32,
    pub(crate) executed_events: usize,
}

impl OracleState {
    pub(crate) fn new(problem: &BorrowProblem) -> Self {
        let places = problem
            .places()
            .iter()
            .map(|place| (place.id, RuntimePlaceState::Unavailable))
            .collect();
        Self {
            places,
            aggregates: BTreeMap::new(),
            capabilities: BTreeMap::new(),
            pending_call_results: BTreeMap::new(),
            block_entry_states: BTreeMap::new(),
            generation_count: 0,
            next_generation: 0,
            next_capability: 0,
            executed_events: 0,
        }
    }

    /// Records one entry to `block` on this path. A dynamic state equal to an earlier entry of the
    /// same block is a closed cycle that cannot progress, so it is refused before the bound.
    pub(crate) fn enter_block(
        &mut self,
        block: BlockId,
        max_block_entries: usize,
    ) -> Option<OracleLimitReason> {
        if self.is_repeated_block_entry(block) {
            return Some(OracleLimitReason::NonTerminatingCycle { block });
        }
        // A block with no recorded entry yet counts as zero, so a zero bound refuses its first
        // entry rather than letting a missing row skip the check.
        if self.block_entry_states.get(&block).map_or(0, Vec::len) >= max_block_entries {
            return Some(OracleLimitReason::BlockEntryBound {
                block,
                limit: max_block_entries,
            });
        }
        let snapshot = self.dynamic_snapshot();
        self.block_entry_states
            .entry(block)
            .or_default()
            .push(snapshot);
        None
    }

    pub(crate) fn is_repeated_block_entry(&self, block: BlockId) -> bool {
        self.block_entry_states
            .get(&block)
            .is_some_and(|entries| entries.iter().any(|entry| self.matches_snapshot(entry)))
    }

    fn matches_snapshot(&self, snapshot: &DynamicStateSnapshot) -> bool {
        self.generation_count == snapshot.generation_count
            && self.next_generation == snapshot.next_generation
            && self.next_capability == snapshot.next_capability
            && self.places == snapshot.places
            && self.aggregates == snapshot.aggregates
            && self.capabilities == snapshot.capabilities
            && self.pending_call_results == snapshot.pending_call_results
    }

    pub(crate) fn block_entry_counts(&self) -> BTreeMap<BlockId, usize> {
        self.block_entry_states
            .iter()
            .map(|(block, entries)| (*block, entries.len()))
            .collect()
    }

    fn dynamic_snapshot(&self) -> DynamicStateSnapshot {
        DynamicStateSnapshot {
            places: self.places.clone(),
            aggregates: self.aggregates.clone(),
            capabilities: self.capabilities.clone(),
            pending_call_results: self.pending_call_results.clone(),
            generation_count: self.generation_count,
            next_generation: self.next_generation,
            next_capability: self.next_capability,
        }
    }

    pub(crate) fn issue_generation(
        &mut self,
        max_dynamic_generations: usize,
    ) -> Result<DynamicOriginId, OracleLimitReason> {
        if self.generation_count >= max_dynamic_generations {
            return Err(OracleLimitReason::GenerationBound {
                limit: max_dynamic_generations,
            });
        }
        let Some(next_generation) = self.next_generation.checked_add(1) else {
            return Err(OracleLimitReason::GenerationBound {
                limit: max_dynamic_generations,
            });
        };
        let generation = DynamicOriginId(self.next_generation);
        self.next_generation = next_generation;
        self.generation_count += 1;
        Ok(generation)
    }

    pub(crate) fn state(&self, place: PlaceId) -> Result<RuntimePlaceState, CompilerError> {
        self.places.get(&place).cloned().ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "Boracle oracle cannot resolve uninterned place {:?}",
                place
            ))
        })
    }
    pub(crate) fn set_state(&mut self, place: PlaceId, state: RuntimePlaceState) {
        self.places.insert(place, state);
    }

    /// Applies the concrete role of one definition-producing event.
    ///
    /// `role` describes the destination as it was BEFORE the paired defining access, so the
    /// access must not have installed anything. The destination's pre-definition state alone
    /// decides the row: only an uninitialised destination uses the event category to choose
    /// between an alias and a slot, and an established slot stays slot-backed while every alias
    /// row writes through and retires nothing. The two non-write-through rows retire every
    /// capability held by a place that structurally overlaps the destination, because the
    /// reference's `holder_kills` keys the kill on structural overlap, not on holder identity
    /// (`loans.rs:790-805`), so a bare rebind of a root must also end a capability held by a
    /// covered projection. An already-ended capability is skipped here, which is what keeps the
    /// paired access's own retirement from double-reporting the same end, and callers record
    /// the returned ends in the trace.
    ///
    /// Projection and alias-parameters results belong in the value row: the reference's
    /// projection arm and result arm both call `replace_generation` with `BindingMode::Slot`
    /// for destinations that are neither alias-only nor mixed (`origins.rs:1206-1213` and
    /// `origins.rs:1367-1370`). Keeping an alias-installing exception for them would
    /// manufacture the definition conflicts the table must never produce.
    pub(crate) fn apply_definition_transition(
        &mut self,
        problem: &BorrowProblem,
        destination: PlaceId,
        event_kind: DefinitionEventKind,
        role: DefinitionRole,
        event_index: usize,
    ) -> Result<DefinitionTransition, CompilerError> {
        // A pending call result is confirmed only by the builder's defining write, which
        // follows the `CallEffect` immediately. Any definition event that retires or replaces
        // the result place while the confirmation is still pending therefore cannot be builder
        // output, and consuming the entry here would leave the confirmation bound to a
        // generation the oracle no longer associates with the place. That is malformed
        // normalized input, not a role the transition may silently drop.
        if self.pending_call_results.contains_key(&destination) {
            return Err(CompilerError::compiler_error(format!(
                "Boracle oracle received a definition event for place {:?} whose pending call \
                 result has not been confirmed",
                destination
            )));
        }
        let current = self.state(destination)?;
        match current {
            RuntimePlaceState::Unavailable => {
                let installs_alias = matches!(
                    event_kind,
                    DefinitionEventKind::DirectAlias | DefinitionEventKind::MutableParameter
                );
                let (installed_state, target) = if installs_alias {
                    let DefinitionRole::Alias { target, access } = role else {
                        return Err(CompilerError::compiler_error(
                            "Boracle definition alias role did not provide an alias target",
                        ));
                    };
                    let installed_state = RuntimePlaceState::Alias {
                        target: target.node,
                        path: target.path.clone(),
                        access,
                    };
                    (installed_state, target)
                } else {
                    let target = role.slot_target();
                    (
                        RuntimePlaceState::Slot {
                            current: target.node,
                        },
                        target,
                    )
                };
                let retired_capabilities =
                    self.retire_overlapping_holders(problem, destination, event_index)?;
                self.set_state(destination, installed_state);
                Ok(DefinitionTransition::Installed {
                    target,
                    retired_capabilities,
                })
            }
            RuntimePlaceState::Slot { .. } => {
                let target = role.slot_target();
                let retired_capabilities =
                    self.retire_overlapping_holders(problem, destination, event_index)?;
                self.set_state(
                    destination,
                    RuntimePlaceState::Slot {
                        current: target.node,
                    },
                );
                Ok(DefinitionTransition::ReplacedSlot {
                    target,
                    retired_capabilities,
                })
            }
            RuntimePlaceState::Alias { target, path, .. } => {
                Ok(DefinitionTransition::WriteThroughAlias {
                    target: RuntimeAccessTarget { node: target, path },
                })
            }
        }
    }

    pub(crate) fn resolve_place(
        &mut self,
        problem: &BorrowProblem,
        place_index: &PlaceIndex,
        place_id: PlaceId,
        max_dynamic_generations: usize,
    ) -> Result<Result<Option<ResolvedPlace>, OracleLimitReason>, CompilerError> {
        let place = problem.places().get(place_id.index()).ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "Boracle oracle cannot locate place {:?}",
                place_id
            ))
        })?;
        if place.id != place_id {
            return Err(CompilerError::compiler_error(format!(
                "Boracle oracle place index {:?} names {:?}",
                place_id, place.id
            )));
        }

        for prefix_length in (0..=place.projections.len()).rev() {
            let prefix = &place.projections[..prefix_length];
            let Some(candidate) = place_index.lookup(place.root, prefix) else {
                continue;
            };
            let state = self.state(candidate)?;
            if matches!(state, RuntimePlaceState::Unavailable) {
                continue;
            }
            let suffix = place.projections[prefix_length..].to_vec();
            let mut target = match &state {
                RuntimePlaceState::Slot { current } => RuntimeAccessTarget {
                    node: *current,
                    path: suffix.into_boxed_slice(),
                },
                RuntimePlaceState::Alias {
                    target,
                    path,
                    access: _,
                } => {
                    let mut path = path.to_vec();
                    path.extend(suffix);
                    RuntimeAccessTarget {
                        node: *target,
                        path: path.into_boxed_slice(),
                    }
                }
                RuntimePlaceState::Unavailable => continue,
            };
            match self.descend(&mut target, max_dynamic_generations) {
                Ok(()) => {}
                Err(reason) => return Ok(Err(reason)),
            }
            return Ok(Ok(Some(ResolvedPlace {
                candidate,
                state,
                target,
            })));
        }
        Ok(Ok(None))
    }

    pub(crate) fn resolve_target(
        &mut self,
        mut target: RuntimeAccessTarget,
        max_dynamic_generations: usize,
    ) -> Result<Result<RuntimeAccessTarget, OracleLimitReason>, CompilerError> {
        match self.descend(&mut target, max_dynamic_generations) {
            Ok(()) => Ok(Ok(target)),
            Err(reason) => Ok(Err(reason)),
        }
    }

    fn descend(
        &mut self,
        target: &mut RuntimeAccessTarget,
        max_dynamic_generations: usize,
    ) -> Result<(), OracleLimitReason> {
        let mut consumed = 0;
        while consumed < target.path.len() {
            let element = target.path[consumed];
            let child = self
                .aggregates
                .get(&target.node)
                .and_then(|aggregate| aggregate.children.get(&element))
                .copied();
            if let Some(child) = child {
                target.node = child;
                consumed += 1;
                continue;
            }
            if !matches!(
                element,
                ProjectionElem::Field(_) | ProjectionElem::FixedIndex(_)
            ) {
                break;
            }
            let child = self.issue_generation(max_dynamic_generations)?;
            self.aggregates
                .entry(target.node)
                .or_default()
                .children
                .insert(element, child);
            target.node = child;
            consumed += 1;
        }
        if consumed != 0 {
            target.path = target.path[consumed..].to_vec().into_boxed_slice();
        }
        Ok(())
    }

    pub(crate) fn issue_capability(
        &mut self,
        kind: AccessKind,
        target: RuntimeAccessTarget,
        holders: BTreeSet<PlaceId>,
        event_index: usize,
        event_id: EventId,
        source: CapabilitySource,
    ) -> Result<RuntimeCapabilityId, CompilerError> {
        let capability_id = RuntimeCapabilityId(self.next_capability);
        self.next_capability = self.next_capability.checked_add(1).ok_or_else(|| {
            CompilerError::compiler_error("Boracle oracle capability identity overflow")
        })?;
        let capability = RuntimeCapability {
            kind,
            target: target.node,
            path: target.path,
            holders,
            issue_index: event_index,
            issue_event: event_id,
            last_exercised: event_index,
            explicit_end: None,
            end_reason: None,
            retired_holders: BTreeSet::new(),
            call_effect_index: None,
            source,
        };
        self.capabilities.insert(capability_id, capability);
        Ok(capability_id)
    }

    /// Ends a capability from the `LoanKill` event path.
    pub(crate) fn end_capability(
        &mut self,
        capability_id: RuntimeCapabilityId,
        event_index: usize,
    ) -> Result<(), CompilerError> {
        self.end_capability_with_reason(
            capability_id,
            event_index,
            CapabilityEndReason::LoanKill,
            BTreeSet::new(),
        )
    }

    fn end_capability_with_reason(
        &mut self,
        capability_id: RuntimeCapabilityId,
        event_index: usize,
        end_reason: CapabilityEndReason,
        retired_holders: BTreeSet<PlaceId>,
    ) -> Result<(), CompilerError> {
        let capability = self.capabilities.get_mut(&capability_id).ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "Boracle oracle cannot locate capability {:?}",
                capability_id
            ))
        })?;
        if capability.explicit_end.is_some() {
            return Err(CompilerError::compiler_error(format!(
                "Boracle oracle capability {:?} ended more than once",
                capability_id
            )));
        }
        capability.explicit_end = Some(event_index);
        capability.end_reason = Some(end_reason);
        capability.retired_holders = retired_holders;
        Ok(())
    }

    pub(crate) fn extend_call_capabilities(&mut self, call: CallId, event_index: usize) {
        for capability in self.capabilities.values_mut() {
            if !matches!(
                capability.source,
                CapabilitySource::CallArgument(source_call) if source_call == call
            ) || capability.call_effect_index.is_some()
            {
                continue;
            }
            // An explicit end truncates the interval, so the call's hold on its argument cannot
            // reach past it. The hold still closes here either way, because this invocation's
            // effect has now executed.
            if capability
                .explicit_end
                .is_none_or(|explicit_end| explicit_end >= event_index)
            {
                capability.last_exercised = capability.last_exercised.max(event_index);
            }
            capability.call_effect_index = Some(event_index);
        }
    }

    fn retire_holder(
        &mut self,
        holder: PlaceId,
        event_index: usize,
    ) -> Result<Box<[RuntimeCapabilityId]>, CompilerError> {
        self.end_capabilities_for_retired_holders(&BTreeSet::from([holder]), event_index)
    }

    /// Ends every capability held by a place that structurally overlaps `written`.
    ///
    /// The reference's `holder_kills` `Access` arm keys the kill on structural place overlap
    /// (`loans.rs:794-800`), not on holder identity, so a defining write that covers a
    /// projection-addressed holder ends it even though the capability is not held by the
    /// written place itself. A defining write with no paired provenance event runs through
    /// here, which is what keeps that kill alive when the writer never arrives.
    pub(crate) fn retire_overlapping_holders(
        &mut self,
        problem: &BorrowProblem,
        written: PlaceId,
        event_index: usize,
    ) -> Result<Box<[RuntimeCapabilityId]>, CompilerError> {
        let written_row = problem.places().get(written.index()).ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "Boracle oracle cannot locate written place {:?} for holder retirement",
                written
            ))
        })?;
        let mut overlapping = BTreeSet::new();
        for holder in self
            .capabilities
            .values()
            .flat_map(|cap| cap.holders.iter())
        {
            let holder_overlaps = problem
                .places()
                .get(holder.index())
                .is_some_and(|holder_row| {
                    holder_row.overlap(written_row) != PlaceOverlap::Disjoint
                });
            if holder_overlaps {
                overlapping.insert(*holder);
            }
        }
        if overlapping.is_empty() {
            return Ok(Box::new([]));
        }
        self.end_capabilities_for_retired_holders(&overlapping, event_index)
    }

    /// Ends the capabilities held by every place rooted at the listed bindings and marks those
    /// places `Unavailable`.
    ///
    /// A scope exit that reaches a pending call result must be rejected rather than silently
    /// dropping the entry: the builder emits the confirming definition write immediately after
    /// the `CallEffect`, so a scope exit that retires the result's binding first is malformed
    /// normalized input, and a dropped entry would leave the confirmation's exemptions
    /// bound to nothing.
    pub(crate) fn end_holders(
        &mut self,
        problem: &BorrowProblem,
        bindings: &[BindingId],
        event_index: usize,
    ) -> Result<Box<[RuntimeCapabilityId]>, CompilerError> {
        let binding_set = bindings.iter().copied().collect::<BTreeSet<_>>();
        let mut retired_places = BTreeSet::new();
        for place in self.pending_call_results.keys() {
            let pending_root = problem
                .places()
                .get(place.index())
                .map(|place_row| place_row.root)
                .ok_or_else(|| {
                    CompilerError::compiler_error(format!(
                        "Boracle oracle cannot locate a pending call result place {:?}",
                        place
                    ))
                })?;
            if binding_set.contains(&pending_root) {
                return Err(CompilerError::compiler_error(format!(
                    "Boracle oracle scope exit retires a pending call result for place {:?} \
                     before its confirming write",
                    place
                )));
            }
        }
        for place in problem.places() {
            if binding_set.contains(&place.root) {
                self.set_state(place.id, RuntimePlaceState::Unavailable);
                retired_places.insert(place.id);
            }
        }
        self.end_capabilities_for_retired_holders(&retired_places, event_index)
    }

    fn end_capabilities_for_retired_holders(
        &mut self,
        retired_places: &BTreeSet<PlaceId>,
        event_index: usize,
    ) -> Result<Box<[RuntimeCapabilityId]>, CompilerError> {
        let capability_ids = self.capabilities.keys().copied().collect::<Vec<_>>();
        let mut ended = Vec::new();
        for capability_id in capability_ids {
            let Some(capability) = self.capabilities.get(&capability_id) else {
                return Err(CompilerError::compiler_error(
                    "Boracle oracle lost a capability row during holder retirement",
                ));
            };
            let retired_holders = capability
                .holders
                .intersection(retired_places)
                .copied()
                .collect::<BTreeSet<_>>();
            if retired_holders.is_empty() {
                continue;
            }
            if capability.explicit_end.is_none() {
                self.end_capability_with_reason(
                    capability_id,
                    event_index,
                    CapabilityEndReason::HolderRetired,
                    retired_holders,
                )?;
                ended.push(capability_id);
            } else if capability.end_reason == Some(CapabilityEndReason::HolderRetired) {
                // A multi-holder capability can cross several holder retirements; retain every
                // static holder that retires while preserving the original interval endpoint.
                let capability = self.capabilities.get_mut(&capability_id).ok_or_else(|| {
                    CompilerError::compiler_error("Boracle oracle lost a capability row")
                })?;
                capability.retired_holders.extend(retired_holders);
            }
        }
        Ok(ended.into_boxed_slice())
    }

    pub(crate) fn active_capability_for_loan(&self, loan: LoanId) -> Option<RuntimeCapabilityId> {
        self.capabilities.iter().rev().find_map(|(id, capability)| {
            (capability.explicit_end.is_none()
                && matches!(
                    capability.source,
                    CapabilitySource::Loan(source_loan) if source_loan == loan
                ))
            .then_some(*id)
        })
    }
}
