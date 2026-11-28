//! Borrow-checker fixed-point engine.
//!
//! WHAT: runs the per-function forward dataflow analysis, state joins, and fact persistence.
//! WHY: separating the algorithm from the module seam keeps `mod.rs` a structural map and makes
//!      the engine independently readable.

use crate::borrow_log;
use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckError;
use crate::compiler_frontend::analysis::borrow_checker::diagnostics::BorrowDiagnostics;
use crate::compiler_frontend::analysis::borrow_checker::state::{
    AllocationRetirement, BorrowState, CapturedValue, CellId, FunctionLayout, LocalState, OrderKey,
    RootSet, StoreReceipt, TypedStore, UNKNOWN_ORDER_KEY, WrapperId, remap_invoke_capture,
};
use crate::compiler_frontend::analysis::borrow_checker::transfer::facts::{
    AccessFootprint, Carrier, Demand, ObservationBoundary, ObservationCheckpoint, PlaceUse,
    ValueObservations,
};
use crate::compiler_frontend::analysis::borrow_checker::transfer::{
    BlockTransferStats, BorrowTransferContext, CapturedCallBatch, CapturedDestination,
    EdgeTransferView, InvokeLayerOp, InvokeTransfer, transfer_block,
};
use crate::compiler_frontend::analysis::borrow_checker::types::{
    BorrowCheckReport, BorrowCheckStats, BorrowDropSite, BorrowDropSiteKind, FunctionBorrowSummary,
    LocalMode, ValueBorrowFact,
};
use crate::compiler_frontend::compiler_messages::{DiagnosticPlace, InvalidMutableAccessReason};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::hir::blocks::HirBlock;
use crate::compiler_frontend::hir::functions::HirFunction;
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId, HirValueId, LocalId, RegionId};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::hir::utils::try_for_each_terminator_target;
#[cfg(feature = "benchmark_counters")]
use crate::compiler_frontend::instrumentation::{FrontendCounter, add_frontend_counter};
use crate::compiler_frontend::public_call_summary::PublicCallSummary;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use rustc_hash::{FxHashMap, FxHashSet};
#[cfg(test)]
use std::cell::RefCell;
use std::collections::{BinaryHeap, VecDeque, hash_map::Entry};
use std::rc::Rc;

#[cfg(test)]
#[path = "transfer/tests/invoke_edge_tests.rs"]
mod invoke_edge_tests;

struct ForwardEdgeSource<'a> {
    function: FunctionId,
    block: &'a HirBlock,
    view: EdgeTransferView<'a>,
    order: OrderKey,
}

#[allow(dead_code)]
pub(super) struct CapturedReplayEdge<Outcomes> {
    pub source: BlockId,
    pub destination: BlockId,
    pub order: OrderKey,
    pub outcomes: Outcomes,
}

#[derive(Default)]
struct FunctionObservations {
    blocks: FxHashMap<BlockId, ValueObservations>,
    edges: FxHashMap<(BlockId, usize), ValueObservations>,
    diverges: FxHashSet<BlockId>,
}

impl FunctionObservations {
    fn record_block(
        &mut self,
        block: BlockId,
        observations: ValueObservations,
        diverges: bool,
    ) -> bool {
        let divergence_changed = if diverges {
            self.diverges.insert(block)
        } else {
            self.diverges.remove(&block)
        };
        if observations.is_empty() {
            self.blocks.remove(&block);
        } else {
            self.blocks.insert(block, observations);
        }
        divergence_changed
    }

    fn solve<'s>(
        &'s self,
        layout: &FunctionLayout,
        outputs: &FxHashMap<BlockId, BorrowState>,
        invokes: &FxHashMap<BlockId, InvokeTransfer>,
    ) -> Option<(BlockId, &'s ObservationCheckpoint, &'s Carrier)> {
        if !self
            .blocks
            .values()
            .chain(self.edges.values())
            .any(ValueObservations::has_obligations)
        {
            return None;
        }
        let executable = |block, edge| {
            !self.diverges.contains(&block)
                && invokes.get(&block).is_none_or(|invoke| {
                    if edge == 0 {
                        !invoke.success.unreachable
                    } else {
                        !invoke.error.unreachable
                    }
                })
        };
        let successors = |block| {
            layout.block_successors[&block]
                .iter()
                .copied()
                .enumerate()
                .filter(move |&(edge, successor)| {
                    executable(block, edge) && outputs.contains_key(&successor)
                })
        };

        let mut predecessors = FxHashMap::<BlockId, Vec<BlockId>>::default();
        for &block in outputs.keys() {
            for (_, successor) in successors(block) {
                predecessors.entry(successor).or_default().push(block);
            }
        }
        let mut pending: FxHashSet<_> = self
            .blocks
            .keys()
            .copied()
            .chain(self.edges.keys().map(|&(block, _)| block))
            .collect();
        let mut worklist: VecDeque<_> = pending.iter().copied().collect();
        let mut entry_demand = FxHashMap::<BlockId, Demand>::default();
        let mut conflict: Option<(BlockId, &ObservationCheckpoint, &Carrier)> = None;
        while let Some(block) = worklist.pop_front() {
            pending.remove(&block);
            let mut check = |checkpoint: &'s ObservationCheckpoint, demand: &Demand| {
                let offender = checkpoint
                    .candidates
                    .iter()
                    .filter(|carrier| {
                        demand.contains(*carrier) || checkpoint.pending.contains(*carrier)
                    })
                    .min();
                if let Some(offender) = offender
                    && conflict.is_none_or(|(prior_block, prior, prior_offender)| {
                        (checkpoint.order, checkpoint.span, block.0, offender)
                            < (prior.order, prior.span, prior_block.0, prior_offender)
                    })
                {
                    conflict = Some((block, checkpoint, offender));
                }
            };
            let mut demand = Demand::default();
            for (edge, successor) in successors(block) {
                let next = entry_demand.get(&successor);
                if let Some(summary) = self.edges.get(&(block, edge)) {
                    let mut edge_demand = next.cloned().unwrap_or_default();
                    summary.backward(&mut edge_demand, &mut check);
                    demand.extend(edge_demand);
                } else if let Some(next) = next {
                    demand.extend(next.iter().cloned());
                }
            }
            if let Some(summary) = self.blocks.get(&block) {
                summary.backward(&mut demand, check);
            }
            if !demand.is_empty() && entry_demand.get(&block) != Some(&demand) {
                entry_demand.insert(block, demand);
                if let Some(predecessors) = predecessors.get(&block) {
                    for &pred in predecessors {
                        if pending.insert(pred) {
                            worklist.push_back(pred);
                        }
                    }
                }
            }
        }
        conflict
    }
}

pub(super) struct BorrowChecker<'a> {
    pub(super) module: &'a HirModule,
    pub(super) type_environment: &'a TypeEnvironment,
    pub(super) external_package_registry: &'a ExternalPackageRegistry,
    pub(super) string_table: &'a StringTable,
    pub(super) path_fork: &'a PathInternerFork,
    pub(super) diagnostics: BorrowDiagnostics<'a>,
    // Fast ID lookups used throughout analysis.
    pub(super) block_index_by_id: FxHashMap<BlockId, usize>,
    pub(super) region_parent_by_id: FxHashMap<RegionId, Option<RegionId>>,
    // One frontend-owned local call contract used by transfer and retained for semantic consumers.
    pub(super) public_call_summaries: FxHashMap<FunctionId, PublicCallSummary>,
    pub(super) local_types: FxHashMap<LocalId, TypeId>,
    #[cfg(test)]
    pub(super) forward_probe: Option<RefCell<ForwardProbe>>,
}

