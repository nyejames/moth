//! Borrow-checker transfer driver.
//!
//! WHAT: coordinates one-block transfer for the fixed-point borrow analysis.
//! Statement/terminator rules live in submodules so external call semantics,
//! access validation, and fact buffering stay inspectable.
//! WHY: transfer owns borrow-rule effects and side-table fact emission; it must
//! not mutate HIR or perform backend ownership lowering.

pub(super) mod access;
pub(super) mod call_semantics;
pub(super) mod facts;

use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckError;
use crate::compiler_frontend::analysis::borrow_checker::diagnostics::BorrowDiagnostics;
use crate::compiler_frontend::analysis::borrow_checker::state::{
    AllocId, BorrowState, CapturedValue, FieldKey, FunctionLayout, LocalState, RootSet,
    StoredValue, TypedStore,
};
#[cfg(any(test, feature = "show_borrow_checker"))]
use crate::compiler_frontend::analysis::borrow_checker::types::BorrowStateSnapshot;
use crate::compiler_frontend::analysis::borrow_checker::types::{
    StatementBorrowFact, TerminatorBorrowFact, ValueBorrowFact,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::hir::blocks::HirBlock;
use crate::compiler_frontend::hir::expression_store::HirExpressionStore;
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId, HirNodeId, HirValueId, LocalId};
use crate::compiler_frontend::hir::module::HirChoice;
use crate::compiler_frontend::hir::structs::HirStruct;
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::public_call_summary::PublicCallSummary;
use crate::compiler_frontend::public_call_summary::typed::SupportId;
use crate::compiler_frontend::semantic_identity::{
    GeneratedFunctionIdentity, ModulePrivateExecutableIdentity, OriginFunctionId,
};
use rustc_hash::{FxHashMap, FxHashSet};
use std::borrow::Cow;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::ops::{Deref, DerefMut};
use std::rc::Rc;

pub(super) use access::{call_member_result, call_member_writes};

pub(super) use access::typed_call;
use access::{
    AggregateTransferContext, CallTransferContext, transfer_aggregate_expression_ownership,
    transfer_statement, transfer_statement_operation, transfer_terminator,
    transfer_terminator_operation,
};
use facts::{
    Demand, ObservationCheckpoint, StatementAccessTracker, ValueFactBuffer, ValueObservations,
};

#[cfg(test)]
#[path = "transfer/tests/value_fact_buffer_tests.rs"]
mod value_fact_buffer_tests;

/// Ordered scratch that needs no allocation for zero or one item.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub(super) enum InlineVec<T> {
    #[default]
    Empty,
    One(T),
    Many(Vec<T>),
}

impl<T> InlineVec<T> {
    pub(super) fn push(&mut self, value: T) {
        match self {
            Self::Empty => *self = Self::One(value),
            Self::Many(values) => values.push(value),
            Self::One(_) => {
                let Self::One(first) = std::mem::take(self) else {
                    unreachable!()
                };
                *self = Self::Many(vec![first, value]);
            }
        }
    }

    pub(super) fn pop(&mut self) -> Option<T> {
        match self {
            Self::Many(values) => values.pop(),
            _ => match std::mem::take(self) {
                Self::One(value) => Some(value),
                _ => None,
            },
        }
    }
}

impl<T> Deref for InlineVec<T> {
    type Target = [T];

    fn deref(&self) -> &[T] {
        match self {
            Self::Empty => &[],
            Self::One(value) => std::slice::from_ref(value),
            Self::Many(values) => values,
        }
    }
}

impl<T> DerefMut for InlineVec<T> {
    fn deref_mut(&mut self) -> &mut [T] {
        match self {
            Self::Empty => &mut [],
            Self::One(value) => std::slice::from_mut(value),
            Self::Many(values) => values,
        }
    }
}

impl<T> FromIterator<T> for InlineVec<T> {
    fn from_iter<I: IntoIterator<Item = T>>(values: I) -> Self {
        let mut values = values.into_iter();
        match (values.next(), values.next()) {
            (None, _) => Self::Empty,
            (Some(first), None) => Self::One(first),
            (Some(first), Some(second)) => {
                Self::Many([first, second].into_iter().chain(values).collect())
            }
        }
    }
}

impl<T> IntoIterator for InlineVec<T> {
    type Item = T;
    type IntoIter = std::iter::Chain<std::option::IntoIter<T>, std::vec::IntoIter<T>>;

    fn into_iter(self) -> Self::IntoIter {
        let (one, many) = match self {
            Self::Empty => (None, Vec::new()),
            Self::One(value) => (Some(value), Vec::new()),
            Self::Many(values) => (None, values),
        };
        one.into_iter().chain(many)
    }
}

