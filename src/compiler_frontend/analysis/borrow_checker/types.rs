//! Borrow-checker snapshots, facts, and summary data structures.
//!
//! WHAT: defines the immutable analysis records produced while validating HIR borrows.
//! WHY: transfer and diagnostics need a shared vocabulary for states, facts, and summaries.

use crate::compiler_frontend::compiler_errors::SourceLocation;
use crate::compiler_frontend::external_packages::CallTarget;
use crate::compiler_frontend::hir::expressions::HirMapOp;
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId, HirNodeId, HirValueId, LocalId};
use crate::compiler_frontend::hir::reactivity::ReactiveSourceId;
use crate::compiler_frontend::public_call_summary::PublicCallSummary;
use rustc_hash::FxHashMap;

#[derive(Debug, Clone, Default)]
pub(crate) struct BorrowCheckReport {
    pub analysis: BorrowAnalysis,
    pub stats: BorrowCheckStats,
}

impl BorrowCheckReport {
    pub(crate) fn borrow_facts(&self) -> &BorrowAnalysis {
        &self.analysis
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct BorrowAnalysis {
    /// Complete local function call contracts retained for semantic consumers.
    ///
    /// WHAT: stores parameter access, mutation, optional transfer, reactive effects, and
    /// return-alias facts in one local-function summary.
    /// WHY: call transfer and the public-interface draft consume the same frontend-owned semantic
    /// contract instead of reconstructing it from separate HIR metadata caches.
    pub public_call_summaries: FxHashMap<FunctionId, PublicCallSummary>,
    pub function_summaries: FxHashMap<FunctionId, FunctionBorrowSummary>,
    pub block_entry_states: FxHashMap<BlockId, BorrowStateSnapshot>,
    pub block_exit_states: FxHashMap<BlockId, BorrowStateSnapshot>,
    pub statement_entry_states: FxHashMap<HirNodeId, BorrowStateSnapshot>,
    pub statement_facts: FxHashMap<HirNodeId, StatementBorrowFact>,
    pub terminator_facts: FxHashMap<BlockId, TerminatorBorrowFact>,
    pub value_facts: FxHashMap<HirValueId, ValueBorrowFact>,
    /// Conservative source-level invalidation facts for reactive sources.
    ///
    /// WHY: backend lowering needs to know which statements may dirty a stable reactive source,
    /// while borrow validation must keep those subscriptions out of the active borrow state.
    pub reactive_invalidations: FxHashMap<HirNodeId, Vec<ReactiveInvalidationFact>>,
    /// Advisory drop insertion points for later lowering stages.
    ///
    /// WHY: borrow checking must not mutate HIR, but lowering still needs
    /// deterministic drop-site guidance for ownership-aware optimizations.
    pub advisory_drop_sites: FxHashMap<BlockId, Vec<BorrowDropSite>>,
}

impl BorrowAnalysis {
    #[cfg(any(test, feature = "show_borrow_checker"))]
    pub(crate) fn total_state_snapshots(&self) -> usize {
        self.block_entry_states.len()
            + self.block_exit_states.len()
            + self.statement_entry_states.len()
    }

    #[cfg(test)]
    pub(crate) fn statement_fact(&self, id: HirNodeId) -> Option<&StatementBorrowFact> {
        self.statement_facts.get(&id)
    }

    #[cfg(test)]
    pub(crate) fn terminator_fact(&self, block: BlockId) -> Option<&TerminatorBorrowFact> {
        self.terminator_facts.get(&block)
    }

    #[cfg(test)]
    pub(crate) fn value_fact(&self, id: HirValueId) -> Option<&ValueBorrowFact> {
        self.value_facts.get(&id)
    }

    #[cfg(test)]
    pub(crate) fn reactive_invalidations_for_statement(
        &self,
        id: HirNodeId,
    ) -> Option<&[ReactiveInvalidationFact]> {
        self.reactive_invalidations.get(&id).map(Vec::as_slice)
    }

    pub(crate) fn drop_sites_for_block(&self, block: BlockId) -> Option<&[BorrowDropSite]> {
        // Exposed as a read-only view so downstream phases cannot mutate facts.
        self.advisory_drop_sites.get(&block).map(Vec::as_slice)
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct BorrowCheckStats {
    pub functions_analyzed: usize,
    pub blocks_analyzed: usize,
    pub statements_analyzed: usize,
    pub terminators_analyzed: usize,
    pub worklist_iterations: usize,
    pub state_joins: usize,
    pub conflicts_checked: usize,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct FunctionBorrowSummary {
    pub reachable_blocks: usize,
    pub mutable_call_sites: usize,
    pub alias_heavy_blocks: Vec<BlockId>,
    pub worklist_iterations: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct StatementBorrowFact {
    pub shared_roots: Vec<LocalId>,
    pub mutable_roots: Vec<LocalId>,
    pub conflicts_checked: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct TerminatorBorrowFact {
    pub shared_roots: Vec<LocalId>,
    pub mutable_roots: Vec<LocalId>,
    pub conflicts_checked: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ValueBorrowFact {
    pub classification: ValueAccessClassification,
    pub roots: Vec<LocalId>,
    /// Advisory ownership optimisation outcome for this value access.
    ///
    /// This fact never changes the mandatory borrow state. A `Borrow` outcome means that the
    /// checker could not prove a transfer on every relevant path, while `Transfer` records a
    /// proven optional destruction-responsibility handoff for a later lowering stage.
    pub optional_transfer: OptionalTransferStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum OptionalTransferStatus {
    #[default]
    NotAttempted,
    Borrow,
    Transfer,
}

impl OptionalTransferStatus {
    pub(crate) fn merge(self, other: Self) -> Self {
        match (self, other) {
            (Self::NotAttempted, status) | (status, Self::NotAttempted) => status,
            (Self::Borrow, Self::Borrow) => Self::Borrow,
            (Self::Transfer, Self::Transfer) => Self::Transfer,
            // A conservative fallback wins if one traversal cannot prove the transfer.
            (Self::Borrow, Self::Transfer) | (Self::Transfer, Self::Borrow) => Self::Borrow,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReactiveInvalidationFact {
    pub statement_id: HirNodeId,
    pub source: ReactiveSourceId,
    pub kind: ReactiveInvalidationKind,
    pub location: SourceLocation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ReactiveInvalidationKind {
    Assignment,
    PlaceWrite(ReactivePlaceWriteKind),
    MapMutation(HirMapOp),
    MutableCallArgument {
        target: Box<CallTarget>,
        argument_index: usize,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReactivePlaceWriteKind {
    Field,
    Index,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum ValueAccessClassification {
    #[default]
    None,
    SharedRead,
    MutableArgument,
    Mixed,
}

impl ValueAccessClassification {
    pub(crate) fn merge(self, other: Self) -> Self {
        match (self, other) {
            (Self::None, rhs) => rhs,
            (lhs, Self::None) => lhs,
            (Self::SharedRead, Self::SharedRead) => Self::SharedRead,
            (Self::MutableArgument, Self::MutableArgument) => Self::MutableArgument,
            _ => Self::Mixed,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BorrowStateSnapshot {
    pub locals: Vec<LocalBorrowSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LocalBorrowSnapshot {
    pub local: LocalId,
    pub mode: LocalMode,
    pub alias_roots: Vec<LocalId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct LocalMode(u8);

impl LocalMode {
    pub(crate) const UNINIT: Self = Self(0b001);
    pub(crate) const SLOT: Self = Self(0b010);
    pub(crate) const ALIAS: Self = Self(0b100);

    pub(crate) fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }

    pub(crate) fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub(crate) fn is_definitely_uninit(self) -> bool {
        self.0 == Self::UNINIT.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AccessKind {
    Shared,
    Mutable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum BorrowDropSiteKind {
    /// Edge leaves current lexical region scope.
    BlockExit,
    /// Function return path.
    Return,
    /// Loop break path.
    Break,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BorrowDropSite {
    /// Control-flow reason this site exists.
    pub kind: BorrowDropSiteKind,
    /// Candidate locals sorted by local id for deterministic lowering.
    pub locals: Vec<LocalId>,
}
