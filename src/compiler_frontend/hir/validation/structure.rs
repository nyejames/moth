//! Structural graph and function-origin validation for HIR.
//!
//! WHAT: checks region parents, entry function metadata, function origins, and CFG ownership.
//! WHY: borrow validation and backend lowering both assume every block belongs to exactly one
//! function and that semantic function origins are complete.

use super::HirValidator;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::hir::functions::HirFunctionOrigin;
use crate::compiler_frontend::hir::hir_side_table::HirLocation;
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId, HirNodeId};
use crate::compiler_frontend::hir::terminators::{HirJumpArgument, HirTerminator};
use crate::compiler_frontend::hir::utils::terminator_targets;
use rustc_hash::{FxHashMap, FxHashSet};
use std::collections::VecDeque;

impl<'a> HirValidator<'a> {
    // -------------------------
    //  Structural Validation
    // -------------------------

    pub(super) fn validate_region_graph(&self) -> Result<(), CompilerError> {
        let parent_by_region = self
            .module
            .regions
            .iter()
            .map(|region| (region.id(), region.parent()))
            .collect::<FxHashMap<_, _>>();

        for region in &self.module.regions {
            if let Some(parent) = region.parent()
                && !self.region_ids.contains(&parent)
            {
                return Err(self.error_with_hir(
                    format!(
                        "Region {} references missing parent region {}",
                        region.id().0,
                        parent.0
                    ),
                    None,
                ));
            }
        }

        for region in &self.module.regions {
            let mut chain = FxHashSet::default();
            let mut current = Some(region.id());

            while let Some(region_id) = current {
                if !chain.insert(region_id) {
                    return Err(self.error_with_hir(
                        format!(
                            "Region parent graph contains a cycle at region {}",
                            region_id.0
                        ),
                        None,
                    ));
                }

                current = parent_by_region.get(&region_id).copied().flatten();
            }
        }

        Ok(())
    }

    pub(super) fn validate_start_function(&self) -> Result<(), CompilerError> {
        if let Some(start_function) = self.module.start_function
            && !self.function_ids.contains(&start_function)
        {
            return Err(self.error_with_hir(
                format!(
                    "HIR start_function {:?} is not present in module functions",
                    start_function
                ),
                Some(HirLocation::Function(start_function)),
            ));
        }

        Ok(())
    }

    pub(super) fn validate_function_origins(&self) -> Result<(), CompilerError> {
        // WHAT: enforce complete and consistent function-origin coverage.
        // WHY: backends rely on this map to preserve entry/runtime semantics.
        if self.module.function_origins.len() != self.module.functions.len() {
            return Err(self.error_with_hir(
                format!(
                    "HIR function_origins contains {} entries, but module has {} functions",
                    self.module.function_origins.len(),
                    self.module.functions.len()
                ),
                None,
            ));
        }

        for function in &self.module.functions {
            if !self.module.function_origins.contains_key(&function.id) {
                return Err(self.error_with_hir(
                    format!("HIR function {:?} is missing an origin entry", function.id),
                    Some(HirLocation::Function(function.id)),
                ));
            }
        }

        if let Some(start_function) = self.module.start_function {
            if !matches!(
                self.module.function_origins.get(&start_function),
                Some(HirFunctionOrigin::EntryStart)
            ) {
                return Err(self.error_with_hir(
                    format!(
                        "HIR start function {:?} must be tagged as EntryStart",
                        start_function
                    ),
                    Some(HirLocation::Function(start_function)),
                ));
            }
        } else if self
            .module
            .function_origins
            .values()
            .any(|origin| matches!(origin, HirFunctionOrigin::EntryStart))
        {
            return Err(self.error_with_hir(
                "HIR module without start_function contains an EntryStart origin".to_owned(),
                None,
            ));
        }

        // Enforce that the stable origin-to-local-function relation is injective in both
        // directions. The lowering owner already rejects one origin mapped to two local
        // functions, so this loop independently catches the reverse direction (two origins
        // mapped to one local function) against malformed HIR inputs rather than relying only
        // on construction-time map key uniqueness.
        let mut identity_for_local_function: FxHashMap<FunctionId, String> = FxHashMap::default();
        for (identity, function_id) in self
            .module
            .function_ids_by_origin
            .iter()
            .map(|(origin, function_id)| (format!("{origin:?}"), function_id))
            .chain(
                self.module
                    .function_ids_by_private_origin
                    .iter()
                    .map(|(identity, function_id)| (format!("{identity:?}"), function_id)),
            )
        {
            if Some(*function_id) == self.module.start_function {
                return Err(self.error_with_hir(
                    format!(
                        "HIR implicit start function {:?} must not carry a stable executable identity {}",
                        function_id, identity
                    ),
                    Some(HirLocation::Function(*function_id)),
                ));
            }

            if !self.function_ids.contains(function_id) {
                return Err(self.error_with_hir(
                    format!(
                        "HIR stable executable identity {} references missing function {:?}",
                        identity, function_id
                    ),
                    Some(HirLocation::Function(*function_id)),
                ));
            }

            if let Some(existing_identity) = identity_for_local_function.get(function_id) {
                return Err(self.error_with_hir(
                    format!(
                        "HIR stable executable identities {} and {} both map to local function {:?}",
                        existing_identity, identity, function_id
                    ),
                    Some(HirLocation::Function(*function_id)),
                ));
            }

            identity_for_local_function.insert(*function_id, identity);
        }

        Ok(())
    }