/// Pending replay inputs retain first-arrival order. A singleton does not
/// need hashing, and only blocks that may revisit keep processed keys alive.
#[derive(Default)]
pub(super) struct ReplayStates {
    pending: InlineVec<Rc<BorrowState>>,
    seen: FxHashSet<Rc<BorrowState>>,
}

impl ReplayStates {
    pub(super) fn insert(&mut self, state: Rc<BorrowState>, revisits: bool) -> bool {
        if !revisits && self.seen.is_empty() {
            if let InlineVec::One(first) = &self.pending {
                if first == &state {
                    return false;
                }
                self.seen.insert(Rc::clone(first));
            } else {
                self.pending.push(state);
                return true;
            }
        }
        if !self.seen.insert(Rc::clone(&state)) {
            return false;
        }
        self.pending.push(state);
        true
    }

    pub(super) fn take_pending(&mut self) -> InlineVec<Rc<BorrowState>> {
        std::mem::take(&mut self.pending)
    }

    pub(super) fn into_pending(self) -> InlineVec<Rc<BorrowState>> {
        self.pending
    }
}

impl IntoIterator for ReplayStates {
    type Item = Rc<BorrowState>;
    type IntoIter = <InlineVec<Self::Item> as IntoIterator>::IntoIter;

    fn into_iter(self) -> Self::IntoIter {
        self.into_pending().into_iter()
    }
}

#[cfg(feature = "detailed_timers")]
#[derive(Clone, Copy)]
pub(crate) enum BorrowTimingOwner {
    BaseConverge,
    GeneratedRecheck,
}

#[cfg(feature = "detailed_timers")]
#[derive(Clone, Copy)]
pub(crate) struct BorrowTiming {
    pub(crate) owner: BorrowTimingOwner,
    pub(crate) context: Option<crate::timing::TimingContext>,
}

#[cfg(feature = "detailed_timers")]
impl BorrowTiming {
    pub(crate) fn start(
        self,
        base: crate::timing::TimingMetric,
        generated: crate::timing::TimingMetric,
    ) -> crate::timing::PipelineTimingGuardAttributed {
        let metric = match self.owner {
            BorrowTimingOwner::BaseConverge => base,
            BorrowTimingOwner::GeneratedRecheck => generated,
        };
        crate::timing::PipelineTimingGuardAttributed::new(metric, || self.context)
    }
}

pub(super) struct BorrowTransferContext<'a> {
    // WHAT: shared lookup/diagnostic tables for one function transfer pass.
    // WHY: avoids repeated module scans while statements/terminators are analyzed.
    pub external_package_registry: &'a ExternalPackageRegistry,
    pub public_call_summaries: &'a FxHashMap<FunctionId, PublicCallSummary>,
    pub imported_call_summaries: &'a FxHashMap<OriginFunctionId, PublicCallSummary>,
    pub module_private_call_summaries:
        &'a FxHashMap<ModulePrivateExecutableIdentity, PublicCallSummary>,
    pub generated_call_summaries: &'a FxHashMap<GeneratedFunctionIdentity, PublicCallSummary>,
    pub local_types: &'a FxHashMap<LocalId, TypeId>,
    pub structs: &'a [HirStruct],
    pub choices: &'a [HirChoice],
    pub expressions: &'a HirExpressionStore,
    pub type_environment: &'a TypeEnvironment,
    pub type_projection:
        crate::compiler_frontend::canonical_type_identity::CanonicalTypeProjectionContext<'a>,
    pub diagnostics: BorrowDiagnostics<'a>,
    pub typed_call_cache: typed_call::TypedCallCache,
    #[cfg(feature = "detailed_timers")]
    pub timing: Option<BorrowTiming>,
}

/// The storage queries use local indices, not validation's dense liveness tables.
#[derive(Clone, Copy)]
pub(super) struct OperationView<'a> {
    pub local_ids: &'a [LocalId],
    pub local_indices: &'a FxHashMap<LocalId, usize>,
    pub local_mutable: &'a [bool],
    pub local_has_in_place_mutation_surface: &'a [bool],
    pub block_successors: &'a FxHashMap<BlockId, Vec<BlockId>>,
    pub local_first_write_order: &'a [i32],
    pub acyclic_entry_block: Option<BlockId>,
    pub entry_roots: Option<&'a RootSet>,
}

