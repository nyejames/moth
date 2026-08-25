//! Immutable normalized CFG rows and ordered event membership.

use super::events::EventSource;
use super::ids::{BlockId, EventId, PointId};

/// One semantic program point inside a normalized CFG block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProgramPoint {
    pub(crate) id: PointId,
    pub(crate) block: BlockId,
    pub(crate) ordinal: u32,
    pub(crate) source: EventSource,
}

impl ProgramPoint {
    pub(crate) const fn new(id: PointId, block: BlockId, ordinal: u32) -> Self {
        Self {
            id,
            block,
            ordinal,
            source: EventSource::none(),
        }
    }

    pub(crate) fn with_source(
        id: PointId,
        block: BlockId,
        ordinal: u32,
        source: EventSource,
    ) -> Self {
        Self {
            id,
            block,
            ordinal,
            source,
        }
    }
}

/// One CFG block with an explicit ordered event stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CfgBlock {
    pub(crate) id: BlockId,
    pub(crate) entry: PointId,
    pub(crate) exit: PointId,
    pub(crate) events: Box<[EventId]>,
}

impl CfgBlock {
    pub(crate) fn new(id: BlockId, entry: PointId, exit: PointId, events: Vec<EventId>) -> Self {
        Self {
            id,
            entry,
            exit,
            events: events.into_boxed_slice(),
        }
    }
}

/// One directed CFG edge between normalized blocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CfgEdge {
    pub(crate) from: BlockId,
    pub(crate) to: BlockId,
}

impl CfgEdge {
    pub(crate) const fn new(from: BlockId, to: BlockId) -> Self {
        Self { from, to }
    }
}

/// The immutable block graph and its entry/exit boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ControlFlow {
    pub(crate) blocks: Box<[CfgBlock]>,
    pub(crate) edges: Box<[CfgEdge]>,
    pub(crate) entry: BlockId,
    pub(crate) exits: Box<[BlockId]>,
}

impl ControlFlow {
    pub(crate) fn new(
        blocks: Vec<CfgBlock>,
        edges: Vec<CfgEdge>,
        entry: BlockId,
        exits: Vec<BlockId>,
    ) -> Self {
        Self {
            blocks: blocks.into_boxed_slice(),
            edges: edges.into_boxed_slice(),
            entry,
            exits: exits.into_boxed_slice(),
        }
    }
}
