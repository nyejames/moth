//! Install the internal failure lane for inferred-failure private functions.
//!
//! After summary convergence, inferred private failures use native success and error
//! edges. Installation preserves success slots and producer identities, routes catches
//! and prunes unreachable handlers before final validation and analysis refresh.

use crate::compiler_frontend::compiler_messages::CompilerStage;
use std::collections::VecDeque;

use rustc_hash::{FxHashMap, FxHashSet};

use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckReport;
use crate::compiler_frontend::builtins::error_codes::BuiltinErrorCode;
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalBuiltinType, CanonicalTypeIdentity,
};
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::datatypes::numeric_operators::numeric_failure_codes;
use crate::compiler_frontend::external_packages::{CallTarget, ExternalPackageRegistry};
use crate::compiler_frontend::hir::blocks::{HirBlock, HirLocal};
use crate::compiler_frontend::hir::expression_store::HirConstructionFailure;
use crate::compiler_frontend::hir::expressions::{HirExpression, HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::failure_facts::HirBuiltinFailureBoundary;
use crate::compiler_frontend::hir::hir_builder::{
    CatchHandlerRecord, CatchHandlerTarget, CatchProtectedCall,
};
use crate::compiler_frontend::hir::hir_side_table::{HirLocalOriginKind, HirLocation};
use crate::compiler_frontend::hir::ids::{
    BlockId, FunctionId, HirNodeId, HirValueId, LocalId, RegionId,
};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::numeric::NumericFailureMode;
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::statements::{
    HirLocalDestination, HirStatement, HirStatementKind, HirWriteTarget,
};
use crate::compiler_frontend::hir::terminators::{
    HirFallibleOperation, HirJumpArgument, HirTerminator, RuntimeFailureCause,
};
use crate::compiler_frontend::hir::utils::terminator_targets;
use crate::compiler_frontend::hir::validation::validate_hir_module;
use crate::compiler_frontend::public_call_summary::PublicCallSummary;

/// Whether lane installation rewrote the HIR it was given.
///
/// WHY: borrow analysis and validation of unchanged HIR would only reproduce the convergence
///      results, so the caller re-runs them only after a rewrite. Most generated sidecars and
///      fully proven bodies have no escaping private failure and stay unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PrivateFailureLaneInstallation {
    Unchanged,
    Rewritten,
}

/// Give escaping private functions one internal builtin-`Error` lane and make
/// every call to such a function branch before its success value is used.
pub(crate) fn install_private_failure_lanes(
    hir: &mut HirModule,
    report: &BorrowCheckReport,
    type_environment: &mut TypeEnvironment,
    external_packages: &ExternalPackageRegistry,
) -> Result<PrivateFailureLaneInstallation, HirConstructionFailure> {
    let lane_functions = private_lane_functions(hir, report)?;
    let builtin_error = type_environment.type_id_for_canonical_identity(
        &CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Error),
    );
    // Records are pending CFG edges only until installation. Draining them here keeps
    // post-install validation and every later pass free of record awareness.
    let records = std::mem::take(&mut hir.catch_protected_calls);
    let handler_records = std::mem::take(&mut hir.catch_handlers);
    let catch_handlers = records
        .iter()
        .map(|record| (record.statement, record.handler))
        .collect::<FxHashMap<_, _>>();
    // Draining records, installing return lanes and pruning handlers all change the HIR.
    let mut rewritten =
        !records.is_empty() || !handler_records.is_empty() || !lane_functions.is_empty();
    {
        let mut installer = LaneInstaller {
            hir,
            report,
            lane_functions,
            builtin_error,
            catch_handlers,
            local_types: FxHashMap::default(),
            next_local: 0,
            next_node: 0,
        };
        installer.assert_records_reference_live_calls(&records)?;
        installer.prepare_ids()?;
        installer.install_return_lanes()?;
        rewritten |= installer.rewrite_bodies()?;
        installer.retarget_implicit_conversion_failures()?;
        rewritten |= installer.narrow_entry_return()?;
        installer.prune_unrouted_catch_handlers(&handler_records);
        installer.hir.compact_block_ids();
        rewritten |= installer
            .hir
            .finalize_function_provenance_after_rewrites()?;
    }
    if !rewritten {
        return Ok(PrivateFailureLaneInstallation::Unchanged);
    }

    // Rewrites finish before analysis can consume the final base or generated body.
    validate_hir_module(hir, type_environment, external_packages)?;
    Ok(PrivateFailureLaneInstallation::Rewritten)
}