    pub(super) fn validate_function_cfg_ownership(&mut self) -> Result<(), CompilerError> {
        // WHAT: ensure every block belongs to exactly one function CFG.
        // WHY: prevents cross-function jumps and ensures clear ownership for analysis.
        self.block_owner_by_id.clear();

        // Pending catch edges: a recorded scalar call still falls through until the
        // private-lane installer routes its failure edge, so each record counts as a
        // pending edge from the recorded statement's block to its handler block. The
        // installer drains every record, so post-install validation sees none of these.
        let mut statement_block_by_id: FxHashMap<HirNodeId, BlockId> = FxHashMap::default();
        for block in &self.module.blocks {
            for statement in &block.statements {
                statement_block_by_id.insert(statement.id, block.id);
            }
        }
        let mut pending_handlers: FxHashMap<(FunctionId, BlockId), Vec<BlockId>> =
            FxHashMap::default();
        for record in &self.module.catch_protected_calls {
            let Some(statement_block) = statement_block_by_id.get(&record.statement).copied()
            else {
                return Err(self.error_with_hir(
                    format!(
                        "Catch-protected call record references missing statement {:?}",
                        record.statement
                    ),
                    None,
                ));
            };
            pending_handlers
                .entry((record.owner, statement_block))
                .or_default()
                .push(record.handler.block);
        }

        for function in &self.module.functions {
            let mut queue = VecDeque::new();
            let mut visited = FxHashSet::default();
            queue.push_back(function.entry);

            while let Some(block_id) = queue.pop_front() {
                if !visited.insert(block_id) {
                    continue;
                }

                self.require_block_id(block_id, Some(HirLocation::Function(function.id)))?;

                if let Some(existing_owner) = self.block_owner_by_id.get(&block_id).copied() {
                    if existing_owner != function.id {
                        return Err(self.error_with_hir(
                            format!(
                                "Block {} is reachable from multiple functions ({:?} and {:?})",
                                block_id, existing_owner, function.id
                            ),
                            Some(HirLocation::Block(block_id)),
                        ));
                    }
                } else {
                    self.block_owner_by_id.insert(block_id, function.id);
                }

                let block = self.block_by_id(block_id)?;
                for successor in terminator_targets(&block.terminator) {
                    queue.push_back(successor);
                }
                if let Some(handlers) = pending_handlers.get(&(function.id, block_id)) {
                    for handler in handlers {
                        queue.push_back(*handler);
                    }
                }
            }
        }

        for block in &self.module.blocks {
            if self.block_owner_by_id.contains_key(&block.id) {
                continue;
            }

            return Err(self.error_with_hir(
                format!(
                    "Block {} is not reachable from any function entry and has no CFG owner",
                    block.id
                ),
                Some(HirLocation::Block(block.id)),
            ));
        }

        // A pending edge never crosses a function boundary: the recorded statement block
        // and its handler block both belong to the recorded owner.
        for record in &self.module.catch_protected_calls {
            let Some(statement_block) = statement_block_by_id.get(&record.statement).copied()
            else {
                continue;
            };
            let statement_owner = self.block_owner_by_id.get(&statement_block).copied();
            let handler_owner = self.block_owner_by_id.get(&record.handler.block).copied();
            if statement_owner != Some(record.owner) || handler_owner != Some(record.owner) {
                return Err(self.error_with_hir(
                    format!(
                        "Catch-protected call record for statement {:?} crosses a function boundary",
                        record.statement
                    ),
                    Some(HirLocation::Block(record.handler.block)),
                ));
            }
        }

        self.validate_incoming_edge_destinations()?;

        Ok(())
    }

    fn validate_incoming_edge_destinations(&self) -> Result<(), CompilerError> {
        // The existing HIR edge contract names destinations on Jump edges. All other
        // terminators transfer zero local values, so every predecessor of a block must agree
        // on the same destination set regardless of edge kind or argument ordering.
        let mut incoming_by_target: FxHashMap<BlockId, (BlockId, &[HirJumpArgument])> =
            FxHashMap::default();

        for source in &self.module.blocks {
            let arguments = match &source.terminator {
                HirTerminator::Jump { args, .. } => args.as_slice(),
                _ => &[],
            };

            for target in terminator_targets(&source.terminator) {
                if let Some((first_source, first_arguments)) = incoming_by_target.get(&target) {
                    // Destination uniqueness is checked on each Jump. Compare the borrowed
                    // pairs without allocating another destination collection for every edge.
                    let same_destinations = first_arguments.len() == arguments.len()
                        && arguments.iter().all(|argument| {
                            first_arguments
                                .iter()
                                .any(|first| first.destination == argument.destination)
                        });
                    if !same_destinations {
                        return Err(self.error_with_hir(
                            format!(
                                "Incoming CFG edges to block {target} disagree on explicit destination set or arity: blocks {first_source} and {}",
                                source.id
                            ),
                            Some(HirLocation::Block(source.id)),
                        ));
                    }
                } else {
                    incoming_by_target.insert(target, (source.id, arguments));
                }
            }
        }

        Ok(())
    }

    pub(super) fn validate_function_provenance(&self) -> Result<(), CompilerError> {
        // WHAT: enforce exactly one direct synthetic-interface provenance fact per local function.
        // WHY: the per-function link-fact lane requires complete, in-range coverage. Missing,
        // extra or out-of-range provenance is an internal `CompilerError` because the fact is
        // compiler-owned metadata, not user-facing source state.
        if self.module.function_provenance.len() != self.module.functions.len() {
            return Err(self.error_with_hir(
                format!(
                    "HIR function_provenance contains {} entries, but module has {} functions",
                    self.module.function_provenance.len(),
                    self.module.functions.len()
                ),
                None,
            ));
        }

        for function in &self.module.functions {
            if !self.module.function_provenance.contains_key(&function.id) {
                return Err(self.error_with_hir(
                    format!(
                        "HIR function {:?} is missing a function_provenance synthetic-interface fact",
                        function.id
                    ),
                    Some(HirLocation::Function(function.id)),
                ));
            }
        }

        Ok(())
    }
}
