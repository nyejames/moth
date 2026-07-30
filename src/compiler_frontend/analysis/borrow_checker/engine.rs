//! Borrow-checker fixed-point engine.
//!
//! WHAT: runs the per-function forward dataflow analysis, state joins, and fact persistence.
//! WHY: separating the algorithm from the module seam keeps `mod.rs` a structural map and makes
//!      the engine independently readable.

use crate::borrow_log;
use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckError;
use crate::compiler_frontend::analysis::borrow_checker::diagnostics::BorrowDiagnostics;
use crate::compiler_frontend::analysis::borrow_checker::state::{
    BorrowState, FunctionLayout, LocalState,
};
use crate::compiler_frontend::analysis::borrow_checker::transfer::{
    BlockTransferStats, BorrowTransferContext, transfer_block,
};
use crate::compiler_frontend::analysis::borrow_checker::types::{
    BorrowCheckReport, BorrowCheckStats, BorrowDropSite, BorrowDropSiteKind, FunctionBorrowSummary,
    LocalMode,
};
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::hir::functions::HirFunction;
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId, LocalId, RegionId};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::hir::utils::try_for_each_terminator_target;
use crate::compiler_frontend::public_call_summary::PublicCallSummary;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use rustc_hash::{FxHashMap, FxHashSet};
use std::collections::VecDeque;

pub(super) struct BorrowChecker<'a> {
    pub(super) module: &'a HirModule,
    pub(super) external_package_registry: &'a ExternalPackageRegistry,
    pub(super) string_table: &'a StringTable,
    pub(super) diagnostics: BorrowDiagnostics<'a>,
    // Fast ID lookups used throughout analysis.
    pub(super) block_index_by_id: FxHashMap<BlockId, usize>,
    pub(super) region_parent_by_id: FxHashMap<RegionId, Option<RegionId>>,
    // One frontend-owned local call contract used by transfer and retained for semantic consumers.
    pub(super) public_call_summaries: FxHashMap<FunctionId, PublicCallSummary>,
}

impl<'a> BorrowChecker<'a> {
    pub(super) fn new(
        module: &'a HirModule,
        external_package_registry: &'a ExternalPackageRegistry,
        string_table: &'a StringTable,
    ) -> Self {
        let block_index_by_id = module
            .blocks
            .iter()
            .enumerate()
            .map(|(index, block)| (block.id, index))
            .collect::<FxHashMap<_, _>>();

        let region_parent_by_id = module
            .regions
            .iter()
            .map(|region| (region.id(), region.parent()))
            .collect::<FxHashMap<_, _>>();

        Self {
            module,
            external_package_registry,
            string_table,
            diagnostics: BorrowDiagnostics::new(module, string_table),
            block_index_by_id,
            region_parent_by_id,
            public_call_summaries: FxHashMap::default(),
        }
    }