impl<'a> BorrowChecker<'a> {
    pub(super) fn new(
        module: &'a HirModule,
        type_environment: &'a TypeEnvironment,
        external_package_registry: &'a ExternalPackageRegistry,
        path_fork: &'a PathInternerFork,
        string_table: &'a StringTable,
    ) -> Self {
        Self::without_transfer_indices(
            module,
            type_environment,
            external_package_registry,
            path_fork,
            string_table,
        )
        .with_local_types()
        .with_region_parents()
    }

    pub(super) fn without_transfer_indices(
        module: &'a HirModule,
        type_environment: &'a TypeEnvironment,
        external_package_registry: &'a ExternalPackageRegistry,
        path_fork: &'a PathInternerFork,
        string_table: &'a StringTable,
    ) -> Self {
        let block_index_by_id = module
            .blocks
            .iter()
            .enumerate()
            .map(|(index, block)| (block.id, index))
            .collect::<FxHashMap<_, _>>();

        Self {
            module,
            type_environment,
            external_package_registry,
            string_table,
            path_fork,
            diagnostics: BorrowDiagnostics::new(module, path_fork, string_table),
            block_index_by_id,
            region_parent_by_id: FxHashMap::default(),
            public_call_summaries: FxHashMap::default(),
            local_types: FxHashMap::default(),
            #[cfg(test)]
            forward_probe: None,
        }
    }

    pub(super) fn with_local_types(mut self) -> Self {
        self.local_types = self
            .module
            .blocks
            .iter()
            .flat_map(|block| &block.locals)
            .map(|local| (local.id, local.ty))
            .collect();
        self
    }

    fn with_region_parents(mut self) -> Self {
        self.region_parent_by_id = self
            .module
            .regions
            .iter()
            .map(|region| (region.id(), region.parent()))
            .collect();
        self
    }

