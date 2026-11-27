//! Borrow-checker fixed-point engine.
//!
//! WHAT: runs the per-function forward dataflow analysis, state joins, and fact persistence.
//! WHY: separating the algorithm from the module seam keeps `mod.rs` a structural map and makes
//!      the engine independently readable.

use crate::borrow_log;
use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckError;
use crate::compiler_frontend::analysis::borrow_checker::diagnostics::BorrowDiagnostics;
use crate::compiler_frontend::analysis::borrow_checker::state::{
    AllocationRetirement, BorrowState, FunctionLayout, LocalState, OrderKey, RootSet,
};
use crate::compiler_frontend::analysis::borrow_checker::transfer::{
    BlockTransferStats, BorrowTransferContext, InvokeLayerOp, InvokeTransfer, transfer_block,
};
use crate::compiler_frontend::analysis::borrow_checker::types::{
    BorrowCheckReport, BorrowCheckStats, BorrowDropSite, BorrowDropSiteKind, FunctionBorrowSummary,
    LocalMode, ValueBorrowFact,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::hir::functions::HirFunction;
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId, HirValueId, LocalId, RegionId};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::hir::utils::try_for_each_terminator_target;
use crate::compiler_frontend::public_call_summary::PublicCallSummary;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use rustc_hash::{FxHashMap, FxHashSet};
use std::borrow::Cow;
use std::collections::{VecDeque, hash_map::Entry};