struct LaneInstaller<'a> {
    hir: &'a mut HirModule,
    report: &'a BorrowCheckReport,
    lane_functions: FxHashSet<FunctionId>,
    builtin_error: Option<TypeId>,
    /// Catch handler owning each recorded call's failure edge, keyed by call statement.
    catch_handlers: FxHashMap<HirNodeId, CatchHandlerTarget>,
    local_types: FxHashMap<LocalId, TypeId>,
    next_local: u32,
    next_node: u32,
}

impl LaneInstaller<'_> {
    fn prepare_ids(&mut self) -> Result<(), InfrastructureFailure> {
        let mut next_local = 0u32;
        let mut next_node = 0u32;
        for (index, block) in self.hir.blocks.iter().enumerate() {
            if block.id != BlockId(index as u32) {
                return Err(InfrastructureFailure::internal(
                    CompilerStage::HirConstruction,
                    "private failure lane install requires dense HIR block ids",
                ));
            }
            for local in &block.locals {
                next_local = next_local.max(local.id.0.saturating_add(1));
                self.local_types.insert(local.id, local.ty);
            }
            for statement in &block.statements {
                next_node = next_node.max(statement.id.0.saturating_add(1));
            }
            if let HirTerminator::Invoke { id, .. } = &block.terminator {
                next_node = next_node.max(id.0.saturating_add(1));
            }
        }
        self.next_local = next_local;
        self.next_node = next_node;
        Ok(())
    }

    /// Installing an error channel leaves every ordered success slot and return intact.
    fn install_return_lanes(&mut self) -> Result<(), InfrastructureFailure> {
        let error_type = if self.lane_functions.is_empty() {
            return Ok(());
        } else {
            self.builtin_error_type()?
        };
        for function in &mut self.hir.functions {
            if !self.lane_functions.contains(&function.id) {
                continue;
            }
            if function.error_return.is_some() {
                return Err(InfrastructureFailure::internal(
                    CompilerStage::HirConstruction,
                    "private internal failure lane cannot coexist with a source error slot",
                ));
            }
            function.error_return = Some(error_type);
        }
        Ok(())
    }

    /// Returns whether any producer was split.
    fn rewrite_bodies(&mut self) -> Result<bool, HirConstructionFailure> {
        let function_ids = self
            .hir
            .functions
            .iter()
            .map(|function| function.id)
            .collect::<Vec<_>>();
        let mut split_any = false;
        for function_id in function_ids {
            let entry = self.function_entry(function_id)?;
            let mut pending = VecDeque::from([entry]);
            let mut seen = FxHashSet::default();
            while let Some(block_id) = pending.pop_front() {
                if !seen.insert(block_id) {
                    continue;
                }
                // A split replaces the terminator with its error and success edges. Following
                // both reaches the suffix and any catch handler first routed by this split.
                split_any |= self.rewrite_block(function_id, block_id)?;
                for target in terminator_targets(&self.hir.blocks[block_id.0 as usize].terminator) {
                    pending.push_back(target);
                }
            }
        }
        Ok(split_any)
    }

    /// Split the first internal-lane producer in this block, returning whether one existed.
    /// Its success continuation holds the remaining statements and is rewritten when the
    /// traversal reaches it.
    fn rewrite_block(
        &mut self,
        function_id: FunctionId,
        block_id: BlockId,
    ) -> Result<bool, HirConstructionFailure> {
        let statement_count = self.hir.blocks[block_id.0 as usize].statements.len();
        for index in 0..statement_count {
            if !self.producer_at(function_id, block_id, index)? {
                continue;
            }
            self.split_producer(function_id, block_id, index)?;
            return Ok(true);
        }
        Ok(false)
    }

    fn producer_at(
        &self,
        function_id: FunctionId,
        block_id: BlockId,
        index: usize,
    ) -> Result<bool, InfrastructureFailure> {
        let statement = &self.hir.blocks[block_id.0 as usize].statements[index];
        match &statement.kind {
            HirStatementKind::Call { target, .. } => self.target_uses_private_lane(target),
            HirStatementKind::NumericOp { failure_mode, .. }
            | HirStatementKind::RangeStepFailure { failure_mode, .. }
            | HirStatementKind::FormatFloat { failure_mode, .. }
            | HirStatementKind::ValidateFloat { failure_mode, .. } => {
                Ok(self.lane_functions.contains(&function_id)
                    && *failure_mode == NumericFailureMode::Trap
                    && statement_has_implicit_failure(&statement.kind))
            }
            _ => Ok(false),
        }
    }

    fn split_producer(
        &mut self,
        function_id: FunctionId,
        block_id: BlockId,
        index: usize,
    ) -> Result<(), HirConstructionFailure> {
        let error_type = self.builtin_error_type()?;
        let region = self.hir.blocks[block_id.0 as usize].region;
        let mut suffix = self.hir.blocks[block_id.0 as usize]
            .statements
            .split_off(index + 1);
        // The selected producer is now the last statement in the prefix.
        let statement = self.hir.blocks[block_id.0 as usize]
            .statements
            .pop()
            .ok_or_else(|| {
                InfrastructureFailure::internal(
                    CompilerStage::HirConstruction,
                    "private failure lane lost its selected producer",
                )
            })?;
        let (operation, destinations, scalar_destination) = match statement.kind {
            HirStatementKind::Call {
                target,
                args,
                results,
            } => (HirFallibleOperation::Call { target, args }, results, None),
            HirStatementKind::NumericOp {
                op,
                operands,
                result,
                ..
            } => (
                HirFallibleOperation::NumericOp { op, operands },
                Vec::new(),
                Some(result),
            ),
            // Range guards always fail, so they define no success result.
            HirStatementKind::RangeStepFailure { cause, .. } => (
                HirFallibleOperation::RangeStepFailure { cause },
                Vec::new(),
                None,
            ),
            HirStatementKind::FormatFloat { source, result, .. } => (
                HirFallibleOperation::FormatFloat { source },
                Vec::new(),
                Some(result),
            ),
            HirStatementKind::ValidateFloat { source, result, .. } => (
                HirFallibleOperation::ValidateFloat { source },
                Vec::new(),
                Some(result),
            ),
            _ => {
                return Err(InfrastructureFailure::internal(
                    CompilerStage::HirConstruction,
                    "private failure lane selected a non-producer statement",
                )
                .into());
            }
        };
        if let HirFallibleOperation::Call { target, .. } = &operation
            && matches!(
                target,
                CallTarget::CrossModule(_)
                    | CallTarget::ModulePrivate(_)
                    | CallTarget::Generated(_)
            )
        {
            // These projections were bound before summary convergence. Installing the
            // inferred lane changes only delivery, not the stable target or success slots.
            let contract = self
                .hir
                .call_result_contracts
                .get_mut(target)
                .ok_or_else(|| {
                    InfrastructureFailure::internal(
                        CompilerStage::HirConstruction,
                        "private failure lane call is missing its nonlocal result contract",
                    )
                })?;
            contract.error_return = Some(error_type);
        }
        let old_terminator = std::mem::replace(
            &mut self.hir.blocks[block_id.0 as usize].terminator,
            HirTerminator::Uninitialized,
        );
        let failure = self.allocate_block(region);
        let success = self.allocate_block(region);
        let catch_handler = self.catch_handlers.get(&statement.id).copied();
        let error_result = if let Some(handler) = catch_handler {
            if self.local_type(handler.error_local) != Some(error_type) {
                return Err(InfrastructureFailure::internal(
                    CompilerStage::HirConstruction,
                    "catch handler error local does not hold a builtin Error value",
                )
                .into());
            }
            let error = self.allocate_local(block_id, error_type, region);
            self.hir.blocks[failure.0 as usize].terminator = HirTerminator::Jump {
                target: handler.block,
                args: vec![HirJumpArgument {
                    source: error,
                    destination: handler.error_local,
                }],
            };
            error
        } else {
            if !self.function_propagates_builtin_error(function_id)? {
                return Err(InfrastructureFailure::internal(CompilerStage::HirConstruction,
                    "private implicit failure reached a function with no Error! slot and no internal lane",
                ).into());
            }
            let error = self.allocate_local(block_id, error_type, region);
            let payload = self.load(error, error_type, region)?;
            self.hir.blocks[failure.0 as usize].terminator = HirTerminator::ReturnError(payload);
            error
        };

        // Invoke defines edge locals. Updates still commit through ordinary writes,
        // after all ordered results have been produced, preserving mutable aliases.
        let mut success_results =
            Vec::with_capacity(destinations.len() + usize::from(scalar_destination.is_some()));
        let mut updates = Vec::new();
        for destination in destinations.into_iter().chain(scalar_destination) {
            match destination {
                HirLocalDestination::Define(local) => success_results.push(local),
                HirLocalDestination::Update(local) => {
                    let ty = self.local_type(local).ok_or_else(|| {
                        InfrastructureFailure::internal(
                            CompilerStage::HirConstruction,
                            "private failure result local has no type",
                        )
                    })?;
                    let result = self.allocate_local(block_id, ty, region);
                    success_results.push(result);
                    let value = self.load(result, ty, region)?;
                    let update = HirStatement {
                        id: self.allocate_node(),
                        kind: HirStatementKind::Write {
                            target: HirWriteTarget::AssignPlace(HirPlace::local(local)),
                            value,
                        },
                        span: statement.span,
                    };
                    let ast_span = self
                        .hir
                        .side_table
                        .ast_span_for_hir(HirLocation::Statement(statement.id));
                    self.hir.side_table.map_statement(ast_span, &update);
                    updates.push(update);
                }
            }
        }
        let statements = if updates.is_empty() {
            suffix
        } else {
            updates.append(&mut suffix);
            updates
        };
        self.hir.blocks[success.0 as usize].statements = statements;
        self.hir.blocks[success.0 as usize].terminator = old_terminator;
        self.hir.blocks[block_id.0 as usize].terminator = HirTerminator::Invoke {
            id: statement.id,
            span: statement.span,
            operation,
            success,
            success_results,
            failure,
            error_result,
        };
        Ok(())
    }

    /// Every record must name an infallible call lowered into this module.
    ///
    /// WHY: a dangling record would silently drop a routed failure edge.
    fn assert_records_reference_live_calls(
        &self,
        records: &[CatchProtectedCall],
    ) -> Result<(), InfrastructureFailure> {
        // Most modules carry no protected calls, so skip indexing every call statement.
        if records.is_empty() {
            return Ok(());
        }
        let call_statements = self
            .hir
            .blocks
            .iter()
            .flat_map(|block| &block.statements)
            .filter(|statement| matches!(statement.kind, HirStatementKind::Call { .. }))
            .map(|statement| statement.id)
            .collect::<FxHashSet<_>>();
        if records
            .iter()
            .any(|record| !call_statements.contains(&record.statement))
        {
            return Err(InfrastructureFailure::internal(
                CompilerStage::HirConstruction,
                "Catch-protected call record does not reference an infallible call",
            ));
        }
        Ok(())
    }

    /// Deletes catch handler subgraphs that no routed error edge reaches.
    ///
    /// WHAT: convergence may prove every protected call infallible, leaving the lowering-time
    ///       handler unreachable. Blocks reachable from such a handler but from no function entry
    ///       are removed; entry-reachable blocks such as the shared catch merge are kept.
    /// WHY: proof may remove a handler's runtime path, and full-strength
    ///      ownership validation requires every remaining block to belong to a function CFG.
    fn prune_unrouted_catch_handlers(&mut self, handlers: &[CatchHandlerRecord]) {
        if handlers.is_empty() {
            return;
        }
        let entry_reachable = self
            .hir
            .functions
            .iter()
            .flat_map(|function| reachable_blocks(self.hir, function.entry))
            .collect::<FxHashSet<_>>();
        // Several records may share one handler, and handler subgraphs may overlap, so
        // deduplicated roots share a single visited set. The walk stops at
        // entry-reachable blocks: successors of a shared merge belong to a live CFG.
        let mut roots = FxHashSet::default();
        for record in handlers {
            if !entry_reachable.contains(&record.handler.block) {
                roots.insert(record.handler.block);
            }
        }
        let mut visited = FxHashSet::default();
        let mut pending = roots.into_iter().collect::<VecDeque<_>>();
        let mut remove = FxHashSet::default();
        while let Some(block_id) = pending.pop_front() {
            if !visited.insert(block_id) || entry_reachable.contains(&block_id) {
                continue;
            }
            remove.insert(block_id);
            let Some(block) = self.hir.blocks.get(block_id.0 as usize) else {
                continue;
            };
            for target in terminator_targets(&block.terminator) {
                pending.push_back(target);
            }
        }
        if !remove.is_empty() {
            self.hir.blocks.retain(|block| !remove.contains(&block.id));
            self.hir
                .function_provenance_by_block
                .retain(|(_, block_id), _| match block_id {
                    Some(block_id) => !remove.contains(block_id),
                    None => true,
                });
            let live_locals = self
                .hir
                .blocks
                .iter()
                .flat_map(|block| block.locals.iter().map(|local| local.id))
                .collect::<FxHashSet<_>>();
            let live_statements = self
                .hir
                .blocks
                .iter()
                .flat_map(|block| {
                    block.statements.iter().map(|statement| statement.id).chain(
                        match &block.terminator {
                            HirTerminator::Invoke { id, .. } => Some(*id),
                            _ => None,
                        },
                    )
                })
                .collect::<FxHashSet<_>>();
            self.hir
                .side_table
                .retain_live_locals_and_statements(&live_locals, &live_statements);
        }
    }

    /// Conversion traps retain the edge-local Error until their inferred lane is installed.
    fn retarget_implicit_conversion_failures(&mut self) -> Result<(), HirConstructionFailure> {
        let lane_functions = self.lane_functions.iter().copied().collect::<Vec<_>>();
        for function_id in lane_functions {
            let entry = self.function_entry(function_id)?;
            for block_id in reachable_blocks(self.hir, entry) {
                let cause = match &self.hir.blocks[block_id.0 as usize].terminator {
                    HirTerminator::RuntimeFailure {
                        cause: Some(cause), ..
                    } => *cause,
                    _ => continue,
                };
                let (error, owner) = match cause {
                    RuntimeFailureCause::StoreConversion { error } => {
                        (error, "compound write-back")
                    }
                    RuntimeFailureCause::AuthoredCastConversion { error } => {
                        (error, "authored cast")
                    }
                };
                let error_type = self.builtin_error_type()?;
                if self.local_type(error) != Some(error_type) {
                    return Err(InfrastructureFailure::internal(
                        CompilerStage::HirConstruction,
                        format!("{owner} error local must hold builtin Error"),
                    )
                    .into());
                }
                let region = self.hir.blocks[block_id.0 as usize].region;
                let payload = self.load(error, error_type, region)?;
                self.hir.blocks[block_id.0 as usize].terminator =
                    HirTerminator::ReturnError(payload);
            }
        }
        Ok(())
    }

    /// The AST entry always accepts builtin Error, but a dead error slot must not
    /// introduce a backend ABI or runtime requirement. Narrow only after private
    /// callees and statement-owned checks have installed their escaping edges.
    ///
    /// Returns whether the entry return was narrowed.
    fn narrow_entry_return(&mut self) -> Result<bool, InfrastructureFailure> {
        let Some(function_id) = self.hir.start_function else {
            return Ok(false);
        };
        let function_index = self.function_index(function_id)?;
        if self.hir.functions[function_index].error_return.is_none() {
            return Ok(false);
        }
        let entry = self.hir.functions[function_index].entry;
        let blocks = reachable_blocks(self.hir, entry);
        if blocks.iter().any(|block_id| {
            matches!(
                self.hir.blocks[block_id.0 as usize].terminator,
                HirTerminator::ReturnError(_)
            )
        }) {
            return Ok(false);
        }

        self.hir.functions[function_index].error_return = None;
        Ok(true)
    }

    fn function_propagates_builtin_error(
        &self,
        function_id: FunctionId,
    ) -> Result<bool, InfrastructureFailure> {
        if self.lane_functions.contains(&function_id) {
            return Ok(true);
        }
        let error_type = self.hir.functions[self.function_index(function_id)?].error_return;
        Ok(error_type.is_some() && self.builtin_error == error_type)
    }

    fn target_uses_private_lane(&self, target: &CallTarget) -> Result<bool, InfrastructureFailure> {
        match target {
            CallTarget::Local(function_id) => Ok(self.lane_functions.contains(function_id)),
            CallTarget::CrossModule(_)
            | CallTarget::ModulePrivate(_)
            | CallTarget::Generated(_) => Ok(self.call_summary(target)?.escapes_builtin_failure),
            CallTarget::External(_) => Ok(false),
        }
    }

    fn call_summary(
        &self,
        target: &CallTarget,
    ) -> Result<&PublicCallSummary, InfrastructureFailure> {
        let summary = match target {
            CallTarget::Local(function) => self.report.analysis.public_call_summaries.get(function),
            CallTarget::CrossModule(origin) => self.hir.imported_call_summaries.get(origin),
            CallTarget::ModulePrivate(identity) => {
                self.hir.module_private_call_summaries.get(identity)
            }
            CallTarget::Generated(identity) => {
                self.hir.generated_call_summaries.get(identity.as_ref())
            }
            CallTarget::External(_) => None,
        };
        summary.ok_or_else(|| {
            InfrastructureFailure::internal(
                CompilerStage::HirConstruction,
                format!("private failure lane has no exact call summary for {target:?}"),
            )
        })
    }

    fn builtin_error_type(&self) -> Result<TypeId, InfrastructureFailure> {
        self.builtin_error.ok_or_else(|| {
            InfrastructureFailure::internal(
                CompilerStage::HirConstruction,
                "private failure lane requires the builtin Error type",
            )
        })
    }

    fn function_index(&self, function_id: FunctionId) -> Result<usize, InfrastructureFailure> {
        self.hir
            .functions
            .iter()
            .position(|function| function.id == function_id)
            .ok_or_else(|| {
                InfrastructureFailure::internal(
                    CompilerStage::HirConstruction,
                    format!("private failure lane lost function {function_id:?}"),
                )
            })
    }

    fn function_entry(&self, function_id: FunctionId) -> Result<BlockId, InfrastructureFailure> {
        Ok(self.hir.functions[self.function_index(function_id)?].entry)
    }

    fn local_type(&self, local_id: LocalId) -> Option<TypeId> {
        self.local_types.get(&local_id).copied()
    }

    fn allocate_local(&mut self, block_id: BlockId, ty: TypeId, region: RegionId) -> LocalId {
        let id = LocalId(self.next_local);
        self.next_local += 1;
        self.local_types.insert(id, ty);
        self.hir.blocks[block_id.0 as usize].locals.push(HirLocal {
            id,
            ty,
            mutable: true,
            region,
            span: None,
        });

        // Mutable transport storage is scratch space, not a source-exclusive alias.
        self.hir
            .side_table
            .bind_local_origin(id, HirLocalOriginKind::CompilerTemp, None, None);
        id
    }

    fn allocate_block(&mut self, region: RegionId) -> BlockId {
        let id = BlockId(self.hir.blocks.len() as u32);
        self.hir.blocks.push(HirBlock {
            id,
            region,
            locals: Vec::new(),
            statements: Vec::new(),
            terminator: HirTerminator::Uninitialized,
        });
        id
    }

    fn allocate_node(&mut self) -> HirNodeId {
        let id = HirNodeId(self.next_node);
        self.next_node += 1;
        id
    }

    fn append_expression(
        &mut self,
        kind: HirExpressionKind,
        ty: TypeId,
        region: RegionId,
    ) -> Result<HirValueId, HirConstructionFailure> {
        self.hir.expressions.append_expression(HirExpression {
            kind,
            ty,
            value_kind: ValueKind::RValue,
            region,
            span: None,
        })
    }

    fn load(
        &mut self,
        local: LocalId,
        ty: TypeId,
        region: RegionId,
    ) -> Result<HirValueId, HirConstructionFailure> {
        self.append_expression(HirExpressionKind::Load(HirPlace::local(local)), ty, region)
    }
}