    fn transfer_context(&self) -> BorrowTransferContext<'_> {
        BorrowTransferContext {
            external_package_registry: self.external_package_registry,
            public_call_summaries: &self.public_call_summaries,
            imported_call_summaries: &self.module.imported_call_summaries,
            module_private_call_summaries: &self.module.module_private_call_summaries,
            generated_call_summaries: &self.module.generated_call_summaries,
            local_types: &self.local_types,
            structs: &self.module.structs,
            choices: &self.module.choices,
            expressions: &self.module.expressions,
            type_environment: self.type_environment,
            diagnostics: BorrowDiagnostics::new(self.module, self.path_fork, self.string_table),
        }
    }

    pub(super) fn run(
        mut self,
        summaries: Option<FxHashMap<FunctionId, PublicCallSummary>>,
    ) -> Result<BorrowCheckReport, BorrowCheckError> {
        if let Some(summaries) = summaries {
            self.public_call_summaries = summaries;
        } else {
            self.build_public_call_summaries()?;
        }

        let mut report = BorrowCheckReport::default();

        for function_index in 0..self.module.functions.len() {
            let function_stats = self.analyze_function(function_index, &mut report)?;
            report.stats.functions_analyzed += 1;
            report.stats.blocks_analyzed += function_stats.reachable_blocks;
            report.stats.worklist_iterations += function_stats.worklist_iterations;
        }

        self.finalize_public_call_summary_effects_to_fixed_point(&mut report)?;

        report.analysis.public_call_summaries = self.public_call_summaries;

        borrow_log!(format!(
            "[Borrow] Completed borrow checking: functions={} blocks={} states={} facts={{stmt:{} term:{} value:{}}}",
            report.stats.functions_analyzed,
            report.stats.blocks_analyzed,
            report.analysis.total_state_snapshots(),
            report.analysis.statement_facts.len(),
            report.analysis.terminator_facts.len(),
            report.analysis.value_facts.len()
        ));

        Ok(report)
    }

    pub(super) fn analyze_function(
        &mut self,
        function_index: usize,
        report: &mut BorrowCheckReport,
    ) -> Result<FunctionBorrowSummary, BorrowCheckError> {
        // Function analysis pipeline:
        // 1) Build per-function local layout and visibility masks.
        // 2) Run forward worklist transfer to fixed point.
        // 3) Persist facts + entry/exit snapshots for reachable blocks.
        let function = &self.module.functions[function_index];
        let reachable_blocks = self.collect_reachable_blocks(function)?;
        let layout = self.build_function_layout(function, &reachable_blocks)?;
        #[cfg(feature = "benchmark_counters")]
        add_frontend_counter(
            FrontendCounter::BorrowValidationDenseLayoutSlots,
            layout.local_count() * layout.block_local_max_use_order.len(),
        );
        let visible_locals_by_block = &layout.visible_locals_by_block;

        let transfer_context = self.transfer_context();

        let mut in_states: FxHashMap<BlockId, BorrowState> = FxHashMap::default();
        let mut out_states: FxHashMap<BlockId, BorrowState> = FxHashMap::default();
        let mut invoke_by_block = FxHashMap::<BlockId, InvokeTransfer>::default();
        let mut observations = FunctionObservations::default();
        // HIR expression rows are immutable and may be shared across blocks. Keep only the
        // latest facts for each block during fixed-point iteration, then merge those surviving
        // snapshots after the state has stabilized.
        let mut final_value_facts_by_block: FxHashMap<BlockId, Vec<(HirValueId, ValueBorrowFact)>> =
            FxHashMap::default();

        // Entry state starts as UNINIT, with parameters immediately initialized.
        let mut initial_state = BorrowState::new_uninitialized(layout.local_count());
        #[cfg(test)]
        if let Some(probe) = &self.forward_probe {
            initial_state = probe.borrow().entry.clone();
        }
        #[cfg(test)]
        let supplied_entry = self.forward_probe.is_some();
        #[cfg(not(test))]
        let supplied_entry = false;
        for param in &function.params {
            let Some(param_index) = layout.index_of(*param) else {
                return Err(self.diagnostics.internal_error(format!(
                    "Borrow checker could not map parameter '{}' into function state layout",
                    self.diagnostics.local_name(*param)
                )));
            };

            if !supplied_entry {
                initial_state.initialize_parameter(param_index);
            }
        }

        if let Some(mask) = visible_locals_by_block.get(&function.entry) {
            initial_state.kill_invisible(mask);
        }
        in_states.insert(function.entry, initial_state);

        let mut worklist = ForwardWorklist::new(function.entry, &layout.block_successors);
        #[cfg(test)]
        if let Some(probe) = &self.forward_probe {
            let probe = probe.borrow();
            worklist.fifo = probe.fifo.then(|| VecDeque::from([function.entry]));
            worklist.reverse_siblings = probe.reverse_siblings;
        }

        let mut summary = FunctionBorrowSummary {
            reachable_blocks: reachable_blocks.len(),
            mutable_call_sites: 0,
            worklist_iterations: 0,
        };

        borrow_log!(format!(
            "[Borrow] Analyzing function '{}' (entry={} blocks={})",
            self.diagnostics.function_name(function.id),
            function.entry,
            reachable_blocks.len()
        ));

        // Keep diagnostic priority independent of fact publication order.
        while let Some(block_id) = worklist.pop() {
            #[cfg(test)]
            if let Some(probe) = &self.forward_probe {
                probe.borrow_mut().dequeues.push(block_id);
            }
            summary.worklist_iterations += 1;

            // Stored inputs are already narrowed to this block's visibility: the entry state and
            // every incoming edge are narrowed before insertion or join, and joining states
            // narrowed to the same mask stays narrowed.
            let Some(input_state) = in_states.get(&block_id) else {
                continue;
            };
            #[cfg(feature = "benchmark_counters")]
            add_frontend_counter(
                FrontendCounter::BorrowValidationRowsCloned,
                input_state.validation_row_count(),
            );
            let mut output_state = input_state.clone();

            let block = self.block_by_id_or_error(block_id, function.id)?;

            // Transfer the block once and collect facts while the state is hot.
            let block_stats = transfer_block(&transfer_context, &layout, block, &mut output_state)?;
            self.merge_block_stats(&mut report.stats, &block_stats);
            summary.mutable_call_sites += block_stats.mutable_call_sites;

            #[cfg(any(test, feature = "show_borrow_checker"))]
            for (statement_id, snapshot) in block_stats.statement_entry_states {
                report
                    .analysis
                    .statement_entry_states
                    .insert(statement_id, snapshot);
            }
            for (statement_id, fact) in block_stats.statement_facts {
                report.analysis.statement_facts.insert(statement_id, fact);
            }
            if let Some((terminator_block, fact)) = block_stats.terminator_fact {
                report
                    .analysis
                    .terminator_facts
                    .insert(terminator_block, fact);
            }
            final_value_facts_by_block.insert(block_id, block_stats.value_facts);
            let divergence_changed = layout.lookup_observations
                && observations.record_block(
                    block_id,
                    block_stats.observations,
                    block_stats.diverges,
                );
            let invoke = block_stats.invoke;
            let previous_invoke = invoke_by_block.get(&block_id);
            let has_invoke_effects = !invoke.is_empty();
            let changed_out = out_states.get(&block_id) != Some(&output_state)
                || divergence_changed
                || previous_invoke.map_or(has_invoke_effects, |previous| previous != &invoke);

            // If output state is unchanged, successor joins cannot change either.
            if !changed_out {
                continue;
            }

            let mut deliver_edge = |edge_index, successor| {
                let edge = (block_id, edge_index);
                #[cfg(test)]
                if let Some(probe) = &self.forward_probe {
                    probe.borrow_mut().deliveries.push(edge);
                }
                if block_stats.diverges {
                    #[cfg(test)]
                    if let Some(probe) = &self.forward_probe {
                        probe.borrow_mut().edges.remove(&edge);
                    }
                    observations.edges.remove(&edge);
                    return Ok::<(), BorrowCheckError>(());
                }
                let invoke_lane = if matches!(&block.terminator, HirTerminator::Invoke { .. }) {
                    Some([&invoke.success, &invoke.error][edge_index])
                } else {
                    None
                };
                if invoke_lane.is_some_and(|lane| lane.unreachable) {
                    observations.edges.remove(&edge);
                    #[cfg(test)]
                    if let Some(probe) = &self.forward_probe {
                        probe.borrow_mut().edges.remove(&edge);
                    }
                    return Ok::<(), BorrowCheckError>(());
                }
                if !worklist.ranks.contains_key(&successor) {
                    return Ok::<(), BorrowCheckError>(());
                }

                #[cfg(feature = "benchmark_counters")]
                add_frontend_counter(
                    FrontendCounter::BorrowValidationRowsCloned,
                    output_state.validation_row_count(),
                );
                let mut successor_input = output_state.clone();
                let mut edge_observations = self.apply_forward_edge(
                    &ForwardEdgeSource {
                        function: function.id,
                        block,
                        view: EdgeTransferView::from(&layout),
                        order: layout.terminator_order_or_unknown(block_id),
                    },
                    successor,
                    invoke_lane,
                    &mut successor_input,
                )?;

                // Apply lexical visibility kills before join to prevent
                // branch-local aliases from leaking into outer regions.
                if let Some(mask) = visible_locals_by_block.get(&successor)
                    && !mask.covers_all(layout.local_count())
                {
                    let before = edge_observations.before_rekey(&successor_input);
                    successor_input.kill_invisible(mask);
                    edge_observations.after_rekey(
                        before,
                        &successor_input,
                        |slot| slot.retired(|local| !mask.contains(local)),
                        |id| successor_input.visible_allocation(id, mask),
                    );
                }
                #[cfg(test)]
                if let Some(probe) = &self.forward_probe {
                    probe
                        .borrow_mut()
                        .edges
                        .insert(edge, successor_input.clone());
                }
                if !edge_observations.is_empty() {
                    observations.edges.insert(edge, edge_observations);
                } else {
                    observations.edges.remove(&edge);
                }

                // Revisit successors only when their input state grows.
                let changed_in = match in_states.get_mut(&successor) {
                    Some(existing) => {
                        report.stats.state_joins += 1;
                        #[cfg(feature = "benchmark_counters")]
                        add_frontend_counter(
                            FrontendCounter::BorrowValidationRowsJoined,
                            existing.validation_row_count()
                                + successor_input.validation_row_count(),
                        );
                        existing.join_in_place(&successor_input)
                    }
                    None => {
                        in_states.insert(successor, successor_input);
                        true
                    }
                };
                if changed_in {
                    worklist.push(successor);
                }

                Ok::<(), BorrowCheckError>(())
            };
            #[cfg(test)]
            let reverse_delivery = self
                .forward_probe
                .as_ref()
                .is_some_and(|probe| probe.borrow().reverse_delivery);
            #[cfg(not(test))]
            let reverse_delivery = false;
            if reverse_delivery {
                let mut edges = Vec::new();
                try_for_each_terminator_target(&block.terminator, |successor| {
                    edges.push((edges.len(), successor));
                    Ok::<_, BorrowCheckError>(())
                })?;
                for (index, successor) in edges.into_iter().rev() {
                    deliver_edge(index, successor)?;
                }
            } else {
                let mut edge_index = 0;
                try_for_each_terminator_target(&block.terminator, |successor| {
                    let index = edge_index;
                    edge_index += 1;
                    deliver_edge(index, successor)
                })?;
            }
            #[cfg(test)]
            worklist.finish_successors();

            out_states.insert(block_id, output_state);
            if has_invoke_effects {
                invoke_by_block.insert(block_id, invoke);
            } else {
                invoke_by_block.remove(&block_id);
            }
        }
        #[cfg(test)]
        if let Some(probe) = &self.forward_probe {
            let mut probe = probe.borrow_mut();
            probe.block_observations = observations.blocks.clone();
            probe.observations = observations
                .edges
                .iter()
                .map(|(&edge, observations)| (edge, observations.clone()))
                .collect();
        }

        // The verdict uses final executable edges and cannot change forward state.
        if let Some((block, at, carrier)) =
            observations.solve(&layout, &out_states, &invoke_by_block)
        {
            let local = match carrier {
                Carrier::Cell(CellId::Slot(slot), ..) | Carrier::Route(slot, ..) => {
                    Some(slot.local)
                }
                Carrier::Cell(CellId::Projected(holder, _), ..) => {
                    out_states[&block].alloc_origin_local(*holder)
                }
            };
            let diagnostics = &transfer_context.diagnostics;
            let name = |local: usize| diagnostics.local_place(layout.local_ids[local]);
            return Err(diagnostics.invalid_mutable_access(
                at.actor.map_or(DiagnosticPlace::Unknown, name),
                InvalidMutableAccessReason::AliasedValueRequiresExclusiveAccess,
                local.map(name),
                local.and_then(|local| diagnostics.local_source_span(layout.local_ids[local])),
                at.span,
            ));
        }

        // A row may be observed under different final states in different blocks or functions.
        // Merge only the last transfer result for each reachable block; merging during worklist
        // visits would retain transient facts from earlier iterations.
        // A single context already has sorted roots: the layout sorts LocalIds before the root
        // bitset is projected back to IDs. Only shared rows need the sorting merge.
        for block_id in &reachable_blocks {
            if let Some(value_facts) = final_value_facts_by_block.remove(block_id) {
                for (value_id, fact) in value_facts {
                    match report.analysis.value_facts.entry(value_id) {
                        Entry::Occupied(mut existing) => existing.get_mut().merge(fact),
                        Entry::Vacant(vacant) => {
                            vacant.insert(fact);
                        }
                    }
                }
            }
        }

        // Borrow-state snapshots support developer presentation only; downstream analyses use
        // the immutable semantic facts and fixed-point states above.
        #[cfg(any(test, feature = "show_borrow_checker"))]
        for block_id in &reachable_blocks {
            if let Some(state) = in_states.get(block_id) {
                report
                    .analysis
                    .block_entry_states
                    .insert(*block_id, state.to_snapshot(&layout.local_ids));
            }

            if let Some(state) = out_states.get(block_id) {
                report
                    .analysis
                    .block_exit_states
                    .insert(*block_id, state.to_snapshot(&layout.local_ids));
            }
        }

        self.record_advisory_drop_sites(function, &reachable_blocks, &layout, &out_states, report)?;

        report
            .analysis
            .function_summaries
            .insert(function.id, summary.clone());

        Ok(summary)
    }

    pub(super) fn finalize_public_call_summary_effects_to_fixed_point(
        &mut self,
        report: &mut BorrowCheckReport,
    ) -> Result<(), BorrowCheckError> {
        let parameter_count = self
            .module
            .functions
            .iter()
            .map(|function| function.params.len())
            .sum::<usize>();
        let max_iterations = parameter_count.saturating_add(1);

        for _ in 0..max_iterations {
            let mut changed = false;
            let module = self.module;
            for function in &module.functions {
                if function.params.is_empty() {
                    continue;
                }
                let reachable_blocks = self.collect_reachable_blocks(function)?;
                changed |=
                    self.finalize_public_call_summary_effects(function, &reachable_blocks, report)?;
            }

            if !changed {
                return Ok(());
            }
        }

        Err(self
            .diagnostics
            .internal_error("Borrow checker could not stabilize local mutation summaries"))
    }

    fn record_advisory_drop_sites(
        &self,
        function: &HirFunction,
        reachable_blocks: &[BlockId],
        layout: &FunctionLayout,
        out_states: &FxHashMap<BlockId, BorrowState>,
        report: &mut BorrowCheckReport,
    ) -> Result<(), BorrowCheckError> {
        for block_id in reachable_blocks {
            let Some(exit_state) = out_states.get(block_id) else {
                continue;
            };

            let candidate_locals = self.drop_candidate_locals_from_state(exit_state, layout);
            if candidate_locals.is_empty() {
                continue;
            }

            let block = self.block_by_id_or_error(*block_id, function.id)?;
            let mut block_sites = Vec::new();

            match &block.terminator {
                HirTerminator::Return(_) | HirTerminator::ReturnError(_) => {
                    block_sites.push(BorrowDropSite {
                        kind: BorrowDropSiteKind::Return,
                        locals: candidate_locals.clone(),
                    });
                }
                HirTerminator::Invoke { .. } => {}
                HirTerminator::Break { .. } => {
                    block_sites.push(BorrowDropSite {
                        kind: BorrowDropSiteKind::Break,
                        locals: candidate_locals.clone(),
                    });
                }
                _ => {}
            }

            if self.block_has_region_exit_edge(block.id, block.region, function.id)? {
                block_sites.push(BorrowDropSite {
                    kind: BorrowDropSiteKind::BlockExit,
                    locals: candidate_locals,
                });
            }

            if block_sites.is_empty() {
                continue;
            }

            report
                .analysis
                .advisory_drop_sites
                .entry(*block_id)
                .or_default()
                .extend(block_sites);
        }

        for sites in report.analysis.advisory_drop_sites.values_mut() {
            sites.sort_by_key(|site| match site.kind {
                BorrowDropSiteKind::BlockExit => 0u8,
                BorrowDropSiteKind::Return => 1u8,
                BorrowDropSiteKind::Break => 2u8,
            });
        }

        Ok(())
    }

    fn drop_candidate_locals_from_state(
        &self,
        state: &BorrowState,
        layout: &FunctionLayout,
    ) -> Vec<LocalId> {
        let mut locals = Vec::new();

        for (index, local_id) in layout.local_ids.iter().enumerate() {
            if state.binding_mode(index).contains(LocalMode::SLOT) {
                locals.push(*local_id);
            }
        }

        locals.sort_by_key(|local| local.0);
        locals.dedup_by_key(|local| local.0);
        locals
    }

    fn block_has_region_exit_edge(
        &self,
        block_id: BlockId,
        block_region: RegionId,
        function_id: FunctionId,
    ) -> Result<bool, BorrowCheckError> {
        let block = self.block_by_id_or_error(block_id, function_id)?;
        let mut has_region_exit_edge = false;

        try_for_each_terminator_target(&block.terminator, |successor| {
            let successor_block = self.block_by_id_or_error(successor, function_id)?;
            if !self.is_same_or_descendant_region(successor_block.region, block_region) {
                has_region_exit_edge = true;
            }

            Ok::<(), BorrowCheckError>(())
        })?;

        Ok(has_region_exit_edge)
    }

    fn is_same_or_descendant_region(&self, region: RegionId, ancestor: RegionId) -> bool {
        let mut current = Some(region);
        while let Some(region_id) = current {
            if region_id == ancestor {
                return true;
            }

            current = self.region_parent_by_id.get(&region_id).copied().flatten();
        }

        false
    }

    pub(super) fn block_by_id_or_error(
        &self,
        block_id: BlockId,
        function_id: FunctionId,
    ) -> Result<&crate::compiler_frontend::hir::blocks::HirBlock, BorrowCheckError> {
        let Some(index) = self.block_index_by_id.get(&block_id).copied() else {
            return Err(self.diagnostics.internal_error(format!(
                "Borrow checker could not resolve block '{}' while analyzing function '{}'",
                block_id,
                self.diagnostics.function_name(function_id)
            )));
        };

        Ok(&self.module.blocks[index])
    }

    fn merge_block_stats(&self, total: &mut BorrowCheckStats, block: &BlockTransferStats) {
        total.statements_analyzed += block.statements_analyzed;
        total.terminators_analyzed += block.terminators_analyzed;
        total.conflicts_checked += block.conflicts_checked;
    }

    /// A component which does not write at an edge shares its incoming facts at the successor.
    #[allow(dead_code)] // Prepared for guarded extraction over a projected support.
    pub(super) fn drive_sparse_support(
        &self,
        function: &HirFunction,
        entry: BorrowState,
        support: &super::metadata::ProjectedSupport<'_>,
    ) -> Result<FxHashMap<BlockId, Rc<BorrowState>>, BorrowCheckError> {
        self.drive_sparse_support_from(function, function.entry, entry, support)
    }

    fn drive_sparse_support_from(
        &self,
        function: &HirFunction,
        start: BlockId,
        mut entry: BorrowState,
        support: &super::metadata::ProjectedSupport<'_>,
    ) -> Result<FxHashMap<BlockId, Rc<BorrowState>>, BorrowCheckError> {
        use super::transfer::transfer_sparse_block;

        let context = self.transfer_context();
        entry.use_indexed_recurring_blocks(support.recurring_blocks());
        if let Some(mask) = support.visibility(start) {
            entry.kill_invisible(mask);
        }
        let entry_block = support.continuation(start);
        let mut worklist = ForwardWorklist::with_ranks(entry_block, support.ranks());
        let mut inputs = FxHashMap::default();
        inputs.insert(entry_block, Rc::new(entry));
        let mut exits = FxHashMap::default();
        while let Some(block_id) = worklist.pop() {
            let block = self.block_by_id_or_error(block_id, function.id)?;
            if matches!(block.terminator, HirTerminator::Uninitialized) {
                return Err(context
                    .diagnostics
                    .internal_error("Missing terminator during sparse borrow transfer"));
            }
            let view = support.block_view(block_id);
            let source = ForwardEdgeSource {
                function: function.id,
                block,
                view: EdgeTransferView {
                    operations: view.operations,
                    observations: false,
                },
                order: view.terminator_order,
            };
            let transfer_terminator = view.transfer_terminator;
            let mut state = Rc::clone(&inputs[&block_id]);
            let transfer = if view.statement_positions.is_empty() && !transfer_terminator {
                BlockTransferStats::default()
            } else {
                transfer_sparse_block(&context, view, block, Rc::make_mut(&mut state))?
            };
            if transfer.diverges {
                continue;
            }
            self.transfer_sparse_successors(
                &source,
                transfer_terminator.then_some(&transfer.invoke),
                &state,
                support,
                |successor, incoming| {
                    let changed = match inputs.entry(successor) {
                        Entry::Occupied(mut entry) => {
                            let existing = entry.get_mut();
                            !Rc::ptr_eq(existing, &incoming)
                                && existing.as_ref() != incoming.as_ref()
                                && Rc::make_mut(existing).join_in_place(&incoming)
                        }
                        Entry::Vacant(entry) => {
                            entry.insert(incoming);
                            true
                        }
                    };
                    if changed {
                        worklist.push(successor);
                    }
                },
            )?;
            exits.insert(block_id, state);
        }
        Ok(exits)
    }

    /// Resume the same sparse solve from one alternative's receiving state.
    /// Callers supply its projected predecessor, never a dense join.
    #[allow(dead_code)]
    pub(super) fn replay_captured_sparse_support(
        &self,
        function: &HirFunction,
        edge: CapturedReplayEdge<impl IntoIterator<Item = usize>>,
        mut entry: BorrowState,
        support: &super::metadata::ProjectedSupport<'_>,
        batch: CapturedCallBatch,
        mut rekey: impl FnMut(&BorrowState, AllocationRetirement),
    ) -> Result<FxHashMap<BlockId, Rc<BorrowState>>, BorrowCheckError> {
        entry.use_indexed_recurring_blocks(support.recurring_blocks());
        apply_captured_call_batch_rekey(
            EdgeTransferView {
                operations: support.block_view(edge.source).operations,
                observations: false,
            },
            batch,
            edge.outcomes,
            edge.order,
            &mut entry,
            &mut rekey,
        )?;
        self.drive_sparse_support_from(function, edge.destination, entry, support)
    }

    fn transfer_sparse_successors(
        &self,
        source: &ForwardEdgeSource<'_>,
        terminator_effects: Option<&InvokeTransfer>,
        state: &Rc<BorrowState>,
        support: &super::metadata::ProjectedSupport<'_>,
        mut deliver: impl FnMut(BlockId, Rc<BorrowState>),
    ) -> Result<(), BorrowCheckError> {
        let terminator = &source.block.terminator;
        let mut lanes = terminator_effects
            .filter(|_| matches!(terminator, HirTerminator::Invoke { .. }))
            .map(|invoke| [&invoke.success, &invoke.error].into_iter());
        let jump_rebinds = terminator_effects.is_some()
            && matches!(terminator, HirTerminator::Jump { args, .. } if !args.is_empty());
        try_for_each_terminator_target(terminator, |successor| {
            let lane = lanes.as_mut().and_then(Iterator::next);
            if lane.is_some_and(|lane| lane.unreachable) {
                return Ok(());
            }
            let mut incoming = Rc::clone(state);
            if jump_rebinds || lane.is_some() {
                self.apply_forward_edge(source, successor, lane, Rc::make_mut(&mut incoming))?;
            }
            if let Some(mask) = support.visibility(successor) {
                Rc::make_mut(&mut incoming).kill_invisible(mask);
            }
            deliver(support.continuation(successor), incoming);
            Ok(())
        })
    }

    fn apply_forward_edge(
        &self,
        source: &ForwardEdgeSource<'_>,
        successor: BlockId,
        invoke: Option<&super::transfer::InvokeLaneTransfer>,
        state: &mut BorrowState,
    ) -> Result<ValueObservations, BorrowCheckError> {
        let ForwardEdgeSource {
            function,
            block,
            view,
            order,
        } = *source;
        let terminator = &block.terminator;
        if let Some(lane) = invoke {
            let operations = view.operations;
            let fresh = lane.operations.is_empty()
                && !state.layer_is_empty()
                && !state.block_may_repeat(block.id, operations.block_successors);
            Ok(apply_invoke_edge_transfer(
                view,
                terminator,
                lane,
                order,
                |index| {
                    fresh && operations.local_first_write_order[index] == order
                        && matches!(
                            self.diagnostics.local_origin_kind(operations.local_ids[index]),
                            Some(crate::compiler_frontend::hir::hir_side_table::HirLocalOriginKind::CompilerTemp)
                        )
                },
                state,
            ))
        } else {
            self.apply_jump_argument_transfer(function, view, terminator, successor, order, state)
        }
    }

    pub(super) fn apply_jump_argument_transfer(
        &self,
        function_id: FunctionId,
        view: EdgeTransferView<'_>,
        terminator: &HirTerminator,
        successor: BlockId,
        order: OrderKey,
        successor_input: &mut BorrowState,
    ) -> Result<ValueObservations, BorrowCheckError> {
        let layout = view.operations;
        let mut observations = ValueObservations {
            enabled: view.observations,
            ..Default::default()
        };
        let HirTerminator::Jump { target, args } = terminator else {
            return Ok(observations);
        };

        if *target != successor || args.is_empty() {
            return Ok(observations);
        }

        let mut destination_states = args
            .iter()
            .map(|argument| {
                let Some(source_index) = layout.index_of(argument.source) else {
                    return Err(self.diagnostics.internal_error(
                        format!(
                            "Borrow checker could not map jump argument local '{}' into function state layout",
                            self.diagnostics.local_name(argument.source)
                        ),
                    ));
                };

                if observations.enabled && successor_input.has_value_alternatives() {
                    let mut resolved =
                        successor_input.resolve_place_path(source_index, layout.local_mutable, []);
                    if layout.local_has_in_place_mutation_surface[source_index] {
                        successor_input.load_place_content(&mut resolved, layout.local_mutable, false);
                    }
                    let mut access = AccessFootprint::cells(resolved.address, PlaceUse::Read);
                    access.content = resolved.content;
                    observations.tail.observe(successor_input, &access, true, Some(source_index));
                }
                let payload = successor_input.capture_jump_value(
                    source_index,
                    !layout.local_has_in_place_mutation_surface[source_index],
                    false,
                );
                let detached = successor_input.detached_holders(source_index).collect::<Vec<_>>();
                let destination_state = if successor_input
                    .binding_mode(source_index)
                    .is_definitely_uninit()
                {
                    LocalState::uninit(layout.local_count())
                } else {
                    // A jump transports the selected value into an independent slot, not the
                    // source's cell or route. Preserve its provenance and must derivation so a
                    // read through the destination still derives from the loan on every path
                    // that supplied that proof.
                    let mut destination = LocalState::slot_from_value_roots(
                        successor_input.effective_roots(source_index),
                    );
                    destination.derived_from_roots =
                        successor_input.derived_roots_from_source(source_index);
                    destination
                };

                Ok((source_index, destination_state, payload, detached))
            })
            .collect::<Result<Vec<_>, BorrowCheckError>>()?;

        let successor_block = self.block_by_id_or_error(successor, function_id)?;
        let destination_indices = args
            .iter()
            .map(|argument| {
                if !successor_block
                    .locals
                    .iter()
                    .any(|local| local.id == argument.destination)
                {
                    return Err(self.diagnostics.internal_error(
                        format!(
                            "Borrow checker saw jump edge into block '{}' defining local '{}' that the block does not own",
                            successor,
                            self.diagnostics.local_name(argument.destination)
                        ),
                    ));
                }
                let Some(destination_index) = layout.index_of(argument.destination) else {
                    return Err(self.diagnostics.internal_error(
                        format!(
                            "Borrow checker could not map jump destination local '{}' into function state layout",
                            self.diagnostics.local_name(argument.destination)
                        ),
                    ));
                };

                Ok(destination_index)
            })
            .collect::<Result<Vec<_>, BorrowCheckError>>()?;

        let retires = |local| destination_indices.contains(&local);
        for (source, destination_state, payload, _) in &mut destination_states {
            for &retired in &destination_indices {
                destination_state.derived_from_roots.remove(retired);
            }
            if payload.is_none() && destination_state.value_roots.iter_ones().any(retires) {
                // A root-only source also needs an override when the edge overwrites its root.
                *payload = successor_input.capture_jump_value(
                    *source,
                    !layout.local_has_in_place_mutation_surface[*source],
                    true,
                );
            }
        }

        if !successor_input.layer_is_empty()
            || destination_states
                .iter()
                .any(|(_, _, payload, detached)| payload.is_some() || !detached.is_empty())
        {
            for &index in &destination_indices {
                let before = observations.before_rekey(successor_input);
                let retirement = successor_input.retire_binding_allocation(index, order);
                successor_input.update_local_state(index, LocalState::uninit(layout.local_count()));
                observations.after_retirement(before, successor_input, retirement);
            }
        }

        // No destination is installed until all sources and their detachments have been captured
        // and every old destination has been retired.
        for ((_, destination_state, payload, mut detached), &destination_index) in
            destination_states.into_iter().zip(&destination_indices)
        {
            successor_input.update_local_state(destination_index, destination_state);
            let remap = |id| successor_input.remap_retired_alloc_id(id, order, retires);
            for holder in &mut detached {
                *holder = remap(*holder);
            }
            if let Some(mut value) = payload {
                value.remap_allocations(remap);
                let readonly = value.readonly();
                let explicit_allocation = value.alternatives.is_empty() || !value.values.is_empty();
                let store = TypedStore::receive_result(
                    successor_input.resolve_cells(destination_index),
                    value,
                    destination_index,
                    readonly,
                );
                let store = if explicit_allocation {
                    store.with_explicit_slot_payload()
                } else {
                    store
                };
                observations
                    .tail
                    .replace(successor_input.publish_store(&store, order, None));
            }
            if !detached.is_empty() {
                successor_input.bind_detached_holders(destination_index, detached);
            }
        }
        observations.prune_retired_wrappers(successor_input);
        Ok(observations)
    }
}