    pub(super) fn run(mut self) -> Result<BorrowCheckReport, BorrowCheckError> {
        // WHAT: run the module-level borrow-analysis driver in four phases:
        // metadata precomputation, per-function fixed-point transfer, summary finalization, then
        // fact/report assembly.
        // WHY: transfer needs allocation-light O(1) metadata lookups, while downstream tooling
        //      expects one consolidated report containing both facts and summary snapshots.
        self.build_public_call_summaries()?;

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
        let visible_locals_by_block =
            self.build_visibility_masks(function.id, &layout, &reachable_blocks)?;

        let transfer_context = BorrowTransferContext {
            external_package_registry: self.external_package_registry,
            public_call_summaries: &self.public_call_summaries,
            imported_call_summaries: &self.module.imported_call_summaries,
            module_private_call_summaries: &self.module.module_private_call_summaries,
            generated_call_summaries: &self.module.generated_call_summaries,
            diagnostics: BorrowDiagnostics::new(self.module, self.string_table),
        };

        let mut in_states: FxHashMap<BlockId, BorrowState> = FxHashMap::default();
        let mut out_states: FxHashMap<BlockId, BorrowState> = FxHashMap::default();

        // Entry state starts as UNINIT, with parameters immediately initialized.
        let mut initial_state = BorrowState::new_uninitialized(layout.local_count());
        for param in &function.params {
            let Some(param_index) = layout.index_of(*param) else {
                return Err(self.diagnostics.internal_error(
                    format!(
                        "Borrow checker could not map parameter '{}' into function state layout",
                        self.diagnostics.local_name(*param)
                    ),
                    self.diagnostics.function_error_location(function.id),
                ));
            };

            initial_state.initialize_parameter(param_index);
        }

        if let Some(mask) = visible_locals_by_block.get(&function.entry) {
            initial_state.kill_invisible(mask);
        }
        in_states.insert(function.entry, initial_state);

        let mut worklist = VecDeque::new();
        worklist.push_back(function.entry);

        let mut summary = FunctionBorrowSummary {
            reachable_blocks: reachable_blocks.len(),
            mutable_call_sites: 0,
            alias_heavy_blocks: Vec::new(),
            worklist_iterations: 0,
        };

        let mut alias_heavy = FxHashSet::default();

        borrow_log!(format!(
            "[Borrow] Analyzing function '{}' (entry={} blocks={})",
            self.diagnostics.function_name(function.id),
            function.entry,
            reachable_blocks.len()
        ));

        // Standard forward fixed-point iteration over reachable blocks.
        while let Some(block_id) = worklist.pop_front() {
            summary.worklist_iterations += 1;

            let Some(mut input_state) = in_states.get(&block_id).cloned() else {
                continue;
            };

            let input_state_changed = visible_locals_by_block
                .get(&block_id)
                .is_some_and(|mask| input_state.kill_invisible(mask));

            let block = self.block_by_id_or_error(block_id, function.id)?;
            let mut output_state = input_state.clone();

            if input_state_changed {
                in_states.insert(block_id, input_state);
            }

            // Transfer the block once and collect facts while the state is hot.
            let block_stats = transfer_block(&transfer_context, &layout, block, &mut output_state)?;
            self.merge_block_stats(&mut report.stats, &block_stats);
            summary.mutable_call_sites += block_stats.mutable_call_sites;

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
            for (value_id, fact) in block_stats.value_facts {
                report.analysis.value_facts.insert(value_id, fact);
            }
            for (statement_id, invalidations) in block_stats.reactive_invalidations {
                if invalidations.is_empty() {
                    report.analysis.reactive_invalidations.remove(&statement_id);
                } else {
                    report
                        .analysis
                        .reactive_invalidations
                        .insert(statement_id, invalidations);
                }
            }

            if output_state.has_any_alias_conflict() {
                alias_heavy.insert(block_id);
            }

            let changed_out = match out_states.get(&block_id) {
                Some(existing) => existing != &output_state,
                None => true,
            };

            // If output state is unchanged, successor joins cannot change either.
            if !changed_out {
                continue;
            }

            try_for_each_terminator_target(&block.terminator, |successor| {
                if !reachable_block_set.contains(&successor) {
                    return Ok::<(), BorrowCheckError>(());
                }

                let mut successor_input = output_state.clone();
                self.apply_jump_argument_transfer(
                    function.id,
                    &layout,
                    &block.terminator,
                    successor,
                    &mut successor_input,
                )?;

                // Apply lexical visibility kills before join to prevent
                // branch-local aliases from leaking into outer regions.
                if let Some(mask) = visible_locals_by_block.get(&successor) {
                    successor_input.kill_invisible(mask);
                }

                let next_state = match in_states.get(&successor) {
                    Some(existing) => {
                        report.stats.state_joins += 1;
                        existing.join(&successor_input)
                    }
                    None => successor_input,
                };

                let changed_in = match in_states.get(&successor) {
                    Some(existing) => existing != &next_state,
                    None => true,
                };

                // Revisit successors only when their input state grows.
                if changed_in {
                    in_states.insert(successor, next_state);
                    worklist.push_back(successor);
                }

                Ok::<(), BorrowCheckError>(())
            })?;

            out_states.insert(block_id, output_state);
        }

        let mut alias_heavy_blocks = alias_heavy.into_iter().collect::<Vec<_>>();
        alias_heavy_blocks.sort_by_key(|id| id.0);
        summary.alias_heavy_blocks = alias_heavy_blocks;

        // Persist snapshots for debug tooling and downstream analyses.
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

    fn finalize_public_call_summary_effects_to_fixed_point(
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
            for function_index in 0..self.module.functions.len() {
                let function = self.module.functions[function_index].clone();
                let reachable_blocks = self.collect_reachable_blocks(&function)?;
                changed |= self.finalize_public_call_summary_effects(
                    &function,
                    &reachable_blocks,
                    report,
                )?;
            }

            if !changed {
                return Ok(());
            }
        }

        Err(self.diagnostics.internal_error(
            "Borrow checker could not stabilize local mutation summaries",
            self.diagnostics.module_error_location(),
        ))
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
                HirTerminator::Return(_)
                | HirTerminator::ReturnSuccess(_)
                | HirTerminator::ReturnError(_) => {
                    block_sites.push(BorrowDropSite {
                        kind: BorrowDropSiteKind::Return,
                        locals: candidate_locals.clone(),
                    });
                }
                HirTerminator::FallibleBranch { .. } => {}
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
            return Err(self.diagnostics.internal_error(
                format!(
                    "Borrow checker could not resolve block '{}' while analyzing function '{}'",
                    block_id,
                    self.diagnostics.function_name(function_id)
                ),
                self.diagnostics.function_error_location(function_id),
            ));
        };