#[cfg(test)]
#[path = "transfer/tests/invoke_edge_tests.rs"]
mod invoke_edge_tests;

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

    fn analyze_function(
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
        let reachable_block_set = reachable_blocks.iter().copied().collect::<FxHashSet<_>>();
        let layout = self.build_function_layout(function, &reachable_blocks)?;
        let visible_locals_by_block = &layout.visible_locals_by_block;

        let transfer_context = BorrowTransferContext {
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
        };

        let mut in_states: FxHashMap<BlockId, BorrowState> = FxHashMap::default();
        let mut out_states: FxHashMap<BlockId, BorrowState> = FxHashMap::default();
        let mut invoke_by_block = FxHashMap::<BlockId, InvokeTransfer>::default();
        // HIR expression rows are immutable and may be shared across blocks. Keep only the
        // latest facts for each block during fixed-point iteration, then merge those surviving
        // snapshots after the state has stabilized.
        let mut final_value_facts_by_block: FxHashMap<BlockId, Vec<(HirValueId, ValueBorrowFact)>> =
            FxHashMap::default();

        // Entry state starts as UNINIT, with parameters immediately initialized.
        let mut initial_state = BorrowState::new_uninitialized(layout.local_count());
        for param in &function.params {
            let Some(param_index) = layout.index_of(*param) else {
                return Err(self.diagnostics.internal_error(format!(
                    "Borrow checker could not map parameter '{}' into function state layout",
                    self.diagnostics.local_name(*param)
                )));
            };

            initial_state.initialize_parameter(param_index);
        }

        if let Some(mask) = visible_locals_by_block.get(&function.entry) {
            initial_state.kill_invisible(mask);
        }
        in_states.insert(function.entry, initial_state);

        // `pending` mirrors queue membership so a block whose input grows from several
        // predecessors before it runs is visited once with the accumulated input.
        let mut worklist = VecDeque::new();
        let mut pending = FxHashSet::default();
        worklist.push_back(function.entry);
        pending.insert(function.entry);

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

        // Standard forward fixed-point iteration over reachable blocks.
        while let Some(block_id) = worklist.pop_front() {
            // Clear before transfer so a self-loop or later back edge can queue the block again.
            pending.remove(&block_id);
            summary.worklist_iterations += 1;

            // Stored inputs are already narrowed to this block's visibility: the entry state and
            // every incoming edge are narrowed before insertion or join, and joining states
            // narrowed to the same mask stays narrowed.
            let Some(input_state) = in_states.get(&block_id) else {
                continue;
            };
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
            let invoke = block_stats.invoke;
            let previous_invoke = invoke_by_block.get(&block_id);
            let has_invoke_effects = !invoke.is_empty();
            let changed_out = out_states.get(&block_id) != Some(&output_state)
                || previous_invoke.map_or(has_invoke_effects, |previous| previous != &invoke);

            // If output state is unchanged, successor joins cannot change either.
            if !changed_out {
                continue;
            }

            // The target visitor enumerates success before error, even when both share a block.
            let mut invoke_lanes = [&invoke.success, &invoke.error].into_iter();

            try_for_each_terminator_target(&block.terminator, |successor| {
                if block_stats.diverges {
                    return Ok::<(), BorrowCheckError>(());
                }
                let invoke_lane = invoke_lanes.next();
                if invoke_lane.is_some_and(|lane| lane.unreachable)
                    || !reachable_block_set.contains(&successor)
                {
                    return Ok::<(), BorrowCheckError>(());
                }

                let mut successor_input = output_state.clone();
                if let Some(lane) = invoke_lane {
                    apply_invoke_edge_transfer(
                        &layout,
                        &block.terminator,
                        lane,
                        layout.terminator_order_or_unknown(block_id),
                        &mut successor_input,
                    );
                }
                self.apply_jump_argument_transfer(
                    function.id,
                    &layout,
                    &block.terminator,
                    successor,
                    layout.terminator_order_or_unknown(block_id),
                    &mut successor_input,
                )?;

                // Apply lexical visibility kills before join to prevent
                // branch-local aliases from leaking into outer regions.
                if let Some(mask) = visible_locals_by_block.get(&successor) {
                    successor_input.kill_invisible(mask);
                }

                // Revisit successors only when their input state grows.
                let changed_in = match in_states.get_mut(&successor) {
                    Some(existing) => {
                        report.stats.state_joins += 1;
                        existing.join_in_place(&successor_input)
                    }
                    None => {
                        in_states.insert(successor, successor_input);
                        true
                    }
                };
                if changed_in && pending.insert(successor) {
                    worklist.push_back(successor);
                }

                Ok::<(), BorrowCheckError>(())
            })?;
            drop(invoke_lanes);

            out_states.insert(block_id, output_state);
            if has_invoke_effects {
                invoke_by_block.insert(block_id, invoke);
            } else {
                invoke_by_block.remove(&block_id);
            }
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
            let local_state = state.local_state(index);
            if local_state.mode.contains(LocalMode::SLOT) {
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

    pub(super) fn apply_jump_argument_transfer(
        &self,
        function_id: FunctionId,
        layout: &FunctionLayout,
        terminator: &HirTerminator,
        successor: BlockId,
        order: OrderKey,
        successor_input: &mut BorrowState,
    ) -> Result<(), BorrowCheckError> {
        let HirTerminator::Jump { target, args } = terminator else {
            return Ok(());
        };

        if *target != successor || args.is_empty() {
            return Ok(());
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

                let source_state = successor_input.local_state(source_index);
                let layer_ids = successor_input.has_layer_binding(source_index).then(|| {
                    (
                        successor_input.local_alloc_ids(source_index).0,
                        successor_input.alloc_binding_readonly(source_index),
                        successor_input.alloc_binding_views(source_index).collect::<Vec<_>>(),
                    )
                });
                let detached = successor_input.detached_holders(source_index).collect::<Vec<_>>();
                let destination_state = if source_state.mode.is_definitely_uninit() {
                    LocalState::uninit(layout.local_count())
                } else {
                    // Jump arguments enter the destination's binding slot. Preserve the source
                    // value's effective allocation provenance, including a plain source slot's
                    // own root, without turning the destination into a write-through alias view.
                    // Cell identity belongs to the source binding; a fresh jump destination is a
                    // new binding, so it must not inherit the source's direct alias cells or write
                    // targets. The derivation ledger travels with the transported value: a read
                    // through the joined destination still derives from the exclusive alias the
                    // branch produced it from. The join intersects it, so a path that did not
                    // derive loses the exemption.
                    let mut destination = LocalState::slot_with_value_roots(
                        successor_input.effective_roots(source_index),
                        RootSet::empty(layout.local_count()),
                    );
                    destination.derived_from_roots =
                        successor_input.derived_roots_from_source(source_index);
                    destination
                };

                Ok((source_index, destination_state, layer_ids, detached))
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
        for (source, destination_state, layer_ids, _) in &mut destination_states {
            if layer_ids.is_none() && destination_state.value_roots.iter_ones().any(retires) {
                // A root-only source also needs an override when the edge overwrites its root.
                *layer_ids = Some((
                    successor_input.local_alloc_ids(*source).0,
                    false,
                    Vec::new(),
                ));
            }
        }

        if !successor_input.layer_is_empty()
            || destination_states
                .iter()
                .any(|(_, _, ids, detached)| ids.is_some() || !detached.is_empty())
        {
            for &index in &destination_indices {
                successor_input.retire_binding_allocation(index, order);
                successor_input.update_local_state(index, LocalState::uninit(layout.local_count()));
            }
        }

        // No destination is installed until all sources and their detachments have been captured
        // and every old destination has been retired.
        for ((_, destination_state, layer_ids, mut detached), &destination_index) in
            destination_states.into_iter().zip(&destination_indices)
        {
            successor_input.update_local_state(destination_index, destination_state);
            let remap = |id| successor_input.remap_retired_alloc_id(id, order, retires);
            for holder in &mut detached {
                *holder = remap(*holder);
            }
            if let Some((mut ids, readonly, mut views)) = layer_ids {
                for id in ids.iter_mut().chain(&mut views) {
                    *id = remap(*id);
                }
                successor_input.bind_alloc_ids(destination_index, &ids, readonly);
                if !views.is_empty() {
                    successor_input.bind_alloc_views(destination_index, &views);
                }
            }
            if !detached.is_empty() {
                successor_input.bind_detached_holders(destination_index, detached);
            }
        }

        Ok(())
    }
}

/// Retire both previous outcomes before installing the selected lane's captures.
fn apply_invoke_edge_transfer(
    layout: &FunctionLayout,
    terminator: &HirTerminator,
    lane: &super::transfer::InvokeLaneTransfer,
    order: OrderKey,
    edge_state: &mut BorrowState,
) {
    let HirTerminator::Invoke {
        success_results,
        error_result,
        ..
    } = terminator
    else {
        return;
    };
    let apply_layer_effects = !lane.operations.is_empty();
    for local in success_results.iter().chain(std::iter::once(error_result)) {
        if let Some(index) = layout.index_of(*local) {
            if apply_layer_effects {
                // Captures can create the first sparse facts on this edge. Still snapshot any
                // existing root-only observers before changing the result binding.
                edge_state.retire_binding_allocation(index, order);
                edge_state.update_local_state(index, LocalState::uninit(layout.local_count()));
            } else {
                edge_state.rebind_local(index, LocalState::uninit(layout.local_count()), order);
            }
        }
    }
    for (local, result) in &lane.results {
        if let Some(index) = layout.index_of(*local) {
            edge_state.update_local_state(index, result.clone());
        }
    }
    let retires = |index| {
        let local = layout.local_ids[index];
        local == *error_result || success_results.contains(&local)
    };
    for op in &lane.operations {
        op.apply(layout, edge_state, Some((order, &retires)));
    }
}

impl InvokeLayerOp {
    pub(super) fn apply(
        &self,
        layout: &FunctionLayout,
        state: &mut BorrowState,
        rebase: Option<(OrderKey, &dyn Fn(usize) -> bool)>,
    ) {
        let remap_id = |state: &BorrowState, id| match rebase {
            Some((order, retires)) => state.remap_retired_alloc_id(id, order, retires),
            None => id,
        };
        match self {
            Self::Store {
                holders,
                key,
                values,
                views,
                distinct_occurrence,
            } => {
                let holders = remap_invoke_capture(holders, |id| remap_id(state, id));
                let values = remap_invoke_capture(values, |mut value| {
                    value.id = remap_id(state, value.id);
                    value
                });
                let views = remap_invoke_capture(views, |id| remap_id(state, id));
                state.store_alloc_field_with_views(
                    &holders,
                    *key,
                    &values,
                    &views,
                    false,
                    *distinct_occurrence,
                );
            }
            Self::StoreCells {
                slots,
                fields,
                values,
                views,
            } => {
                let fields =
                    remap_invoke_capture(fields, |(holder, key)| (remap_id(state, holder), key));
                let values = remap_invoke_capture(values, |mut value| {
                    value.id = remap_id(state, value.id);
                    value
                });
                let views = remap_invoke_capture(views, |id| remap_id(state, id));
                // A result overwrites a captured slot, but not fields in the old value
                // that occupied it; those holders have been rebased above.
                state.store_call_cells(slots, &fields, &values, &views, |local| {
                    rebase.is_some_and(|(_, retires)| retires(local))
                });
            }
            Self::Contribute { holder, key, value } => {
                let holder = remap_id(state, *holder);
                let mut value = *value;
                value.id = remap_id(state, value.id);
                state.contribute_alloc_edge(holder, *key, value);
            }
            Self::ConstructField {
                holder,
                key,
                values,
                views,
            } => {
                let holder = remap_id(state, *holder);
                let values = remap_invoke_capture(values, |mut value| {
                    value.id = remap_id(state, value.id);
                    value
                });
                let views = remap_invoke_capture(views, |id| remap_id(state, id));
                state.record_constructed_field(holder, *key, &values, &views, false);
            }
            Self::Remove {
                holders,
                map,
                result,
            } => {
                let holders = remap_invoke_capture(holders, |id| remap_id(state, id));
                state.remove_alloc_element(
                    &holders,
                    *map,
                    result.and_then(|local| layout.index_of(local)),
                );
            }
            Self::Bind {
                local,
                ids,
                views,
                readonly,
            } => {
                if let Some(index) = layout.index_of(*local) {
                    let ids = remap_invoke_capture(ids, |id| remap_id(state, id));
                    let views = remap_invoke_capture(views, |id| remap_id(state, id));
                    super::transfer::bind_call_result_ids(state, index, &ids, &views, *readonly);
                }
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
    }
}

impl InvokeLayerOp {
    /// Carry owned captures across one result rebind before publishing any of their effects.
    pub(super) fn remap(&mut self, state: &BorrowState, retirement: AllocationRetirement) {
        match self {
            Self::Store {
                holders,
                values,
                views,
                ..
            } => {
                for holder in holders.iter_mut().chain(views) {
                    retirement.remap_id(state, holder);
                }
                for value in values {
                    retirement.remap_value(state, value);
                }
            }
            Self::StoreCells {
                slots,
                fields,
                values,
                views,
            } => {
                slots.retain(|&slot| !retirement.retires_local(slot));
                for (holder, _) in fields {
                    retirement.remap_id(state, holder);
                }
                for value in values {
                    retirement.remap_value(state, value);
                }
                for view in views {
                    retirement.remap_id(state, view);
                }
            }
            Self::Contribute { holder, value, .. } => {
                retirement.remap_id(state, holder);
                retirement.remap_value(state, value);
            }
            Self::ConstructField {
                holder,
                values,
                views,
                ..
            } => {
                retirement.remap_id(state, holder);
                for value in values {
                    retirement.remap_value(state, value);
                }
                for view in views {
                    retirement.remap_id(state, view);
                }
            }
            Self::Remove { holders, .. } => {
                for holder in holders {
                    retirement.remap_id(state, holder);
                }
            }
            Self::Bind { ids, views, .. } => {
                for holder in ids.iter_mut().chain(views) {
                    retirement.remap_id(state, holder);
                }
            }
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

/// Keep captures borrowed unless a reused outcome changes at least one of their identities.
fn remap_invoke_capture<T: Copy + PartialEq>(
    capture: &[T],
    remap: impl Fn(T) -> T,
) -> Cow<'_, [T]> {
    for (index, &original) in capture.iter().enumerate() {
        let rebased = remap(original);
        if rebased != original {
            let mut result = Vec::with_capacity(capture.len());
            result.extend_from_slice(&capture[..index]);
            result.push(rebased);
            result.extend(capture[index + 1..].iter().copied().map(remap));
            return Cow::Owned(result);
        }
    }
    Cow::Borrowed(capture)
}