struct ForwardWorklist {
    ranks: Rc<FxHashMap<BlockId, usize>>,
    rpo: BinaryHeap<(usize, u32)>,
    pending: FxHashSet<BlockId>,
    #[cfg(test)]
    fifo: Option<VecDeque<BlockId>>,
    #[cfg(test)]
    reverse_siblings: bool,
    #[cfg(test)]
    siblings: Vec<BlockId>,
}

impl ForwardWorklist {
    fn new(entry: BlockId, successors: &FxHashMap<BlockId, Vec<BlockId>>) -> Self {
        Self::with_ranks(entry, Rc::new(reverse_postorder_ranks(entry, successors)))
    }

    fn with_ranks(entry: BlockId, ranks: Rc<FxHashMap<BlockId, usize>>) -> Self {
        let rpo = BinaryHeap::from([(ranks[&entry], entry.0)]);
        Self {
            ranks,
            rpo,
            pending: [entry].into_iter().collect(),
            #[cfg(test)]
            fifo: None,
            #[cfg(test)]
            reverse_siblings: false,
            #[cfg(test)]
            siblings: Vec::new(),
        }
    }

    fn push(&mut self, block: BlockId) {
        if !self.pending.insert(block) {
            return;
        }
        #[cfg(test)]
        if let Some(fifo) = &mut self.fifo {
            if self.reverse_siblings {
                self.siblings.push(block);
            } else {
                fifo.push_back(block);
            }
            return;
        }
        self.rpo.push((self.ranks[&block], block.0));
    }

