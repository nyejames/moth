//! Deterministic execution traces for the operational oracle.
//!
//! WHAT: records every executed normalized event, its resolved access evidence and capability
//!       changes, then snapshots the completed capability intervals.
//! WHY: a runtime conflict must be replayable without consulting the static solver.

use super::state::{RuntimeAccessTarget, RuntimeCapability, RuntimeCapabilityId};
use crate::compiler_frontend::analysis::borrow_checker::problem::{
    AccessKind, BlockId, EventId, PlaceId, PointId,
};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TraceAccess {
    pub(crate) place: PlaceId,
    pub(crate) kind: AccessKind,
    pub(crate) target: RuntimeAccessTarget,
    pub(crate) definition: bool,
    pub(crate) exercised: Box<[RuntimeCapabilityId]>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TraceEntry {
    pub(crate) index: usize,
    pub(crate) event: EventId,
    pub(crate) point: PointId,
    pub(crate) block: BlockId,
    pub(crate) access: Option<TraceAccess>,
    pub(crate) issued_capabilities: Box<[RuntimeCapabilityId]>,
    pub(crate) ended_capabilities: Box<[RuntimeCapabilityId]>,
}

impl TraceEntry {
    fn new(index: usize, event: EventId, point: PointId, block: BlockId) -> Self {
        Self {
            index,
            event,
            point,
            block,
            access: None,
            issued_capabilities: Box::new([]),
            ended_capabilities: Box::new([]),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExecutionTrace {
    pub(crate) entries: Box<[TraceEntry]>,
    pub(crate) capabilities: Box<[RuntimeCapability]>,
    pub(crate) block_entries: BTreeMap<BlockId, usize>,
    pub(crate) conflict: Option<RuntimeConflictWitness>,
}

impl ExecutionTrace {
    pub(crate) fn debug_dump(&self) -> String {
        format!("{self:#?}")
    }

    pub(crate) fn entries(&self) -> &[TraceEntry] {
        &self.entries
    }

    pub(crate) fn capabilities(&self) -> &[RuntimeCapability] {
        &self.capabilities
    }

    pub(crate) fn block_entries(&self) -> &BTreeMap<BlockId, usize> {
        &self.block_entries
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RuntimeConflictWitness {
    pub(crate) access_event: EventId,
    pub(crate) access_index: usize,
    // Neither field is optional: a conflict is only ever reported with the capability the access
    // actually exercised, so a conflict whose witness names no live capability is unrepresentable.
    pub(crate) capability_id: RuntimeCapabilityId,
    pub(crate) capability_issue: usize,
    pub(crate) access_kind: AccessKind,
    pub(crate) capability_kind: AccessKind,
    pub(crate) access_target: RuntimeAccessTarget,
    pub(crate) capability_target: RuntimeAccessTarget,
}

#[derive(Debug, Clone)]
pub(crate) struct TraceBuilder {
    entries: Vec<TraceEntry>,
}

impl TraceBuilder {
    pub(crate) fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    pub(crate) fn begin(&mut self, event: EventId, point: PointId, block: BlockId) -> usize {
        let index = self.entries.len();
        self.entries
            .push(TraceEntry::new(index, event, point, block));
        index
    }

    pub(crate) fn record_access(&mut self, index: usize, access: TraceAccess) {
        if let Some(entry) = self.entries.get_mut(index) {
            entry.access = Some(access);
        }
    }

    pub(crate) fn record_issue(&mut self, index: usize, capability: RuntimeCapabilityId) {
        if let Some(entry) = self.entries.get_mut(index) {
            let mut capabilities = entry.issued_capabilities.to_vec();
            capabilities.push(capability);
            entry.issued_capabilities = capabilities.into_boxed_slice();
        }
    }

    pub(crate) fn record_exercise(&mut self, index: usize, capability: RuntimeCapabilityId) {
        if let Some(entry) = self.entries.get_mut(index)
            && let Some(access) = entry.access.as_mut()
        {
            let mut exercised = access.exercised.to_vec();
            if !exercised.contains(&capability) {
                exercised.push(capability);
                exercised.sort_unstable();
                access.exercised = exercised.into_boxed_slice();
            }
        }
    }

    pub(crate) fn record_end(&mut self, index: usize, capability: RuntimeCapabilityId) {
        if let Some(entry) = self.entries.get_mut(index) {
            let mut capabilities = entry.ended_capabilities.to_vec();
            capabilities.push(capability);
            capabilities.sort_unstable();
            entry.ended_capabilities = capabilities.into_boxed_slice();
        }
    }

    pub(crate) fn entries(&self) -> &[TraceEntry] {
        &self.entries
    }

    pub(crate) fn finish(
        self,
        capabilities: &BTreeMap<RuntimeCapabilityId, RuntimeCapability>,
        block_entries: BTreeMap<BlockId, usize>,
        conflict: Option<RuntimeConflictWitness>,
    ) -> ExecutionTrace {
        let capabilities = capabilities.values().cloned().collect::<Vec<_>>();
        ExecutionTrace {
            entries: self.entries.into_boxed_slice(),
            capabilities: capabilities.into_boxed_slice(),
            block_entries,
            conflict,
        }
    }
}
