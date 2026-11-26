//! Return-alias summary classification.
//!
//! WHAT: classifies the provenance of function return values and projects callee summaries
//!       through call arguments.
//! WHY: keeping this recursive classifier separate from metadata construction leaves the metadata
//!      module focused on retained layouts and effects.

use super::BorrowChecker;
use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckError;
use crate::compiler_frontend::external_packages::{CallTarget, ExternalHirReturnAlias};
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, HirMapOp, ValueKind};
use crate::compiler_frontend::hir::functions::HirFunction;
use crate::compiler_frontend::hir::hir_datatypes::classify_hir_type;
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId, HirValueId, LocalId};
use crate::compiler_frontend::hir::statements::{HirStatementKind, HirWriteTarget};
use crate::compiler_frontend::hir::terminators::{HirFallibleOperation, HirTerminator};
use crate::compiler_frontend::public_call_summary::FunctionReturnAliasSummary;
use rustc_hash::{FxHashMap, FxHashSet};

#[path = "return_alias/result_paths.rs"]
mod result_paths;

pub(in crate::compiler_frontend::analysis::borrow_checker) use result_paths::CallResultFacts;

/// Immutable inputs for projecting one callee return-alias summary through a caller.
struct AliasProjectionContext<'a> {
    function: &'a HirFunction,
    return_alias: &'a FunctionReturnAliasSummary,
    args: &'a [HirValueId],
    param_index_by_local: &'a FxHashMap<LocalId, usize>,
    reachable_blocks: &'a [BlockId],
    callee_description: &'a str,
}

/// Flow-insensitive may-sharing between locals. A load or transport can share storage in
/// either direction. A constructor's child does not in turn own the containing allocation.
#[derive(Default)]
struct ReturnedStorage {
    linked: FxHashMap<LocalId, Vec<(LocalId, bool)>>,
    roots: Vec<LocalId>,
    alias_writers: FxHashSet<LocalId>,
    owned: FxHashMap<LocalId, bool>,
    values: Vec<HirValueId>,
    visited_values: FxHashSet<HirValueId>,
}

impl ReturnedStorage {
    fn link(&mut self, holder: LocalId, child: LocalId, may_alias: bool) {
        if holder != child {
            self.linked.entry(holder).or_default().push((child, false));
            self.linked
                .entry(child)
                .or_default()
                .push((holder, !may_alias));
        }
    }

    fn record_writer(&mut self, local: LocalId, owned: bool) {
        self.owned
            .entry(local)
            .and_modify(|all_owned| *all_owned &= owned)
            .or_insert(owned);
    }

    fn may_write_through_descendant(&self, local: LocalId, projections: usize) -> bool {
        projections != 1 || self.owned.get(&local) != Some(&true)
    }