    fn pop(&mut self) -> Option<BlockId> {
        #[cfg(test)]
        if let Some(fifo) = &mut self.fifo {
            let block = fifo.pop_front()?;
            self.pending.remove(&block);
            return Some(block);
        }
        // Clear before transfer so a self-edge can queue this block again.
        let (_, index) = self.rpo.pop()?;
        let block = BlockId(index);
        self.pending.remove(&block);
        Some(block)
    }

    #[cfg(test)]
    fn finish_successors(&mut self) {
        if let Some(fifo) = &mut self.fifo {
            fifo.extend(self.siblings.drain(..).rev());
        }
    }
}

#[cfg(test)]
pub(super) struct ForwardProbe {
    pub entry: BorrowState,
    pub fifo: bool,
    pub reverse_siblings: bool,
    pub reverse_delivery: bool,
    pub deliveries: Vec<(BlockId, usize)>,
    pub dequeues: Vec<BlockId>,
    pub edges: FxHashMap<(BlockId, usize), BorrowState>,
    pub observations: FxHashMap<(BlockId, usize), ValueObservations>,
    pub block_observations: FxHashMap<BlockId, ValueObservations>,
}

/// A larger finishing rank means earlier reverse-postorder priority.
pub(super) fn reverse_postorder_ranks(
    entry: BlockId,
    successors: &FxHashMap<BlockId, Vec<BlockId>>,
) -> FxHashMap<BlockId, usize> {
    let mut ranks = FxHashMap::default();
    ranks.reserve(successors.len());
    ranks.insert(entry, 0);
    // DFS follows successors backwards so independent successors keep their authored order
    // when postorder is reversed.
    let mut stack = vec![(entry, successors[&entry].iter().rev())];
    let mut rank = 0;
    while let Some((block, children)) = stack.last_mut() {
        if let Some(&child) = children.next() {
            if let Entry::Vacant(seen) = ranks.entry(child) {
                seen.insert(0);
                stack.push((child, successors[&child].iter().rev()));
            }
        } else {
            ranks.insert(*block, rank);
            rank += 1;
            stack.pop();
        }
    }
    ranks
}