/// Arithmetic statements contain only implicit causes. Other checked statements carry a
/// range cause or a fixed integrity-guard code, whose classification owns lane eligibility.
fn statement_has_implicit_failure(kind: &HirStatementKind) -> bool {
    // Semantically discharged operations have no failure edge, so they never become lane
    // producers; the `Trap` guard in `producer_at` agrees by construction.
    if let HirStatementKind::NumericOp { failure_mode, .. } = kind
        && *failure_mode == NumericFailureMode::Infallible
    {
        return false;
    }
    let code = match kind {
        // A numeric operation is a lane producer only for its shared code set. Binary-float
        // negation and exact `Dec` arithmetic carry no codes, so their traps stay traps
        // even inside an inferred-lane function reached through an escaping call.
        HirStatementKind::NumericOp { op, .. } => {
            return !numeric_failure_codes(op.operator, op.domain).is_empty();
        }
        HirStatementKind::RangeStepFailure { cause, .. } => cause.builtin_error_code(),
        HirStatementKind::FormatFloat { .. } => BuiltinErrorCode::FloatFormatInvariant,
        HirStatementKind::ValidateFloat { .. } => BuiltinErrorCode::FloatBoundaryNonFinite,
        _ => return false,
    };
    code.is_implicit_failure()
}

fn private_lane_functions(
    hir: &HirModule,
    report: &BorrowCheckReport,
) -> Result<FxHashSet<FunctionId>, InfrastructureFailure> {
    let mut lane_functions = FxHashSet::default();
    for function in &hir.functions {
        let facts = hir
            .function_failure_facts
            .get(&function.id)
            .ok_or_else(|| {
                InfrastructureFailure::internal(
                    CompilerStage::HirConstruction,
                    format!(
                        "private failure lane is missing semantic facts for {:?}",
                        function.id
                    ),
                )
            })?;
        if !matches!(facts.boundary, HirBuiltinFailureBoundary::InferPrivate) {
            continue;
        }
        let summary = report
            .analysis
            .public_call_summaries
            .get(&function.id)
            .ok_or_else(|| {
                InfrastructureFailure::internal(
                    CompilerStage::HirConstruction,
                    format!(
                        "private failure lane is missing the call summary for {:?}",
                        function.id
                    ),
                )
            })?;
        if summary.escapes_builtin_failure {
            lane_functions.insert(function.id);
        }
    }
    Ok(lane_functions)
}

fn reachable_blocks(hir: &HirModule, entry: BlockId) -> Vec<BlockId> {
    let mut pending = VecDeque::from([entry]);
    let mut seen = FxHashSet::default();
    let mut blocks = Vec::new();
    while let Some(block_id) = pending.pop_front() {
        if !seen.insert(block_id) {
            continue;
        }
        let Some(block) = hir.blocks.get(block_id.0 as usize) else {
            continue;
        };
        blocks.push(block_id);
        for target in terminator_targets(&block.terminator) {
            pending.push_back(target);
        }
    }
    blocks
}
