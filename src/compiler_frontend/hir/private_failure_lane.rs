//! Install the internal failure lane for inferred-failure private functions.
//!
//! WHAT: after summary convergence, a private function that escapes builtin failure
//!       returns that failure through the existing fallible carrier. Callers branch
//!       on the carrier. Builtin `Error!` returns the same `Error`. A compound
//!       store conversion lowered as a trap is retargeted onto that same edge.
//!       The synthetic start keeps its builtin slot only while an error edge escapes.
//! WHY: a may-fail bit cannot construct the eventual `Error`. The lane reuses
//!      `FallibleBranch` / `ReturnError` and the JS `{tag, value}` ABI. It is not a
//!      source error slot and is not a public or foreign contract.

use std::collections::VecDeque;

use rustc_hash::{FxHashMap, FxHashSet};

use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckReport;
use crate::compiler_frontend::builtins::error_codes::BuiltinErrorCode;
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalBuiltinType, CanonicalTypeIdentity,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::datatypes::numeric_operators::numeric_failure_codes;
use crate::compiler_frontend::external_packages::CallTarget;
use crate::compiler_frontend::hir::blocks::{HirBlock, HirLocal};
use crate::compiler_frontend::hir::expression_store::HirConstructionFailure;
use crate::compiler_frontend::hir::expressions::{HirExpression, HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::failure_facts::HirBuiltinFailureBoundary;
use crate::compiler_frontend::hir::hir_builder::{
    CatchHandlerRecord, CatchHandlerTarget, CatchProtectedCall,
};
use crate::compiler_frontend::hir::hir_side_table::HirLocalOriginKind;
use crate::compiler_frontend::hir::ids::{
    BlockId, FunctionId, HirNodeId, HirValueId, LocalId, RegionId,
};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::numeric::NumericFailureMode;
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::statements::{
    HirLocalDestination, HirStatement, HirStatementKind, HirWriteTarget,
};
use crate::compiler_frontend::hir::terminators::{HirTerminator, RuntimeFailureCause};
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
            type_environment,
            lane_functions,
            builtin_error,
            catch_handlers,
            local_types: FxHashMap::default(),
            next_local: 0,
            next_node: 0,
        };
        installer.assert_records_reference_live_calls(&records)?;
        installer.prepare_ids()?;
        installer.install_return_lanes(&handler_records)?;
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
    validate_hir_module(hir, type_environment)?;
    Ok(PrivateFailureLaneInstallation::Rewritten)
}

struct LaneInstaller<'a> {
    hir: &'a mut HirModule,
    report: &'a BorrowCheckReport,
    type_environment: &'a mut TypeEnvironment,
    lane_functions: FxHashSet<FunctionId>,
    builtin_error: Option<TypeId>,
    /// Catch handler owning each recorded call's failure edge, keyed by call statement.
    catch_handlers: FxHashMap<HirNodeId, CatchHandlerTarget>,
    local_types: FxHashMap<LocalId, TypeId>,
    next_local: u32,
    next_node: u32,
}