/// Apply the captures of one selected outcome and no effects from its sibling.
/// Reads use entry coordinates; a receiving destination never supplies a
/// source for another captured operation.
#[allow(dead_code)]
pub(super) fn apply_captured_call_batch(
    view: EdgeTransferView<'_>,
    batch: CapturedCallBatch,
    outcomes: impl IntoIterator<Item = usize>,
    order: OrderKey,
    state: &mut BorrowState,
) -> Result<ValueObservations, crate::compiler_frontend::compiler_errors::InfrastructureFailure> {
    apply_captured_call_batch_rekey(view, batch, outcomes, order, state, &mut |_, _| {})
}

fn apply_captured_call_batch_rekey(
    view: EdgeTransferView<'_>,
    mut batch: CapturedCallBatch,
    outcomes: impl IntoIterator<Item = usize>,
    order: OrderKey,
    state: &mut BorrowState,
    rekey: &mut impl FnMut(&BorrowState, AllocationRetirement),
) -> Result<ValueObservations, crate::compiler_frontend::compiler_errors::InfrastructureFailure> {
    if batch
        .post
        .results
        .iter()
        .any(|result| result.slot >= batch.destinations.len())
    {
        return Err(
            crate::compiler_frontend::public_call_summary::allocation_error(
                "typed call result has no receiving destination",
            ),
        );
    }
    let layout = view.operations;
    let mut observations = ValueObservations {
        enabled: view.observations,
        ..Default::default()
    };
    for checkpoint in std::mem::take(&mut batch.checkpoints) {
        observations.split(ObservationBoundary::Checkpoint(checkpoint));
    }
    observations
        .tail
        .read(std::mem::take(&mut batch.member_reads));
    let mut retired = FxHashSet::default();
    // Include immediate definitions as well as the predecessor's invoke outcomes.
    let destinations = 0..batch.destinations.len();
    let retirement_order = outcomes.into_iter().map(Ok).chain(destinations.map(Err));
    for next in retirement_order {
        let index = match next {
            Ok(index) => index,
            Err(slot) => match batch.destinations[slot] {
                CapturedDestination::Define(index) => index,
                CapturedDestination::Update { .. } => continue,
            },
        };
        if !retired.insert(index) {
            continue;
        }
        let wrapper = WrapperId::current(index);
        let retained = batch.post.names_wrapper(wrapper)
            || batch.destinations.iter().any(|destination| {
                matches!(destination, CapturedDestination::Update { address, .. }
                    if address.cells().contains(&CellId::Slot(wrapper)))
            });
        if !retained
            && layout.local_first_write_order[index] == order
            && state.can_publish_first_definition(index)
        {
            continue;
        }
        let before = observations.before_rekey(state);
        if retained {
            state.retain_captured_wrapper(index);
        }
        let retirement = state.retire_binding_allocation(index, order);
        state.update_local_state(index, LocalState::uninit(layout.local_count()));
        observations.after_retirement(before, state, retirement);
        batch.post.remap(state, retirement);
        rekey(state, retirement);
        remap_receivers(
            &mut batch.destinations,
            batch.detachment.as_mut(),
            state,
            retirement,
        );
    }

    batch.post.weaken_collisions(state);
    let (results, receipt) = batch.post.apply(state, order);
    observations.publish(receipt);
    let mut receiving = results.into_iter();
    let mut detached_result = None;
    while let Some(mut result) = receiving.next() {
        let (actor, mut address, define) = match &batch.destinations[result.slot] {
            CapturedDestination::Define(index) => {
                state.update_local_state(*index, LocalState::slot(layout.local_count()));
                (*index, state.resolve_cells(*index), true)
            }
            CapturedDestination::Update { actor, address } => (*actor, address.clone(), false),
        };
        if !define
            && let Some(CellId::Slot(wrapper)) = address.definite()
            && layout.local_has_in_place_mutation_surface[wrapper.local]
        {
            // Replacing an owned value retires its allocation, not the slot.
            let before = observations.before_rekey(state);
            let retirement = state.retire_slot_value(wrapper.local, order);
            observations.after_retirement(before, state, retirement);
            rekey(state, retirement);
            result.remap(state, retirement);
            address.remap_allocations(state, retirement);
            for pending in receiving.as_mut_slice() {
                pending.remap(state, retirement);
            }
            remap_receivers(
                &mut batch.destinations,
                batch.detachment.as_mut(),
                state,
                retirement,
            );
        }
        use super::transfer::typed_call::CapturedDerivation;
        let derivation = match result.derivation {
            Some(CapturedDerivation::Roots(roots)) => roots,
            source => {
                let mut roots = RootSet::empty(layout.local_count());
                if let Some(CapturedDerivation::Wrapper(wrapper)) = source
                    && wrapper.local < layout.local_count()
                    && !retired.contains(&wrapper.local)
                {
                    roots.insert(wrapper.local);
                }
                roots
            }
        };
        let slot = match address.definite() {
            Some(CellId::Slot(wrapper)) => wrapper.local,
            _ => actor,
        };
        let readonly = result.value.readonly();
        if batch
            .detachment
            .as_ref()
            .is_some_and(|removal| removal.result == Some(result.slot))
        {
            detached_result = match address.definite() {
                Some(CellId::Slot(wrapper)) if wrapper.age == super::state::WrapperAge::Current => {
                    Some(wrapper.local)
                }
                _ => None,
            };
        }
        let store = TypedStore::receive_result(address, result.value, actor, readonly)
            .with_explicit_slot_payload()
            .with_slot_roots(
                result
                    .roots
                    .unwrap_or_else(|| RootSet::empty(layout.local_count())),
            );
        observations.publish(state.publish_captured_store(&store, order));
        state.mark_slot_derivation(slot, derivation);
    }
    if let Some(detachment) = batch.detachment {
        // Removal observes the newly installed receiver before detaching it.
        observations.tail.replace(state.remove_alloc_element(
            &detachment.holders,
            detachment.map,
            detached_result,
        ));
    }
    observations.prune_retired_wrappers(state);
    Ok(observations)
}