impl<'a> OperationView<'a> {
    pub(super) fn from_layout(layout: &'a FunctionLayout) -> Self {
        Self {
            local_ids: &layout.local_ids,
            local_indices: &layout.local_index_by_id,
            local_mutable: &layout.local_mutable,
            local_has_in_place_mutation_surface: &layout.local_has_in_place_mutation_surface,
            block_successors: &layout.block_successors,
            local_first_write_order: &layout.local_first_write_order,
            acyclic_entry_block: layout.acyclic_entry_block,
            entry_roots: None,
        }
    }

    pub(super) fn index_of(self, local: LocalId) -> Option<usize> {
        self.local_indices.get(&local).copied()
    }

    pub(super) fn local_count(self) -> usize {
        self.local_ids.len()
    }
}

/// Captures and retirement use the same coordinate queries even when no diagnostic sink exists.
#[derive(Clone, Copy)]
pub(super) struct EdgeTransferView<'a> {
    pub operations: OperationView<'a>,
}

impl<'a> From<&'a FunctionLayout> for EdgeTransferView<'a> {
    fn from(layout: &'a FunctionLayout) -> Self {
        Self {
            operations: OperationView::from_layout(layout),
        }
    }
}

/// Per-block orders are packed alongside the HIR operations, not across every local.
pub(super) struct SparseBlockView<'a> {
    pub operations: OperationView<'a>,
    pub statement_orders: &'a [i32],
    pub statement_positions: &'a [usize],
    pub terminator_order: i32,
    pub transfer_terminator: bool,
    pub structural_calls: &'a [(usize, usize)],
    pub structural_terminator: Option<usize>,
    pub call_components: Option<&'a BTreeMap<usize, Vec<SupportId>>>,
}

pub(super) fn transfer_sparse_block(
    context: &BorrowTransferContext<'_>,
    view: SparseBlockView<'_>,
    block: &HirBlock,
    state: Rc<BorrowState>,
    retained: &mut BTreeSet<usize>,
) -> Result<InlineVec<(Rc<BorrowState>, InvokeTransfer)>, BorrowCheckError> {
    assert_eq!(block.statements.len(), view.statement_orders.len());
    let mut states = InlineVec::One(state);
    let mut structural = view.structural_calls.iter().peekable();
    for &position in view.statement_positions {
        while let Some(&&(at, argc)) = structural.peek() {
            if at >= position {
                if at == position {
                    structural.next();
                }
                break;
            }
            structural.next();
            let target = crate::compiler_frontend::hir::reachability::statement_call_target(
                &block.statements[at].kind,
            )
            .expect("indexed statement call");
            if !sparse_structural_call(context, target, argc, false)? {
                return Ok(InlineVec::Empty);
            }
        }
        let statement = &block.statements[position];
        let order = view.statement_orders[position];
        let call_components = view
            .call_components
            .and_then(|calls| calls.get(&position))
            .map(Vec::as_slice);
        let mut next = ReplayStates::default();
        for mut state in states {
            let mut stats = BlockTransferStats {
                sparse: true,
                ..Default::default()
            };
            let mut transfer = CallTransferContext::new(
                context,
                view.operations,
                Rc::make_mut(&mut state),
                block.id,
                order,
                context.diagnostics.statement_error_span(statement),
                &mut stats,
                None,
            );
            transfer.call_components = call_components;
            transfer_statement_operation(&mut transfer, &statement.kind)?;
            retained.append(&mut stats.executed_retention);
            if !stats.diverges && !stats.deferred {
                if let Some(batch) = stats.sparse_call {
                    super::engine::replay_captured_sparse_support(
                        EdgeTransferView {
                            operations: view.operations,
                        },
                        Cow::Owned(batch),
                        &[],
                        order,
                        state,
                        |branch| {
                            next.insert(branch, false);
                        },
                        #[cfg(feature = "detailed_timers")]
                        context.timing,
                    )?;
                } else {
                    next.insert(state, false);
                }
            }
        }
        states = next.into_pending();
        if states.is_empty() {
            break;
        }
    }
    if !states.is_empty() {
        for &(at, argc) in structural {
            let target = crate::compiler_frontend::hir::reachability::statement_call_target(
                &block.statements[at].kind,
            )
            .expect("indexed statement call");
            if !sparse_structural_call(context, target, argc, false)? {
                return Ok(InlineVec::Empty);
            }
        }
    }
    let mut exits = InlineVec::Empty;
    let call_components = view
        .call_components
        .and_then(|calls| calls.get(&view.statement_orders.len()))
        .map(Vec::as_slice);
    for mut state in states {
        let mut stats = BlockTransferStats {
            sparse: true,
            ..Default::default()
        };
        if view.transfer_terminator {
            let mut transfer = CallTransferContext::new(
                context,
                view.operations,
                Rc::make_mut(&mut state),
                block.id,
                view.terminator_order,
                context.diagnostics.terminator_error_span(block.id),
                &mut stats,
                None,
            );
            transfer.call_components = call_components;
            transfer_terminator_operation(&mut transfer, context, &block.terminator)?;
            retained.append(&mut stats.executed_retention);
        } else if let Some(argc) = view.structural_terminator {
            let target = crate::compiler_frontend::hir::reachability::terminator_call_target(
                &block.terminator,
            )
            .expect("indexed terminal call");
            if matches!(block.terminator, HirTerminator::NeverCall { .. }) {
                continue;
            }
            stats.invoke.success.unreachable =
                !sparse_structural_call(context, target, argc, false)?;
            stats.invoke.error.unreachable = !sparse_structural_call(context, target, argc, true)?;
        }
        if !stats.diverges && !stats.deferred {
            exits.push((state, stats.invoke));
        }
    }
    Ok(exits)
}