impl LaneInstaller<'_> {
    fn prepare_ids(&mut self) -> Result<(), CompilerError> {
        let mut next_local = 0u32;
        let mut next_node = 0u32;
        for (index, block) in self.hir.blocks.iter().enumerate() {
            if block.id != BlockId(index as u32) {
                return Err(CompilerError::compiler_error(
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
        }
        self.next_local = next_local;
        self.next_node = next_node;
        Ok(())
    }

    /// Converts success returns to the new carrier slot, including returns inside catch
    /// handler subgraphs. Handlers gain their inbound edge only when `rewrite_bodies`
    /// routes a split producer to them, so entry reachability alone would leave a
    /// handler `return` as a bare `Return` that the caller misreads as a carrier.
    fn install_return_lanes(
        &mut self,
        handlers: &[CatchHandlerRecord],
    ) -> Result<(), CompilerError> {
        let mut handler_roots: FxHashMap<FunctionId, Vec<BlockId>> = FxHashMap::default();
        for record in handlers {
            if self.lane_functions.contains(&record.owner) {
                handler_roots
                    .entry(record.owner)
                    .or_default()
                    .push(record.handler.block);
            }
        }
        let lane_functions = self.lane_functions.iter().copied().collect::<Vec<_>>();
        for function_id in lane_functions {
            let function_index = self.function_index(function_id)?;
            let success_type = self.hir.functions[function_index].return_type;
            if self
                .type_environment
                .fallible_carrier_slots(success_type)
                .is_some()
            {
                return Err(CompilerError::compiler_error(
                    "private internal failure lane cannot coexist with a source error slot",
                ));
            }
            let error_type = self.builtin_error_type()?;
            let carrier = self
                .type_environment
                .intern_fallible_carrier(success_type, error_type);
            self.hir.functions[function_index].return_type = carrier;
            let entry = self.hir.functions[function_index].entry;
            let mut roots = vec![entry];
            if let Some(handlers) = handler_roots.remove(&function_id) {
                roots.extend(handlers);
            }
            for block_id in reachable_blocks_from_roots(self.hir, &roots) {
                let terminator = &mut self.hir.blocks[block_id.0 as usize].terminator;
                if let HirTerminator::Return(value) = terminator {
                    *terminator = HirTerminator::ReturnSuccess(*value);
                }
            }
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
            let Some(producer) = self.producer_at(function_id, block_id, index)? else {
                continue;
            };
            self.split_producer(function_id, block_id, index, producer)?;
            return Ok(true);
        }
        Ok(false)
    }

    fn producer_at(
        &self,
        function_id: FunctionId,
        block_id: BlockId,
        index: usize,
    ) -> Result<Option<Producer>, CompilerError> {
        let statement = &self.hir.blocks[block_id.0 as usize].statements[index];
        match &statement.kind {
            HirStatementKind::Call { target, result, .. } => {
                if !self.target_uses_private_lane(target)? {
                    return Ok(None);
                }
                if let Some(destination) = result
                    && self.local_type(destination.local()).is_some_and(|ty| {
                        self.type_environment.fallible_carrier_slots(ty).is_some()
                    })
                {
                    return Ok(None);
                }
                let success_type = match result {
                    Some(destination) => self.local_type(destination.local()).ok_or_else(|| {
                        CompilerError::compiler_error(
                            "private failure call result local has no type",
                        )
                    })?,
                    None => self.type_environment.builtins().none,
                };
                Ok(Some(Producer {
                    statement: statement.id,
                    scalar_destination: *result,
                    success_type,
                }))
            }
            HirStatementKind::NumericOp {
                failure_mode,
                result,
                ..
            }
            | HirStatementKind::RangeStepFailure {
                failure_mode,
                result,
                ..
            }
            | HirStatementKind::FormatFloat {
                failure_mode,
                result,
                ..
            }
            | HirStatementKind::ValidateFloat {
                failure_mode,
                result,
                ..
            } if self.lane_functions.contains(&function_id)
                && *failure_mode == NumericFailureMode::Trap
                && statement_has_implicit_failure(&statement.kind) =>
            {
                let success_type = self.local_type(result.local()).ok_or_else(|| {
                    CompilerError::compiler_error("checked numeric result local has no type")
                })?;
                Ok(Some(Producer {
                    statement: statement.id,
                    scalar_destination: Some(*result),
                    success_type,
                }))
            }
            _ => Ok(None),
        }
    }

    fn split_producer(
        &mut self,
        function_id: FunctionId,
        block_id: BlockId,
        index: usize,
        producer: Producer,
    ) -> Result<(), HirConstructionFailure> {
        let error_type = self.builtin_error_type()?;
        let carrier_type = self
            .type_environment
            .intern_fallible_carrier(producer.success_type, error_type);
        let region = self.hir.blocks[block_id.0 as usize].region;
        let carrier_local = self.allocate_local(block_id, carrier_type, region);
        self.retarget_producer(block_id, index, carrier_local)?;

        let suffix = self.hir.blocks[block_id.0 as usize]
            .statements
            .split_off(index + 1);
        let old_terminator = std::mem::replace(
            &mut self.hir.blocks[block_id.0 as usize].terminator,
            HirTerminator::Uninitialized,
        );
        let catch_handler = self.catch_handlers.get(&producer.statement).copied();
        let error_block = self.allocate_block(region);
        let success_block = self.allocate_block(region);
        self.finish_error_edge(
            function_id,
            error_block,
            carrier_local,
            carrier_type,
            error_type,
            catch_handler,
        )?;
        self.finish_success_edge(
            success_block,
            producer.scalar_destination,
            producer.success_type,
            carrier_local,
            carrier_type,
            suffix,
            old_terminator,
        )?;
        let branch = HirTerminator::FallibleBranch {
            result: self.load(carrier_local, carrier_type, region)?,
            success_block,
            error_block,
        };
        self.hir.blocks[block_id.0 as usize].terminator = branch;
        Ok(())
    }

    fn retarget_producer(
        &mut self,
        block_id: BlockId,
        index: usize,
        carrier_local: LocalId,
    ) -> Result<(), CompilerError> {
        let statement = &mut self.hir.blocks[block_id.0 as usize].statements[index];
        let implicit_failure = statement_has_implicit_failure(&statement.kind);
        match &mut statement.kind {
            HirStatementKind::Call { result, .. } => {
                *result = Some(HirLocalDestination::Define(carrier_local));
            }
            HirStatementKind::NumericOp {
                failure_mode,
                result,
                ..
            }
            | HirStatementKind::RangeStepFailure {
                failure_mode,
                result,
                ..
            }
            | HirStatementKind::FormatFloat {
                failure_mode,
                result,
                ..
            }
            | HirStatementKind::ValidateFloat {
                failure_mode,
                result,
                ..
            } if implicit_failure => {
                *failure_mode = NumericFailureMode::ReturnError;
                *result = HirLocalDestination::Define(carrier_local);
            }
            _ => {
                return Err(CompilerError::compiler_error(
                    "private failure lane tried to retarget a non-producer statement",
                ));
            }
        }
        Ok(())
    }

    /// Terminates a producer's error edge: a catch handler receives the error slot with the
    /// original payload, otherwise the enclosing builtin-`Error` lane returns it.
    fn finish_error_edge(
        &mut self,
        function_id: FunctionId,
        error_block: BlockId,
        carrier_local: LocalId,
        carrier_type: TypeId,
        error_type: TypeId,
        catch_handler: Option<CatchHandlerTarget>,
    ) -> Result<(), HirConstructionFailure> {
        let region = self.hir.blocks[error_block.0 as usize].region;
        let payload = self.unwrap_error(carrier_local, carrier_type, error_type, region)?;
        if let Some(handler) = catch_handler {
            let slot_type = self.local_type(handler.error_local).ok_or_else(|| {
                CompilerError::compiler_error("catch handler error local has no type")
            })?;
            if slot_type != error_type {
                return Err(CompilerError::compiler_error(
                    "catch handler error local does not hold a builtin Error value",
                )
                .into());
            }
            let write = HirStatement {
                id: self.allocate_node(),
                kind: HirStatementKind::Write {
                    target: HirWriteTarget::DefineLocal(handler.error_local),
                    value: payload,
                },
                span: None,
            };
            let block = &mut self.hir.blocks[error_block.0 as usize];
            block.statements.push(write);
            block.terminator = HirTerminator::Jump {
                target: handler.block,
                args: vec![],
            };
            return Ok(());
        }
        let terminator = if self.function_propagates_builtin_error(function_id)? {
            HirTerminator::ReturnError(payload)
        } else {
            return Err(CompilerError::compiler_error(
                "private implicit failure reached a function with no Error! slot and no internal lane",
            )
            .into());
        };
        self.hir.blocks[error_block.0 as usize].terminator = terminator;
        Ok(())
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "the success edge rewrite keeps the block, optional scalar, carrier local and types, suffix and terminator as separate inputs"
    )]
    fn finish_success_edge(
        &mut self,
        success_block: BlockId,
        scalar_destination: Option<HirLocalDestination>,
        success_type: TypeId,
        carrier_local: LocalId,
        carrier_type: TypeId,
        mut suffix: Vec<HirStatement>,
        terminator: HirTerminator,
    ) -> Result<(), HirConstructionFailure> {
        let region = self.hir.blocks[success_block.0 as usize].region;
        if let Some(scalar_destination) = scalar_destination {
            let target = match scalar_destination {
                HirLocalDestination::Define(local) => HirWriteTarget::DefineLocal(local),
                HirLocalDestination::Update(local) => {
                    HirWriteTarget::AssignPlace(HirPlace::local(local))
                }
            };
            let value = self.unwrap_success(carrier_local, carrier_type, success_type, region)?;
            suffix.insert(
                0,
                HirStatement {
                    id: self.allocate_node(),
                    kind: HirStatementKind::Write { target, value },
                    span: None,
                },
            );
        }
        let block = &mut self.hir.blocks[success_block.0 as usize];
        block.statements = suffix;
        block.terminator = terminator;
        Ok(())
    }

    /// Every record must name a scalar call lowered into this module.
    ///
    /// WHY: a dangling record would silently drop a routed failure edge.
    fn assert_records_reference_live_calls(
        &self,
        records: &[CatchProtectedCall],
    ) -> Result<(), CompilerError> {
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
            return Err(CompilerError::compiler_error(
                "Catch-protected call record does not reference a scalar call",
            ));
        }
        Ok(())
    }

    /// Deletes catch handler subgraphs that no routed error edge reaches.
    ///
    /// WHAT: convergence may prove every protected call infallible, leaving the lowering-time
    ///       handler unreachable. Blocks reachable from such a handler but from no function entry
    ///       are removed; entry-reachable blocks such as the shared catch merge are kept.
    /// WHY: proof may remove a handler's runtime path (plan section 7), and full-strength
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
                .flat_map(|block| block.statements.iter().map(|statement| statement.id))
                .collect::<FxHashSet<_>>();
            self.hir
                .side_table
                .retain_live_locals_and_statements(&live_locals, &live_statements);
        }
    }

    /// Source casts and compound stores initially retain a trap edge until their function lane
    /// has been installed; each cause keeps the carrier needed to return the original Error.
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
                let (carrier, owner) = match cause {
                    RuntimeFailureCause::StoreConversion { carrier } => {
                        (carrier, "compound write-back")
                    }
                    RuntimeFailureCause::AuthoredCastConversion { carrier } => {
                        (carrier, "authored cast")
                    }
                };
                let carrier_type = self.local_types.get(&carrier).copied().ok_or_else(|| {
                    CompilerError::compiler_error(format!("{owner} carrier has no local type"))
                })?;
                let error_type = self.builtin_error_type()?;
                if self
                    .type_environment
                    .fallible_carrier_slots(carrier_type)
                    .is_none_or(|(_, error)| error != error_type)
                {
                    return Err(CompilerError::compiler_error(format!(
                        "{owner} carrier must contain builtin Error"
                    ))
                    .into());
                }
                let region = self.hir.blocks[block_id.0 as usize].region;
                let payload = self.unwrap_error(carrier, carrier_type, error_type, region)?;
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
    fn narrow_entry_return(&mut self) -> Result<bool, CompilerError> {
        let Some(function_id) = self.hir.start_function else {
            return Ok(false);
        };
        let function_index = self.function_index(function_id)?;
        let return_type = self.hir.functions[function_index].return_type;
        let Some((success_type, _)) = self.type_environment.fallible_carrier_slots(return_type)
        else {
            return Ok(false);
        };
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

        self.hir.functions[function_index].return_type = success_type;
        for block_id in blocks {
            let terminator = &mut self.hir.blocks[block_id.0 as usize].terminator;
            let previous = std::mem::replace(terminator, HirTerminator::Uninitialized);
            *terminator = match previous {
                HirTerminator::ReturnSuccess(value) => HirTerminator::Return(value),
                other => other,
            };
        }
        Ok(true)
    }

    fn function_propagates_builtin_error(
        &self,
        function_id: FunctionId,
    ) -> Result<bool, CompilerError> {
        if self.lane_functions.contains(&function_id) {
            return Ok(true);
        }
        let return_type = self.hir.functions[self.function_index(function_id)?].return_type;
        let Some((_, error_type)) = self.type_environment.fallible_carrier_slots(return_type)
        else {
            return Ok(false);
        };
        Ok(self.builtin_error == Some(error_type))
    }

    fn target_uses_private_lane(&self, target: &CallTarget) -> Result<bool, CompilerError> {
        match target {
            CallTarget::Local(function_id) => Ok(self.lane_functions.contains(function_id)),
            CallTarget::CrossModule(_)
            | CallTarget::ModulePrivate(_)
            | CallTarget::Generated(_) => Ok(self.call_summary(target)?.escapes_builtin_failure),
            CallTarget::External(_) => Ok(false),
        }
    }

    fn call_summary(&self, target: &CallTarget) -> Result<&PublicCallSummary, CompilerError> {
        let summary = match target {
            CallTarget::Local(function) => self.report.analysis.public_call_summaries.get(function),
            CallTarget::CrossModule(origin) => self.hir.imported_call_summaries.get(origin),
            CallTarget::ModulePrivate(identity) => {
                self.hir.module_private_call_summaries.get(identity)
            }
            CallTarget::Generated(identity) => self.hir.generated_call_summaries.get(identity),
            CallTarget::External(_) => None,
        };
        summary.ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "private failure lane has no exact call summary for {target:?}"
            ))
        })
    }

    fn builtin_error_type(&self) -> Result<TypeId, CompilerError> {
        self.builtin_error.ok_or_else(|| {
            CompilerError::compiler_error("private failure lane requires the builtin Error type")
        })
    }

    fn function_index(&self, function_id: FunctionId) -> Result<usize, CompilerError> {
        self.hir
            .functions
            .iter()
            .position(|function| function.id == function_id)
            .ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "private failure lane lost function {function_id:?}"
                ))
            })
    }

    fn function_entry(&self, function_id: FunctionId) -> Result<BlockId, CompilerError> {
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

    fn unwrap_success(
        &mut self,
        carrier_local: LocalId,
        carrier_type: TypeId,
        success_type: TypeId,
        region: RegionId,
    ) -> Result<HirValueId, HirConstructionFailure> {
        let carrier = self.load(carrier_local, carrier_type, region)?;
        self.append_expression(
            HirExpressionKind::FallibleUnwrapSuccess { result: carrier },
            success_type,
            region,
        )
    }

    fn unwrap_error(
        &mut self,
        carrier_local: LocalId,
        carrier_type: TypeId,
        error_type: TypeId,
        region: RegionId,
    ) -> Result<HirValueId, HirConstructionFailure> {
        let carrier = self.load(carrier_local, carrier_type, region)?;
        self.append_expression(
            HirExpressionKind::FallibleUnwrapError { result: carrier },
            error_type,
            region,
        )
    }
}

struct Producer {
    statement: HirNodeId,
    scalar_destination: Option<HirLocalDestination>,
    success_type: TypeId,
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
        // A numeric operation is a lane producer only for its shared code set. `Float`
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
) -> Result<FxHashSet<FunctionId>, CompilerError> {
    let mut lane_functions = FxHashSet::default();
    for function in &hir.functions {
        let facts = hir
            .function_failure_facts
            .get(&function.id)
            .ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "private failure lane is missing semantic facts for {:?}",
                    function.id
                ))
            })?;
        if !matches!(facts.boundary, HirBuiltinFailureBoundary::InferPrivate) {
            continue;
        }
        let summary = report
            .analysis
            .public_call_summaries
            .get(&function.id)
            .ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "private failure lane is missing the call summary for {:?}",
                    function.id
                ))
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

/// Reaches blocks from several roots with one shared visited set, so overlapping
/// handler subgraphs are walked once. Roots may repeat; each block is returned once.
fn reachable_blocks_from_roots(hir: &HirModule, roots: &[BlockId]) -> Vec<BlockId> {
    let mut pending = roots.iter().copied().collect::<VecDeque<_>>();
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