fn remap_receivers(
    destinations: &mut [CapturedDestination],
    detachment: Option<&mut super::transfer::CapturedDetachment>,
    state: &BorrowState,
    retirement: AllocationRetirement,
) {
    for destination in destinations {
        if let CapturedDestination::Update { address, .. } = destination {
            address.remap_allocations(state, retirement);
        }
    }
    if let Some(detachment) = detachment {
        for holder in &mut detachment.holders {
            retirement.remap_id(state, holder);
        }
    }
}

/// Retire both previous outcomes before installing the selected lane's captures.
fn apply_invoke_edge_transfer(
    view: EdgeTransferView<'_>,
    terminator: &HirTerminator,
    lane: &super::transfer::InvokeLaneTransfer,
    order: OrderKey,
    virgin_outcome: impl Fn(usize) -> bool,
    edge_state: &mut BorrowState,
) -> ValueObservations {
    let HirTerminator::Invoke {
        success_results,
        error_result,
        ..
    } = terminator
    else {
        return ValueObservations::default();
    };
    let layout = view.operations;
    let mut observations = ValueObservations {
        enabled: view.observations,
        ..Default::default()
    };
    for checkpoint in &lane.checkpoints {
        observations.split(ObservationBoundary::Checkpoint(checkpoint.clone()));
    }
    observations.tail.read(lane.member_reads.iter().cloned());
    let apply_layer_effects = !lane.operations.is_empty();
    let outcomes = || success_results.iter().chain(std::iter::once(error_result));
    for local in outcomes() {
        if let Some(index) = layout.index_of(*local) {
            // An acyclic first write to compiler-owned result storage has no old capture.
            if virgin_outcome(index) && edge_state.can_publish_first_definition(index) {
                edge_state.update_local_state(index, LocalState::uninit(layout.local_count()));
                continue;
            }
            let before = observations.before_rekey(edge_state);
            let retirement = if apply_layer_effects {
                if lane.operations.iter().any(
                    |op| matches!(op, InvokeLayerOp::Store(store) if store.captures_slot(index)),
                ) {
                    edge_state.retain_captured_wrapper(index);
                }
                // Captures can create the first sparse facts on this edge. Still snapshot any
                // existing root-only observers before changing the result binding.
                let retirement = edge_state.retire_binding_allocation(index, order);
                edge_state.update_local_state(index, LocalState::uninit(layout.local_count()));
                retirement
            } else {
                edge_state.rebind_local(index, LocalState::uninit(layout.local_count()), order)
            };
            observations.after_retirement(before, edge_state, retirement);
        }
    }
    for (local, result) in &lane.results {
        if let Some(index) = layout.index_of(*local) {
            let mut result = result.clone();
            for retired in outcomes() {
                if let Some(retired) = layout.index_of(*retired) {
                    result.derived_from_roots.remove(retired);
                }
            }
            edge_state.update_local_state(index, result);
        }
    }
    let retires = |index| {
        let local = layout.local_ids[index];
        local == *error_result || success_results.contains(&local)
    };
    for op in &lane.operations {
        observations.tail.replace(op.apply_with_indices(
            &|local| layout.index_of(local),
            edge_state,
            Some((order, &retires)),
        ));
    }
    observations.prune_retired_wrappers(edge_state);
    observations
}

