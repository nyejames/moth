//! Bounded operational oracle for one normalised borrow problem and its concrete executions.
//!
//! WHAT: exposes the oracle outcome and coordinates concrete state, event execution, call
//!       handling, dynamic conflicts and deterministic traces.
//! WHY: this is a second semantics owned by Boracle. It never reuses the static origin, loan or
//!      overlap solvers, and it is kept separate from the reference-solver flow.
//!
//! Files:
//! - `state.rs`: dynamic generations, place state, aggregate edges and capabilities
//! - `execute.rs`: single-path forward event and block execution
//! - `paths.rs`: deterministic bounded path enumeration and outcome selection
//! - `calls.rs`: granular call arguments and call-result provenance
//! - `conflicts.rs`: dynamic overlap, holder coverage and interval decisions
//! - `traces.rs`: the replayable execution trace and its conflict witness
//! - `generator.rs`: bounded deterministic normalized-problem generation
//! - `tests/`: operational invariants
//!
//! The path layer owns the frontier and all bounds that span executions. The executor only
//! advances one concrete path through one block at a time.
#![allow(dead_code)]

mod calls;
mod conflicts;
mod execute;
pub(crate) mod generator;
mod paths;
mod state;
mod traces;

#[cfg(test)]
mod tests;

use crate::compiler_frontend::analysis::borrow_checker::problem::{
    BlockId, CallId, CallResultUnknownReason, LoanId, PlaceId, ProjectionElem, ValueOriginId,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OracleBounds {
    /// Maximum number of complete or truncated executions enumerated.
    pub(crate) max_executions: usize,
    /// Maximum number of events executed by each path.
    pub(crate) max_executed_events: usize,
    /// Maximum entries of one block on each path.
    pub(crate) max_block_entries: usize,
    /// Maximum dynamic generations created by each path.
    pub(crate) max_dynamic_generations: usize,
}

impl Default for OracleBounds {
    fn default() -> Self {
        Self {
            max_executions: 256,
            max_executed_events: 4096,
            max_block_entries: 8,
            max_dynamic_generations: 4096,
        }
    }
}

impl OracleBounds {
    pub(crate) const fn new(
        max_executions: usize,
        max_executed_events: usize,
        max_block_entries: usize,
        max_dynamic_generations: usize,
    ) -> Self {
        Self {
            max_executions,
            max_executed_events,
            max_block_entries,
            max_dynamic_generations,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum OracleLimitReason {
    CallAliasParams {
        call: CallId,
        alternative_count: usize,
    },
    RebindAliasOrigins {
        origins: Box<[ValueOriginId]>,
    },
    CallResultAliasOrigins {
        call: CallId,
        origins: Box<[ValueOriginId]>,
    },
    CallResultUnknown {
        call: CallId,
        reason: CallResultUnknownReason,
    },
    UndecidableOverlap {
        left: state::RuntimeAccessTarget,
        right: state::RuntimeAccessTarget,
    },
    ExecutionBound {
        limit: usize,
    },
    EventBound {
        limit: usize,
    },
    BlockEntryBound {
        block: BlockId,
        limit: usize,
    },
    GenerationBound {
        limit: usize,
    },
    NonTerminatingCycle {
        block: BlockId,
    },
    /// A repeated aggregate projection resolves to two distinct nodes. One child position cannot
    /// hold two distinct nodes, and the runtime children map stores one node per projection, so
    /// either repeat would silently detach the forgotten child. The reference still gives the
    /// shape semantics by extending the projected place's alternatives with every repeated
    /// field's origins (`origins.rs:1308-1317`), and the runtime graph cannot represent that
    /// union, whether the projection names a keyed slot or a keyless storage domain.
    RepeatedProjectionChild {
        destination: PlaceId,
        projection: ProjectionElem,
        surviving: state::DynamicOriginId,
        forgotten: state::DynamicOriginId,
    },
    /// A loan row names several distinct holders. No reference semantics defines per-holder
    /// retirement, so the oracle cannot report what the surviving holders do and do not cover.
    /// The count is over distinct places: validation does not require holder uniqueness and a
    /// repeated place collapses to one holder.
    MultiHolderLoan {
        loan: LoanId,
        holders: usize,
    },
}

/// The bounded enumeration result over every concrete path of one normalized problem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum OracleOutcome {
    /// No path truncated and none conflicted. `trace` retains the deterministic first complete
    /// conflict-free execution, finished with its final capability and block-entry snapshots and
    /// without a conflict witness; later safe paths are never retained.
    CompleteSafe {
        executions: usize,
        trace: traces::ExecutionTrace,
    },
    RuntimeConflict {
        trace: traces::ExecutionTrace,
    },
    /// At least one path truncated. `completed_executions` counts the complete conflict-free
    /// executions observed by the enumeration before or alongside that truncation.
    Inconclusive {
        reason: OracleLimitReason,
        explored: usize,
        completed_executions: usize,
    },
}

pub(crate) use paths::execute_bounded;
pub(crate) use traces::ExecutionTrace;