    fn retain_expression(
        &mut self,
        checker: &BorrowChecker<'_>,
        value: HirValueId,
        holder: Option<LocalId>,
        may_alias: bool,
    ) -> Result<(), BorrowCheckError> {
        let expressions = &checker.module.expressions;
        self.visited_values.clear();
        self.values.push(value);
        while let Some(value) = self.values.pop() {
            if !self.visited_values.insert(value) {
                continue;
            }
            let expression = expressions.expression(value);
            if !classify_hir_type(expression.ty, checker.type_environment)?
                .has_in_place_mutation_surface()
            {
                continue;
            }
            match &expression.kind {
                HirExpressionKind::Load(place) => {
                    if let Some(holder) = holder {
                        self.link(holder, place.root, may_alias);
                    } else {
                        self.roots.push(place.root);
                    }
                }
                HirExpressionKind::VariantPayloadGet { source, .. } => self.values.push(*source),
                kind if kind.is_aggregate_construction() => {
                    self.values.extend(expressions.child_values(value));
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// `true` means reachable without walking from a child up into a containing holder.
    /// Nested stores and stores through an unproven view may touch a shared descendant.
    fn reachable(&mut self, include_containing: bool) -> FxHashMap<LocalId, bool> {
        use std::collections::hash_map::Entry;

        let mut reachable = FxHashMap::default();
        let mut containing = Vec::new();
        while let Some(local) = self.roots.pop() {
            if let Entry::Vacant(entry) = reachable.entry(local) {
                entry.insert(true);
                if let Some(neighbors) = self.linked.remove(&local) {
                    for (neighbor, parent) in neighbors {
                        if !parent {
                            self.roots.push(neighbor);
                        } else if include_containing {
                            containing.push(neighbor);
                        }
                    }
                }
            }
        }
        while let Some(local) = containing.pop() {
            if let Entry::Vacant(entry) = reachable.entry(local) {
                entry.insert(false);
                if let Some(neighbors) = self.linked.remove(&local) {
                    containing.extend(neighbors.into_iter().map(|(neighbor, _)| neighbor));
                }
            }
        }
        reachable
    }
}

impl<'a> BorrowChecker<'a> {
    pub(super) fn classify_function_return_alias(
        &self,
        function: &HirFunction,
    ) -> Result<FunctionReturnAliasSummary, BorrowCheckError> {
        // Summary lattice:
        // Fresh < AliasParams(bitset) < Unknown
        // If any reachable return shape is ambiguous, we escalate to Unknown.
        let reachable_blocks = self.collect_reachable_blocks(function)?;
        let mut param_index_by_local = FxHashMap::default();
        for (param_index, param_local) in function.params.iter().enumerate() {
            param_index_by_local.insert(*param_local, param_index);
        }

        let mut summary = FunctionReturnAliasSummary::Fresh;
        let mut saw_return = false;

        for &block_id in &reachable_blocks {
            let block = self.block_by_id_or_error(block_id, function.id)?;
            let HirTerminator::Return(values) = &block.terminator else {
                continue;
            };
            saw_return = true;
            // Separate success positions never imply separate allocation origins.
            for value in self.module.expressions.values(*values) {
                let return_summary = self.classify_return_expression(
                    function,
                    &reachable_blocks,
                    *value,
                    &param_index_by_local,
                )?;
                summary = merge_return_alias(summary, return_summary);
            }

            if matches!(summary, FunctionReturnAliasSummary::Unknown) {
                // Unknown is the lattice top; no additional scanning can improve it.
                break;
            }
        }

        if !saw_return {
            return Ok(FunctionReturnAliasSummary::Unknown);
        }

        if !function.params.is_empty() && !matches!(summary, FunctionReturnAliasSummary::Unknown) {
            let success_types = function
                .return_contract
                .ordinary()
                .map(|shape| shape.success());
            for &ty in success_types.into_iter().flatten() {
                if classify_hir_type(ty, self.type_environment)?.has_in_place_mutation_surface() {
                    return self.join_projected_store_aliases(
                        function,
                        &reachable_blocks,
                        &param_index_by_local,
                        summary,
                    );
                }
            }
        }
        Ok(summary)
    }

    /// A heap store can retain an argument in the result, including through an alias of a field.
    /// Only join stores whose targets could overlap storage in a return.
    fn join_projected_store_aliases(
        &self,
        function: &HirFunction,
        reachable_blocks: &[BlockId],
        param_index_by_local: &FxHashMap<LocalId, usize>,
        mut summary: FunctionReturnAliasSummary,
    ) -> Result<FunctionReturnAliasSummary, BorrowCheckError> {
        let mut stores = Vec::new();
        for &block_id in reachable_blocks {
            let block = self.block_by_id_or_error(block_id, function.id)?;
            for statement in &block.statements {
                let HirStatementKind::Write {
                    target: HirWriteTarget::AssignPlace(place),
                    value,
                } = &statement.kind
                else {
                    continue;
                };
                if !classify_hir_type(
                    self.module.expressions.expression(*value).ty,
                    self.type_environment,
                )?
                .has_in_place_mutation_surface()
                {
                    continue;
                }
                stores.push((place.root, place.projections.len(), *value));
            }
        }
        if stores.is_empty() {
            return Ok(summary);
        }

        let mut storage = ReturnedStorage::default();
        let expressions = &self.module.expressions;
        for &block_id in reachable_blocks {
            let block = self.block_by_id_or_error(block_id, function.id)?;
            for statement in &block.statements {
                match &statement.kind {
                    HirStatementKind::Write { target, value } => {
                        let (local, whole) = match target {
                            HirWriteTarget::DefineLocal(local) => (*local, true),
                            HirWriteTarget::AssignPlace(place) => {
                                (place.root, place.projections.is_empty())
                            }
                        };
                        let expression = expressions.expression(*value);
                        let load = whole && matches!(expression.kind, HirExpressionKind::Load(_));
                        if load && expression.value_kind == ValueKind::Place {
                            storage.alias_writers.insert(local);
                        }
                        if whole && self.storage_local_is_heap(local)? {
                            storage.record_writer(
                                local,
                                matches!(target, HirWriteTarget::DefineLocal(_))
                                    && expression.kind.is_aggregate_construction(),
                            );
                        }
                        storage.retain_expression(self, *value, Some(local), load)?;
                    }
                    HirStatementKind::Call {
                        target,
                        args,
                        results,
                    } => self.link_call_results(
                        &mut storage,
                        target,
                        expressions.values(*args),
                        results.iter().map(|result| result.local()),
                    )?,
                    HirStatementKind::MapOp {
                        op: HirMapOp::Get | HirMapOp::Remove,
                        receiver,
                        result: Some(result),
                        ..
                    } => self.link_extracted_storage(&mut storage, *receiver, result.local())?,
                    _ => {}
                }
            }
            match &block.terminator {
                HirTerminator::Return(values) => {
                    for &value in expressions.values(*values) {
                        storage.retain_expression(self, value, None, false)?;
                    }
                }
                HirTerminator::Jump { args, .. } => {
                    for arg in args {
                        if self.storage_local_is_heap(arg.destination)? {
                            storage.record_writer(arg.destination, false);
                            storage.link(arg.destination, arg.source, true);
                        }
                    }
                }
                HirTerminator::Invoke {
                    operation,
                    success_results,
                    ..
                } => match operation {
                    HirFallibleOperation::Call { target, args } => self.link_call_results(
                        &mut storage,
                        target,
                        expressions.values(*args),
                        success_results.iter().copied(),
                    )?,
                    HirFallibleOperation::MapOp {
                        op: HirMapOp::Get | HirMapOp::Remove,
                        receiver,
                        ..
                    } => {
                        for &result in success_results {
                            self.link_extracted_storage(&mut storage, *receiver, result)?;
                        }
                    }
                    _ => {}
                },
                _ => {}
            }
        }

        stores.retain(|&(local, projections, _)| {
            projections != 0 || storage.alias_writers.contains(&local)
        });
        let include_containing = stores.iter().any(|&(local, projections, _)| {
            storage.may_write_through_descendant(local, projections)
        });
        let reachable = storage.reachable(include_containing);
        for (local, projections, value) in stores {
            if reachable.get(&local).is_some_and(|&owns_storage| {
                owns_storage || storage.may_write_through_descendant(local, projections)
            }) {
                let stored = self.classify_return_expression(
                    function,
                    reachable_blocks,
                    value,
                    param_index_by_local,
                )?;
                summary = merge_return_alias(summary, stored);
                if matches!(summary, FunctionReturnAliasSummary::Unknown) {
                    return Ok(summary);
                }
            }
        }
        Ok(summary)
    }

    fn link_call_results(
        &self,
        storage: &mut ReturnedStorage,
        target: &CallTarget,
        args: &[HirValueId],
        results: impl Iterator<Item = LocalId>,
    ) -> Result<(), BorrowCheckError> {
        let retained = self.call_retained_arguments(target);
        for result in results {
            if !self.storage_local_is_heap(result)? {
                continue;
            }
            storage.record_writer(result, false);
            for (index, &argument) in args.iter().enumerate() {
                if retained.is_none_or(|indices| indices.contains(&index)) {
                    storage.retain_expression(self, argument, Some(result), true)?;
                }
            }
        }
        Ok(())
    }

    fn link_extracted_storage(
        &self,
        storage: &mut ReturnedStorage,
        source: HirValueId,
        result: LocalId,
    ) -> Result<(), BorrowCheckError> {
        if self.storage_local_is_heap(result)? {
            storage.record_writer(result, false);
            storage.retain_expression(self, source, Some(result), true)?;
        }
        Ok(())
    }

    fn storage_local_is_heap(&self, local: LocalId) -> Result<bool, BorrowCheckError> {
        let Some(&ty) = self.local_types.get(&local) else {
            return Err(self.diagnostics.internal_error(
                "Borrow checker cannot classify returned storage without a HIR local type",
            ));
        };
        Ok(classify_hir_type(ty, self.type_environment)?.has_in_place_mutation_surface())
    }

    /// Use the existing return contracts, not callee write effects.
    fn call_retained_arguments(&self, target: &CallTarget) -> Option<&[usize]> {
        let summary = match target {
            CallTarget::Local(id) => self.public_call_summaries.get(id),
            CallTarget::CrossModule(id) => self.module.imported_call_summaries.get(id),
            CallTarget::ModulePrivate(id) => self.module.module_private_call_summaries.get(id),
            CallTarget::Generated(id) => self.module.generated_call_summaries.get(id.as_ref()),
            CallTarget::External(id) => {
                let function = self.external_package_registry.get_function_by_id(*id)?;
                return match function.hir_return_alias() {
                    ExternalHirReturnAlias::Fresh => Some(&[]),
                    ExternalHirReturnAlias::AliasArgs(indices) => Some(indices),
                    ExternalHirReturnAlias::Unknown => None,
                };
            }
        }?;
        match &summary.return_alias {
            FunctionReturnAliasSummary::Fresh => Some(&[]),
            FunctionReturnAliasSummary::AliasParams(indices) => Some(indices),
            FunctionReturnAliasSummary::Unknown => None,
        }
    }

    fn classify_return_expression(
        &self,
        function: &HirFunction,
        reachable_blocks: &[BlockId],
        expression: HirValueId,
        param_index_by_local: &FxHashMap<LocalId, usize>,
    ) -> Result<FunctionReturnAliasSummary, BorrowCheckError> {
        let mut visiting_locals = FxHashSet::default();
        self.classify_return_expression_with_visiting(
            function,
            reachable_blocks,
            expression,
            param_index_by_local,
            &mut visiting_locals,
        )
    }

    fn classify_return_expression_with_visiting(
        &self,
        function: &HirFunction,
        reachable_blocks: &[BlockId],
        expression: HirValueId,
        param_index_by_local: &FxHashMap<LocalId, usize>,
        visiting_locals: &mut FxHashSet<LocalId>,
    ) -> Result<FunctionReturnAliasSummary, BorrowCheckError> {
        let expression_data = self.module.expressions.expression(expression);
        match &expression_data.kind {
            HirExpressionKind::Load(place) => {
                let root_local = place.root;

                if let Some(param_index) = param_index_by_local.get(&root_local).copied() {
                    return Ok(FunctionReturnAliasSummary::AliasParams(vec![param_index]));
                }

                self.classify_return_local(
                    function,
                    reachable_blocks,
                    root_local,
                    param_index_by_local,
                    visiting_locals,
                )
            }
            // Extracting a heap payload reads storage owned by the variant, rather than
            // creating an independent value from it.
            HirExpressionKind::VariantPayloadGet { source, .. }
                if classify_hir_type(expression_data.ty, self.type_environment)?
                    .has_in_place_mutation_surface() =>
            {
                self.classify_return_expression_with_visiting(
                    function,
                    reachable_blocks,
                    *source,
                    param_index_by_local,
                    visiting_locals,
                )
            }
            // A construction holds its stored heap values by reference, so the result aliases
            // whatever those values alias. The coarse summary names only argument origins,
            // not the storage path that retains them.
            kind if kind.is_aggregate_construction() => {
                let mut summary = FunctionReturnAliasSummary::Fresh;
                for child in self.module.expressions.child_values(expression) {
                    let child_expression = self.module.expressions.expression(child);
                    let stores_reference = match &child_expression.kind {
                        HirExpressionKind::Load(_)
                        | HirExpressionKind::VariantPayloadGet { .. } => {
                            classify_hir_type(child_expression.ty, self.type_environment)?
                                .has_in_place_mutation_surface()
                        }
                        kind => kind.is_aggregate_construction(),
                    };
                    if !stores_reference {
                        continue;
                    }
                    summary = merge_return_alias(
                        summary,
                        self.classify_return_expression_with_visiting(
                            function,
                            reachable_blocks,
                            child,
                            param_index_by_local,
                            visiting_locals,
                        )?,
                    );
                }
                Ok(summary)
            }
            // Copies and computed expressions produce independent results. Their input aliases
            // do not become aliases of the result.
            _ => Ok(FunctionReturnAliasSummary::Fresh),
        }
    }

    fn classify_return_local(
        &self,
        function: &HirFunction,
        reachable_blocks: &[BlockId],
        local: LocalId,
        param_index_by_local: &FxHashMap<LocalId, usize>,
        visiting_locals: &mut FxHashSet<LocalId>,
    ) -> Result<FunctionReturnAliasSummary, BorrowCheckError> {
        if !visiting_locals.insert(local) {
            return Ok(FunctionReturnAliasSummary::Unknown);
        }

        let mut alias_param_indices = Vec::new();
        let mut saw_fresh_write = false;
        let mut saw_unknown_write = false;
        let mut saw_any_write = false;

        for block_id in reachable_blocks {
            let block = self.block_by_id_or_error(*block_id, function.id)?;

            for statement in &block.statements {
                let writer_summary = match &statement.kind {
                    HirStatementKind::Write { target, value } => {
                        let target_local = match target {
                            HirWriteTarget::DefineLocal(target_local) => *target_local,
                            HirWriteTarget::AssignPlace(place) if place.projections.is_empty() => {
                                place.root
                            }
                            HirWriteTarget::AssignPlace(_) => continue,
                        };
                        if target_local != local {
                            continue;
                        }

                        Some(self.classify_return_expression_with_visiting(
                            function,
                            reachable_blocks,
                            *value,
                            param_index_by_local,
                            visiting_locals,
                        )?)
                    }
                    HirStatementKind::Call {
                        target,
                        args,
                        results,
                    } if results.iter().any(|result| result.local() == local) => {
                        Some(self.classify_call_result(
                            function,
                            target.clone(),
                            self.module.expressions.values(*args),
                            param_index_by_local,
                            reachable_blocks,
                            visiting_locals,
                        )?)
                    }
                    HirStatementKind::MapOp {
                        op,
                        receiver,
                        result: Some(result_local),
                        ..
                    } if result_local.local() == local => Some(self.classify_map_result(
                        *op,
                        *receiver,
                        function,
                        reachable_blocks,
                        param_index_by_local,
                        visiting_locals,
                    )?),
                    HirStatementKind::NumericOp {
                        result: result_local,
                        ..
                    }
                    | HirStatementKind::FastStoreConversion {
                        result: result_local,
                        ..
                    }
                    | HirStatementKind::LoopStepFailure {
                        result: result_local,
                        ..
                    }
                    | HirStatementKind::FormatFloat {
                        result: result_local,
                        ..
                    }
                    | HirStatementKind::ValidateFloat {
                        result: result_local,
                        ..
                    } if result_local.local() == local => Some(FunctionReturnAliasSummary::Fresh),
                    HirStatementKind::RangeCandidate {
                        candidate_result,
                        in_range_result,
                        ..
                    } if candidate_result.local() == local || in_range_result.local() == local => {
                        Some(FunctionReturnAliasSummary::Fresh)
                    }
                    _ => None,
                };

                let Some(writer_summary) = writer_summary else {
                    continue;
                };

                saw_any_write = true;
                match writer_summary {
                    FunctionReturnAliasSummary::Fresh => saw_fresh_write = true,
                    FunctionReturnAliasSummary::AliasParams(indices) => {
                        alias_param_indices.extend(indices);
                    }
                    FunctionReturnAliasSummary::Unknown => saw_unknown_write = true,
                }
            }
            if let HirTerminator::Jump { args, .. } = &block.terminator {
                // Native joins define each destination from its captured source. A merge slot
                // without a statement writer is not unknown merely because transport owns it.
                for argument in args {
                    if argument.destination != local {
                        continue;
                    }
                    let writer_summary =
                        if let Some(index) = param_index_by_local.get(&argument.source) {
                            FunctionReturnAliasSummary::AliasParams(vec![*index])
                        } else {
                            self.classify_return_local(
                                function,
                                reachable_blocks,
                                argument.source,
                                param_index_by_local,
                                visiting_locals,
                            )?
                        };
                    saw_any_write = true;
                    match writer_summary {
                        FunctionReturnAliasSummary::Fresh => saw_fresh_write = true,
                        FunctionReturnAliasSummary::AliasParams(indices) => {
                            alias_param_indices.extend(indices)
                        }
                        FunctionReturnAliasSummary::Unknown => saw_unknown_write = true,
                    }
                }
            }
            if let HirTerminator::Invoke {
                operation,
                success_results,
                error_result,
                ..
            } = &block.terminator
            {
                let writer_summary = if success_results.contains(&local) {
                    Some(match operation {
                        HirFallibleOperation::Call { target, args } => self.classify_call_result(
                            function,
                            target.clone(),
                            self.module.expressions.values(*args),
                            param_index_by_local,
                            reachable_blocks,
                            visiting_locals,
                        )?,
                        HirFallibleOperation::MapOp { op, receiver, .. } => self
                            .classify_map_result(
                                *op,
                                *receiver,
                                function,
                                reachable_blocks,
                                param_index_by_local,
                                visiting_locals,
                            )?,
                        _ => FunctionReturnAliasSummary::Fresh,
                    })
                } else if *error_result == local {
                    Some(FunctionReturnAliasSummary::Fresh)
                } else {
                    None
                };
                if let Some(writer_summary) = writer_summary {
                    saw_any_write = true;
                    match writer_summary {
                        FunctionReturnAliasSummary::Fresh => saw_fresh_write = true,
                        FunctionReturnAliasSummary::AliasParams(indices) => {
                            alias_param_indices.extend(indices)
                        }
                        FunctionReturnAliasSummary::Unknown => saw_unknown_write = true,
                    }
                }
            }
        }

        visiting_locals.remove(&local);

        alias_param_indices.sort_unstable();
        alias_param_indices.dedup();

        Ok(if !alias_param_indices.is_empty() {
            if saw_fresh_write || saw_unknown_write {
                FunctionReturnAliasSummary::Unknown
            } else {
                FunctionReturnAliasSummary::AliasParams(alias_param_indices)
            }
        } else if saw_unknown_write {
            FunctionReturnAliasSummary::Unknown
        } else if saw_fresh_write || saw_any_write {
            FunctionReturnAliasSummary::Fresh
        } else {
            FunctionReturnAliasSummary::Unknown
        })
    }

    fn classify_call_result(
        &self,
        function: &HirFunction,
        target: CallTarget,
        args: &[HirValueId],
        param_index_by_local: &FxHashMap<LocalId, usize>,
        reachable_blocks: &[BlockId],
        visiting_locals: &mut FxHashSet<LocalId>,
    ) -> Result<FunctionReturnAliasSummary, BorrowCheckError> {
        match target {
            CallTarget::Local(callee_id) => self.project_local_call_return_alias(
                function,
                callee_id,
                args,
                param_index_by_local,
                reachable_blocks,
                visiting_locals,
            ),
            CallTarget::CrossModule(origin) => self.project_imported_call_return_alias(
                function,
                &origin,
                args,
                param_index_by_local,
                reachable_blocks,
                visiting_locals,
            ),
            CallTarget::ModulePrivate(identity) => self.project_module_private_call_return_alias(
                function,
                &identity,
                args,
                param_index_by_local,
                reachable_blocks,
                visiting_locals,
            ),
            CallTarget::Generated(identity) => self.project_generated_call_return_alias(
                function,
                identity.as_ref(),
                args,
                param_index_by_local,
                reachable_blocks,
                visiting_locals,
            ),
            CallTarget::External(function_id) => {
                let Some(definition) = self
                    .external_package_registry
                    .get_function_by_id(function_id)
                else {
                    return Err(self.diagnostics.internal_error(
                        format!(
                            "Borrow checker could not resolve host call target '{}' while classifying a return",
                            function_id.name()
                        ),
                    ));
                };
                self.project_external_return_alias(
                    function,
                    definition.hir_return_alias(),
                    args,
                    param_index_by_local,
                    reachable_blocks,
                    visiting_locals,
                )
            }
        }
    }

    fn project_local_call_return_alias(
        &self,
        function: &HirFunction,
        callee_id: FunctionId,
        args: &[HirValueId],
        param_index_by_local: &FxHashMap<LocalId, usize>,
        reachable_blocks: &[BlockId],
        visiting_locals: &mut FxHashSet<LocalId>,
    ) -> Result<FunctionReturnAliasSummary, BorrowCheckError> {
        let Some(callee_summary) = self.public_call_summaries.get(&callee_id) else {
            return Err(self.diagnostics.internal_error(
                format!(
                    "Borrow checker is missing the public call summary for function '{}' while classifying a forwarded return",
                    self.diagnostics.function_name(callee_id)
                ),
            ));
        };

        self.project_alias_summary_through_arguments(
            AliasProjectionContext {
                function,
                return_alias: &callee_summary.return_alias,
                args,
                param_index_by_local,
                reachable_blocks,
                callee_description: &format!(
                    "user function '{}'",
                    self.diagnostics.function_name(callee_id)
                ),
            },
            visiting_locals,
        )
    }

    fn project_imported_call_return_alias(
        &self,
        function: &HirFunction,
        origin: &crate::compiler_frontend::semantic_identity::OriginFunctionId,
        args: &[HirValueId],
        param_index_by_local: &FxHashMap<LocalId, usize>,
        reachable_blocks: &[BlockId],
        visiting_locals: &mut FxHashSet<LocalId>,
    ) -> Result<FunctionReturnAliasSummary, BorrowCheckError> {
        let Some(summary) = self.module.imported_call_summaries.get(origin) else {
            return Err(self.diagnostics.internal_error(
                format!(
                    "Borrow checker is missing the provider call summary for imported function {origin:?} while classifying a forwarded return"
                ),
            ));
        };

        self.project_alias_summary_through_arguments(
            AliasProjectionContext {
                function,
                return_alias: &summary.return_alias,
                args,
                param_index_by_local,
                reachable_blocks,
                callee_description: "imported function",
            },
            visiting_locals,
        )
    }

    fn project_generated_call_return_alias(
        &self,
        function: &HirFunction,
        identity: &crate::compiler_frontend::semantic_identity::GeneratedFunctionIdentity,
        args: &[HirValueId],
        param_index_by_local: &FxHashMap<LocalId, usize>,
        reachable_blocks: &[BlockId],
        visiting_locals: &mut FxHashSet<LocalId>,
    ) -> Result<FunctionReturnAliasSummary, BorrowCheckError> {
        let Some(summary) = self.module.generated_call_summaries.get(identity) else {
            return Err(self.diagnostics.internal_error(
                format!(
                    "Borrow checker is missing the call summary for generated function {identity:?} while classifying a forwarded return"
                ),
            ));
        };

        self.project_alias_summary_through_arguments(
            AliasProjectionContext {
                function,
                return_alias: &summary.return_alias,
                args,
                param_index_by_local,
                reachable_blocks,
                callee_description: "generated function",
            },
            visiting_locals,
        )
    }

    fn project_module_private_call_return_alias(
        &self,
        function: &HirFunction,
        identity: &crate::compiler_frontend::semantic_identity::ModulePrivateExecutableIdentity,
        args: &[HirValueId],
        param_index_by_local: &FxHashMap<LocalId, usize>,
        reachable_blocks: &[BlockId],
        visiting_locals: &mut FxHashSet<LocalId>,
    ) -> Result<FunctionReturnAliasSummary, BorrowCheckError> {
        let Some(summary) = self.module.module_private_call_summaries.get(identity) else {
            return Err(self.diagnostics.internal_error(
                format!(
                    "Borrow checker is missing the call summary for module-private function {identity:?} while classifying a forwarded return"
                ),
            ));
        };

        self.project_alias_summary_through_arguments(
            AliasProjectionContext {
                function,
                return_alias: &summary.return_alias,
                args,
                param_index_by_local,
                reachable_blocks,
                callee_description: "module-private function",
            },
            visiting_locals,
        )
    }

    fn project_external_return_alias(
        &self,
        function: &HirFunction,
        return_alias: ExternalHirReturnAlias<'_>,
        args: &[HirValueId],
        param_index_by_local: &FxHashMap<LocalId, usize>,
        reachable_blocks: &[BlockId],
        visiting_locals: &mut FxHashSet<LocalId>,
    ) -> Result<FunctionReturnAliasSummary, BorrowCheckError> {
        let alias_summary = match return_alias {
            ExternalHirReturnAlias::Fresh => FunctionReturnAliasSummary::Fresh,
            ExternalHirReturnAlias::AliasArgs(indices) => {
                FunctionReturnAliasSummary::AliasParams(indices.to_vec())
            }
            ExternalHirReturnAlias::Unknown => FunctionReturnAliasSummary::Unknown,
        };

        self.project_alias_summary_through_arguments(
            AliasProjectionContext {
                function,
                return_alias: &alias_summary,
                args,
                param_index_by_local,
                reachable_blocks,
                callee_description: "external function",
            },
            visiting_locals,
        )
    }

    fn project_alias_summary_through_arguments(
        &self,
        context: AliasProjectionContext<'_>,
        visiting_locals: &mut FxHashSet<LocalId>,
    ) -> Result<FunctionReturnAliasSummary, BorrowCheckError> {
        let AliasProjectionContext {
            function,
            return_alias,
            args,
            param_index_by_local,
            reachable_blocks,
            callee_description,
        } = context;

        let alias_arg_indices = match return_alias {
            FunctionReturnAliasSummary::Fresh => return Ok(FunctionReturnAliasSummary::Fresh),
            FunctionReturnAliasSummary::Unknown => return Ok(FunctionReturnAliasSummary::Unknown),
            FunctionReturnAliasSummary::AliasParams(indices) => indices,
        };

        let mut caller_param_indices = Vec::new();
        for arg_index in alias_arg_indices {
            let Some(argument) = args.get(*arg_index) else {
                return Err(self.diagnostics.internal_error(
                    format!(
                        "Return alias metadata for function '{}' references call argument index {} but the forwarded call only has {} argument(s)",
                        callee_description,
                        arg_index,
                        args.len()
                    ),
                ));
            };

            match self.classify_return_expression_with_visiting(
                function,
                reachable_blocks,
                *argument,
                param_index_by_local,
                visiting_locals,
            )? {
                FunctionReturnAliasSummary::Fresh => {}
                FunctionReturnAliasSummary::AliasParams(indices) => {
                    caller_param_indices.extend(indices);
                }
                FunctionReturnAliasSummary::Unknown => {
                    return Ok(FunctionReturnAliasSummary::Unknown);
                }
            }
        }

        caller_param_indices.sort_unstable();
        caller_param_indices.dedup();

        Ok(if caller_param_indices.is_empty() {
            FunctionReturnAliasSummary::Fresh
        } else {
            FunctionReturnAliasSummary::AliasParams(caller_param_indices)
        })
    }

    fn classify_map_result(
        &self,
        op: crate::compiler_frontend::hir::expressions::HirMapOp,
        receiver: HirValueId,
        function: &HirFunction,
        reachable_blocks: &[BlockId],
        param_index_by_local: &FxHashMap<LocalId, usize>,
        visiting_locals: &mut FxHashSet<LocalId>,
    ) -> Result<FunctionReturnAliasSummary, BorrowCheckError> {
        if !matches!(
            op,
            crate::compiler_frontend::hir::expressions::HirMapOp::Get
        ) {
            return Ok(FunctionReturnAliasSummary::Fresh);
        }

        self.classify_return_expression_with_visiting(
            function,
            reachable_blocks,
            receiver,
            param_index_by_local,
            visiting_locals,
        )
    }
}

fn merge_return_alias(
    left: FunctionReturnAliasSummary,
    right: FunctionReturnAliasSummary,
) -> FunctionReturnAliasSummary {
    // Conservative join:
    // Unknown dominates; Fresh is neutral; AliasParams unions indices.
    match (left, right) {
        (FunctionReturnAliasSummary::Unknown, _) | (_, FunctionReturnAliasSummary::Unknown) => {
            FunctionReturnAliasSummary::Unknown
        }

        (FunctionReturnAliasSummary::Fresh, other) | (other, FunctionReturnAliasSummary::Fresh) => {
            other
        }

        (
            FunctionReturnAliasSummary::AliasParams(mut left),
            FunctionReturnAliasSummary::AliasParams(right),
        ) => {
            left.extend(right);
            left.sort_unstable();
            left.dedup();
            FunctionReturnAliasSummary::AliasParams(left)
        }
    }
}