impl InvokeLayerOp {
    pub(super) fn captures_current(
        &self,
        local: usize,
        names: &impl Fn(super::state::AllocId) -> bool,
    ) -> bool {
        match self {
            Self::Store(store) => store.captures_current(local, names),
            Self::Contribute { holder, value, .. } => names(*holder) || names(value.id),
            Self::ConstructField { holder, value, .. } => {
                names(*holder) || value.names_allocation(names)
            }
            Self::Remove { holders, .. } => holders.iter().copied().any(names),
            Self::Bind { value, .. } => value.names_allocation(names),
            Self::ViewMember { view, id } => names(*view) || names(*id),
            Self::ViewField { holder, view, .. } => names(*holder) || names(*view),
            Self::ViewPair { left, right, .. } => names(*left) || names(*right),
        }
    }

    pub(super) fn apply_owned(
        self,
        view: super::transfer::OperationView<'_>,
        state: &mut BorrowState,
    ) -> StoreReceipt {
        let index_of = |local| view.index_of(local);
        match self {
            Self::Bind { local, value } => {
                Self::publish_bound_value(&index_of, state, local, value)
            }
            other => other.apply_with_indices(&index_of, state, None),
        }
    }

    fn publish_bound_value(
        index_of: &impl Fn(LocalId) -> Option<usize>,
        state: &mut BorrowState,
        local: LocalId,
        value: CapturedValue,
    ) -> StoreReceipt {
        let Some(index) = index_of(local) else {
            return StoreReceipt::default();
        };
        // The receiving place is resolved after retirement. Only the value uses entry IDs.
        let readonly = value.readonly();
        let store = TypedStore::receive_result(state.resolve_cells(index), value, index, readonly);
        state.publish_store(&store, UNKNOWN_ORDER_KEY, None)
    }

    fn apply_with_indices(
        &self,
        index_of: &impl Fn(LocalId) -> Option<usize>,
        state: &mut BorrowState,
        rebase: Option<(OrderKey, &dyn Fn(usize) -> bool)>,
    ) -> StoreReceipt {
        let remap_id = |state: &BorrowState, id| match rebase {
            Some((order, retires)) => state.remap_retired_alloc_id(id, order, retires),
            None => id,
        };
        match self {
            Self::Store(store) => {
                let (order, retires) = rebase
                    .map_or((UNKNOWN_ORDER_KEY, None), |(order, retires)| {
                        (order, Some(retires))
                    });
                return state.publish_store(store, order, retires);
            }
            Self::Contribute { holder, key, value } => {
                let holder = remap_id(state, *holder);
                let mut value = *value;
                value.id = remap_id(state, value.id);
                state.contribute_alloc_edge(holder, *key, value);
            }
            Self::ConstructField { holder, key, value } => {
                let holder = remap_id(state, *holder);
                if rebase.is_none() {
                    state.record_constructed_value(holder, *key, value, false);
                } else if value.alternatives.is_empty() {
                    let values = remap_invoke_capture(&value.values, |mut value| {
                        value.id = remap_id(state, value.id);
                        value
                    });
                    let views = remap_invoke_capture(&value.views, |id| remap_id(state, id));
                    state.record_constructed_field(holder, *key, &values, &views, false);
                } else {
                    let mut value = value.clone();
                    value.remap_allocations(|id| remap_id(state, id));
                    state.record_constructed_value(holder, *key, &value, false);
                }
            }
            Self::Remove {
                holders,
                map,
                result,
            } => {
                let holders = remap_invoke_capture(holders, |id| remap_id(state, id));
                return state.remove_alloc_element(&holders, *map, result.and_then(index_of));
            }
            Self::Bind { local, value } => {
                let mut value = value.clone();
                if rebase.is_some() {
                    value.remap_allocations(|id| remap_id(state, id));
                }
                return Self::publish_bound_value(index_of, state, *local, value);
            }
            Self::ViewMember { view, id } => {
                state.add_view_member(remap_id(state, *view), remap_id(state, *id));
            }
            Self::ViewField { holder, key, view } => {
                state.link_view(remap_id(state, *holder), *key, remap_id(state, *view));
            }
            Self::ViewPair {
                left,
                right,
                may_share,
            } => {
                state.relate_views(remap_id(state, *left), remap_id(state, *right), *may_share);
            }
        }
        StoreReceipt::default()
    }
}

impl InvokeLayerOp {
    /// Carry owned captures across one result rebind before publishing any of their effects.
    pub(super) fn remap(&mut self, state: &BorrowState, retirement: AllocationRetirement) {
        match self {
            Self::Store(store) => store.remap(state, retirement),
            Self::Contribute { holder, value, .. } => {
                retirement.remap_id(state, holder);
                retirement.remap_value(state, value);
            }
            Self::ConstructField { holder, value, .. } => {
                retirement.remap_id(state, holder);
                value.remap(state, retirement);
            }
            Self::Remove { holders, .. } => {
                for holder in holders {
                    retirement.remap_id(state, holder);
                }
            }
            Self::Bind { value, .. } => value.remap(state, retirement),
            Self::ViewMember { view, id } => {
                retirement.remap_id(state, view);
                retirement.remap_id(state, id);
            }
            Self::ViewField { holder, view, .. } => {
                retirement.remap_id(state, holder);
                retirement.remap_id(state, view);
            }
            Self::ViewPair { left, right, .. } => {
                retirement.remap_id(state, left);
                retirement.remap_id(state, right);
            }
        }
    }
}