/// Non-observing supports still inherit the callable's structural control edge.
fn sparse_structural_call(
    context: &BorrowTransferContext<'_>,
    target: &crate::compiler_frontend::external_packages::CallTarget,
    argc: usize,
    error: bool,
) -> Result<bool, BorrowCheckError> {
    use call_semantics::{
        CallReturns, TypedContinuation, classify_typed_continuation, resolve_call_semantics,
    };
    match resolve_call_semantics(context, target, argc)?.returns() {
        CallReturns::Summary(summary) => Ok(matches!(
            classify_typed_continuation(
                summary.facts(),
                false,
                error,
                false,
                &context.diagnostics,
            )?,
            TypedContinuation::Match(_)
        )),
        _ => Ok(true),
    }
}

/// Only mandatory validation needs a borrow verdict, liveness or value facts.
pub(in crate::compiler_frontend::analysis::borrow_checker::transfer) struct ValidationSink<
    'a,
    'tracker,
    'facts,
> {
    pub layout: &'a FunctionLayout,
    pub tracker: &'tracker mut StatementAccessTracker,
    pub value_facts: &'facts mut ValueFactBuffer,
}

/// An allocation-layer effect captured before retiring a call's result bindings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum InvokeLayerOp {
    Store(TypedStore),
    Contribute {
        holder: AllocId,
        key: FieldKey,
        value: StoredValue,
    },
    Bind {
        local: LocalId,
        value: CapturedValue,
    },
}

#[derive(Debug, Clone, Copy)]
pub(super) enum InvokeLane {
    Success,
    Error,
}