        Ok(&self.module.blocks[index])
    }

    fn merge_block_stats(&self, total: &mut BorrowCheckStats, block: &BlockTransferStats) {
        total.statements_analyzed += block.statements_analyzed;
        total.terminators_analyzed += block.terminators_analyzed;
        total.conflicts_checked += block.conflicts_checked;
    }

    fn apply_jump_argument_transfer(
        &self,
        function_id: FunctionId,
        layout: &FunctionLayout,
        terminator: &HirTerminator,
        successor: BlockId,
        successor_input: &mut BorrowState,
    ) -> Result<(), BorrowCheckError> {
        let HirTerminator::Jump { target, args } = terminator else {
            return Ok(());
        };

        if *target != successor || args.is_empty() {
            return Ok(());
        }

        let successor_block = self.block_by_id_or_error(successor, function_id)?;
        if args.len() > successor_block.locals.len() {
            return Err(self.diagnostics.internal_error(
                format!(
                    "Borrow checker saw jump edge into block '{}' with {} argument(s), but the block declares only {} local(s)",
                    successor,
                    args.len(),
                    successor_block.locals.len()
                ),
                self.diagnostics.function_error_location(function_id),
            ));
        }

        let source_states = args
            .iter()
            .map(|source_local| {
                let Some(source_index) = layout.index_of(*source_local) else {
                    return Err(self.diagnostics.internal_error(
                        format!(
                            "Borrow checker could not map jump argument local '{}' into function state layout",
                            self.diagnostics.local_name(*source_local)
                        ),
                        self.diagnostics.function_error_location(function_id),
                    ));
                };

                Ok(successor_input.local_state(source_index).clone())
            })
            .collect::<Result<Vec<_>, BorrowCheckError>>()?;

        let destination_indices = successor_block
            .locals
            .iter()
            .take(args.len())
            .map(|local| {
                let Some(destination_index) = layout.index_of(local.id) else {
                    return Err(self.diagnostics.internal_error(
                        format!(
                            "Borrow checker could not map jump target local '{}' into function state layout",
                            self.diagnostics.local_name(local.id)
                        ),
                        self.diagnostics.function_error_location(function_id),
                    ));
                };

                Ok(destination_index)
            })
            .collect::<Result<Vec<_>, BorrowCheckError>>()?;

        let local_count = layout.local_count();
        for (source_state, destination_index) in source_states.into_iter().zip(destination_indices)
        {
            let destination_state = successor_input.local_state(destination_index).clone();
            let destination_is_alias_only = destination_state.mode.contains(LocalMode::ALIAS)
                && !destination_state.mode.contains(LocalMode::SLOT);

            let next_state = if source_state.mode.is_definitely_uninit() {
                LocalState::uninit(local_count)
            } else if destination_is_alias_only {
                destination_state
            } else {
                LocalState::slot(local_count)
            };
            successor_input.update_local_state(destination_index, next_state);
        }

        Ok(())
    }
}