/// Evaluation fixes the address of an update; a definition instead creates
/// a new slot whose incoming value can still name predecessor storage.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum CapturedDestination {
    Define(usize),
    Update {
        actor: usize,
        address: super::state::AddressSet,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CapturedDetachment {
    pub holders: Vec<AllocId>,
    pub map: bool,
    /// The result ordinal, not the actor spelling of the receiving place.
    pub result: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CapturedCallBatch<Post = typed_call::CapturedPost> {
    pub post: Post,
    pub destinations: InlineVec<CapturedDestination>,
    pub member_reads: Demand,
    pub checkpoints: Vec<ObservationCheckpoint>,
    pub detachment: Option<CapturedDetachment>,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct InvokeLaneTransfer {
    pub typed: Option<CapturedCallBatch<typed_call::MatchedLane>>,
    pub results: Vec<(LocalId, LocalState)>,
    pub operations: Vec<InvokeLayerOp>,
    pub member_reads: Demand,
    pub checkpoints: Vec<ObservationCheckpoint>,
    pub unreachable: bool,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct InvokeTransfer {
    pub success: InvokeLaneTransfer,
    pub error: InvokeLaneTransfer,
}

impl InvokeTransfer {
    pub(super) fn is_empty(&self) -> bool {
        [&self.success, &self.error].into_iter().all(|lane| {
            lane.results.is_empty()
                && lane.typed.is_none()
                && lane.operations.is_empty()
                && lane.member_reads.is_empty()
                && lane.checkpoints.is_empty()
                && !lane.unreachable
        })
    }
}

/// Accumulated statistics and emitted facts for one block transfer.
#[derive(Debug, Default)]
pub(super) struct BlockTransferStats {
    // Counters.
    pub statements_analyzed: usize,
    pub terminators_analyzed: usize,
    pub conflicts_checked: usize,
    pub mutable_call_sites: usize,

    // Per-statement snapshots are diagnostic presentation data, not borrow facts.
    #[cfg(any(test, feature = "show_borrow_checker"))]
    pub statement_entry_states: Vec<(HirNodeId, BorrowStateSnapshot)>,

    // Emitted borrow facts.
    pub statement_facts: Vec<(HirNodeId, StatementBorrowFact)>,
    pub terminator_fact: Option<(BlockId, TerminatorBorrowFact)>,
    pub value_facts: Vec<(HirValueId, ValueBorrowFact)>,
    // Captured operation payloads are installed only after the outgoing edge kills old outcomes.
    pub invoke: InvokeTransfer,
    pub observations: ValueObservations,
    pub diverges: bool,
    pub deferred: bool,
    pub sparse: bool,
    pub sparse_call: Option<CapturedCallBatch<typed_call::MatchedLane>>,
    pub executed_retention: BTreeSet<usize>,
}

/// Executes one forward transfer step for a single basic block.
///
/// The caller owns fixed-point scheduling. This function only applies local
/// transfer rules and emits borrow facts for the processed block.
pub(super) fn transfer_block(
    context: &BorrowTransferContext<'_>,
    layout: &FunctionLayout,
    block: &HirBlock,
    state: &mut BorrowState,
) -> Result<BlockTransferStats, BorrowCheckError> {
    let mut stats = BlockTransferStats::default();
    let mut value_fact_buffer = ValueFactBuffer::new(layout.local_count());

    for statement in &block.statements {
        #[cfg(any(test, feature = "show_borrow_checker"))]
        stats
            .statement_entry_states
            .push((statement.id, state.to_snapshot(&layout.local_ids)));
        transfer_statement(
            context,
            layout,
            state,
            block.id,
            statement,
            &mut stats,
            &mut value_fact_buffer,
        )?;
        value_fact_buffer.finish_use();
        stats.statements_analyzed += 1;
        if stats.diverges {
            stats.value_facts = value_fact_buffer.into_serialized(layout);
            #[cfg(test)]
            state.assert_allocation_incidence();
            return Ok(stats);
        }
    }

    transfer_terminator(
        context,
        layout,
        state,
        block.id,
        &block.terminator,
        &mut stats,
        &mut value_fact_buffer,
    )?;
    stats.terminators_analyzed += 1;

    // Aggregate literal children in return terminators receive optional transfer analysis.
    let mut aggregate_context = AggregateTransferContext {
        diagnostics: &context.diagnostics,
        expressions: context.expressions,
        value_fact_buffer: &mut value_fact_buffer,
    };

    match &block.terminator {
        HirTerminator::Return(values) => {
            let terminator_order = layout.terminator_order_or_unknown(block.id);
            let location = context.diagnostics.terminator_error_span(block.id);
            for value in context.expressions.values(*values) {
                transfer_aggregate_expression_ownership(
                    layout,
                    state,
                    *value,
                    block.id,
                    terminator_order,
                    location,
                    &mut aggregate_context,
                )?;
            }
        }
        HirTerminator::ReturnError(value) => {
            let terminator_order = layout.terminator_order_or_unknown(block.id);
            let location = context.diagnostics.terminator_error_span(block.id);
            transfer_aggregate_expression_ownership(
                layout,
                state,
                *value,
                block.id,
                terminator_order,
                location,
                &mut aggregate_context,
            )?;
        }
        HirTerminator::Invoke { operation, .. } => {
            let terminator_order = layout.terminator_order_or_unknown(block.id);
            let location = context.diagnostics.terminator_error_span(block.id);
            for value in operation.operands(context.expressions) {
                transfer_aggregate_expression_ownership(
                    layout,
                    state,
                    value,
                    block.id,
                    terminator_order,
                    location,
                    &mut aggregate_context,
                )?;
            }
        }
        HirTerminator::NeverCall { args, .. } => {
            // Evaluated arguments keep their aggregate ownership and optional-transfer effects
            // before the call diverges, exactly like an ordinary call's arguments.
            let terminator_order = layout.terminator_order_or_unknown(block.id);
            let location = context.diagnostics.terminator_error_span(block.id);
            for value in context.expressions.values(*args) {
                transfer_aggregate_expression_ownership(
                    layout,
                    state,
                    *value,
                    block.id,
                    terminator_order,
                    location,
                    &mut aggregate_context,
                )?;
            }
        }
        _ => {}
    }

    value_fact_buffer.finish_use();
    stats.value_facts = value_fact_buffer.into_serialized(layout);
    #[cfg(test)]
    state.assert_allocation_incidence();
    Ok(stats)
}
