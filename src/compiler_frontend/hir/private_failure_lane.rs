//! Install the internal failure lane for inferred-failure private functions.
//!
//! WHAT: after summary convergence, a private function that escapes builtin failure
//!       returns that failure through the existing fallible carrier. Callers branch
//!       on the carrier. Builtin `Error!` returns the same `Error`. A compound
//!       store conversion lowered as a trap is retargeted onto that same edge.
//!       `start` traps until it receives its own `Error!` slot.
//! WHY: a may-fail bit cannot construct the eventual `Error`. The lane reuses
//!      `FallibleBranch` / `ReturnError` and the JS `{tag, value}` ABI. It is not a
//!      source error slot and is not a public or foreign contract.

use std::collections::VecDeque;

use rustc_hash::FxHashSet;

use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckReport;
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalBuiltinType, CanonicalTypeIdentity,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::external_packages::CallTarget;
use crate::compiler_frontend::hir::blocks::{HirBlock, HirLocal};
use crate::compiler_frontend::hir::expressions::{HirExpression, HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::numeric::HirNumericOperands;
use crate::compiler_frontend::hir::patterns::HirPattern;
use crate::compiler_frontend::hir::failure_facts::HirBuiltinFailureBoundary;
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId, HirNodeId, HirValueId, LocalId, RegionId};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::numeric::NumericFailureMode;
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::statements::{HirStatement, HirStatementKind};
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::hir::utils::terminator_targets;
use crate::compiler_frontend::public_call_summary::PublicCallSummary;

/// Give escaping private functions one internal builtin-`Error` lane and make
/// every call to such a function branch before its success value is used.
pub(crate) fn install_private_failure_lanes(
    hir: &mut HirModule,
    report: &BorrowCheckReport,
    type_environment: &mut TypeEnvironment,
) -> Result<(), CompilerError> {
    let lane_functions = private_lane_functions(hir, report)?;
    let builtin_error = type_environment.type_id_for_canonical_identity(
        &CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Error),
    );
    let mut installer = LaneInstaller {
        hir,
        report,
        type_environment,
        lane_functions,
        builtin_error,
        next_local: 0,
        next_node: 0,
        next_value: 0,
    };
    installer.prepare_ids()?;
    installer.install_return_lanes()?;
    installer.rewrite_bodies()?;
    installer.retarget_store_conversions()
}

struct LaneInstaller<'a> {
    hir: &'a mut HirModule,
    report: &'a BorrowCheckReport,
    type_environment: &'a mut TypeEnvironment,
    lane_functions: FxHashSet<FunctionId>,
    builtin_error: Option<TypeId>,
    next_local: u32,
    next_node: u32,
    next_value: u32,
}

impl LaneInstaller<'_> {
    fn prepare_ids(&mut self) -> Result<(), CompilerError> {
        let mut next_local = 0u32;
        let mut next_node = 0u32;
        let mut next_value = 0u32;
        for (index, block) in self.hir.blocks.iter().enumerate() {
            if block.id != BlockId(index as u32) {
                return Err(CompilerError::compiler_error(
                    "private failure lane install requires dense HIR block ids",
                ));
            }
            note_block_ids(block, &mut next_local, &mut next_node, &mut next_value);
        }
        self.next_local = next_local;
        self.next_node = next_node;
        self.next_value = next_value;
        Ok(())
    }

    fn install_return_lanes(&mut self) -> Result<(), CompilerError> {
        let lane_functions = self.lane_functions.iter().copied().collect::<Vec<_>>();
        for function_id in lane_functions {
            let function_index = self.function_index(function_id)?;
            let success_type = self.hir.functions[function_index].return_type;
            if self.type_environment.fallible_carrier_slots(success_type).is_some() {
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
            for block_id in reachable_blocks(self.hir, entry) {
                let terminator = &mut self.hir.blocks[block_id.0 as usize].terminator;
                if let HirTerminator::Return(value) = terminator {
                    *terminator = HirTerminator::ReturnSuccess(value.clone());
                }
            }
        }
        Ok(())
    }

    fn rewrite_bodies(&mut self) -> Result<(), CompilerError> {
        let function_ids = self.hir.functions.iter().map(|function| function.id).collect::<Vec<_>>();
        for function_id in function_ids {
            let entry = self.function_entry(function_id)?;
            let mut pending = VecDeque::from([entry]);
            let mut seen = FxHashSet::default();
            while let Some(block_id) = pending.pop_front() {
                if !seen.insert(block_id) {
                    continue;
                }
                if let Some(success_block) = self.rewrite_block(function_id, block_id)? {
                    pending.push_back(success_block);
                    continue;
                }
                for target in terminator_targets(&self.hir.blocks[block_id.0 as usize].terminator) {
                    pending.push_back(target);
                }
            }
        }
        Ok(())
    }

    /// Split the first internal-lane producer in this block. The success continuation
    /// is returned so its own producers are rewritten. `None` means the block is done.
    fn rewrite_block(
        &mut self,
        function_id: FunctionId,
        block_id: BlockId,
    ) -> Result<Option<BlockId>, CompilerError> {
        let statement_count = self.hir.blocks[block_id.0 as usize].statements.len();
        for index in 0..statement_count {
            let Some(producer) = self.producer_at(function_id, block_id, index)? else {
                continue;
            };
            return self.split_producer(function_id, block_id, index, producer).map(Some);
        }
        Ok(None)
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
                if let Some(local) = result
                    && self.local_type(*local)?.is_some_and(|ty| {
                        self.type_environment.fallible_carrier_slots(ty).is_some()
                    })
                {
                    return Ok(None);
                }
                let success_type = match result {
                    Some(local) => self.local_type(*local)?.ok_or_else(|| {
                        CompilerError::compiler_error(
                            "private failure call result local has no type",
                        )
                    })?,
                    None => self.type_environment.builtins().none,
                };
                Ok(Some(Producer {
                    scalar_local: *result,
                    success_type,
                }))
            }
            HirStatementKind::NumericOp { failure_mode, result, .. }
            | HirStatementKind::FormatFloat { failure_mode, result, .. }
            | HirStatementKind::ValidateFloat { failure_mode, result, .. }
                if self.lane_functions.contains(&function_id)
                    && *failure_mode == NumericFailureMode::Trap =>
            {
                let success_type = self.local_type(*result)?.ok_or_else(|| {
                    CompilerError::compiler_error("checked numeric result local has no type")
                })?;
                Ok(Some(Producer {
                    scalar_local: Some(*result),
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
    ) -> Result<BlockId, CompilerError> {
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
        let error_block = self.allocate_block(region);
        let success_block = self.allocate_block(region);
        self.finish_error_edge(function_id, error_block, carrier_local, carrier_type, error_type)?;
        self.finish_success_edge(
            success_block,
            producer.scalar_local,
            producer.success_type,
            carrier_local,
            carrier_type,
            suffix,
            old_terminator,
        )?;
        let branch = HirTerminator::FallibleBranch {
            result: self.load(carrier_local, carrier_type, region),
            success_block,
            error_block,
        };
        self.hir.blocks[block_id.0 as usize].terminator = branch;
        Ok(success_block)
    }

    fn retarget_producer(
        &mut self,
        block_id: BlockId,
        index: usize,
        carrier_local: LocalId,
    ) -> Result<(), CompilerError> {
        let statement = &mut self.hir.blocks[block_id.0 as usize].statements[index];
        match &mut statement.kind {
            HirStatementKind::Call { result, .. } => *result = Some(carrier_local),
            HirStatementKind::NumericOp { failure_mode, result, .. }
            | HirStatementKind::FormatFloat { failure_mode, result, .. }
            | HirStatementKind::ValidateFloat { failure_mode, result, .. } => {
                *failure_mode = NumericFailureMode::ReturnError;
                *result = carrier_local;
            }
            _ => {
                return Err(CompilerError::compiler_error(
                    "private failure lane tried to retarget a non-producer statement",
                ));
            }
        }
        Ok(())
    }

    fn finish_error_edge(
        &mut self,
        function_id: FunctionId,
        error_block: BlockId,
        carrier_local: LocalId,
        carrier_type: TypeId,
        error_type: TypeId,
    ) -> Result<(), CompilerError> {
        let region = self.hir.blocks[error_block.0 as usize].region;
        let payload = self.unwrap_error(carrier_local, carrier_type, error_type, region);
        let terminator = if self.function_propagates_builtin_error(function_id)? {
            HirTerminator::ReturnError(payload)
        } else if self.hir.start_function == Some(function_id) {
            HirTerminator::RuntimeFailure {
                message: "implicit builtin failure escaped to start".to_owned(),
            }
        } else {
            return Err(CompilerError::compiler_error(
                "private implicit failure reached a function with no Error! slot and no internal lane",
            ));
        };
        self.hir.blocks[error_block.0 as usize].terminator = terminator;
        Ok(())
    }

    fn finish_success_edge(
        &mut self,
        success_block: BlockId,
        scalar_local: Option<LocalId>,
        success_type: TypeId,
        carrier_local: LocalId,
        carrier_type: TypeId,
        mut suffix: Vec<HirStatement>,
        terminator: HirTerminator,
    ) -> Result<(), CompilerError> {
        let region = self.hir.blocks[success_block.0 as usize].region;
        if let Some(scalar_local) = scalar_local {
            let value = self.unwrap_success(carrier_local, carrier_type, success_type, region);
            suffix.insert(
                0,
                HirStatement {
                    id: self.allocate_node(),
                    kind: HirStatementKind::Assign {
                        target: HirPlace::Local(scalar_local),
                        value,
                    },
                    span: None,
                },
            );
        }
        let block = &mut self.hir.blocks[success_block.0 as usize];
        block.statements = suffix;
        block.terminator = terminator;
        Ok(())
    }

    /// A compound store conversion is lowered before the lane exists, so its error
    /// edge is still a trap. The target assign already sits on the success continuation.
    fn retarget_store_conversions(&mut self) -> Result<(), CompilerError> {
        let lane_functions = self.lane_functions.iter().copied().collect::<Vec<_>>();
        for function_id in lane_functions {
            let entry = self.function_entry(function_id)?;
            let blocks = reachable_blocks(self.hir, entry);
            for block_id in blocks {
                let Some((error_block, carrier_local, carrier_type)) =
                    self.store_conversion_trap(block_id)?
                else {
                    continue;
                };
                let error_type = self.builtin_error_type()?;
                let region = self.hir.blocks[error_block.0 as usize].region;
                let payload = self.unwrap_error(carrier_local, carrier_type, error_type, region);
                self.hir.blocks[error_block.0 as usize].terminator =
                    HirTerminator::ReturnError(payload);
            }
        }
        Ok(())
    }

    fn store_conversion_trap(
        &self,
        block_id: BlockId,
    ) -> Result<Option<(BlockId, LocalId, TypeId)>, CompilerError> {
        let HirTerminator::FallibleBranch {
            result, error_block, ..
        } = &self.hir.blocks[block_id.0 as usize].terminator
        else {
            return Ok(None);
        };
        let error_block = *error_block;
        let HirTerminator::RuntimeFailure { message } =
            &self.hir.blocks[error_block.0 as usize].terminator
        else {
            return Ok(None);
        };
        if message != "Compound assignment conversion failed" {
            return Err(CompilerError::compiler_error(
                "private failure lane found an unexpected runtime failure on a fallible branch",
            ));
        }
        let HirExpressionKind::Load(HirPlace::Local(local)) = &result.kind else {
            return Err(CompilerError::compiler_error(
                "compound write-back branch does not load its conversion carrier",
            ));
        };
        if self
            .type_environment
            .fallible_carrier_slots(result.ty)
            .is_none()
        {
            return Err(CompilerError::compiler_error(
                "compound write-back carrier is not a fallible result",
            ));
        }
        Ok(Some((error_block, *local, result.ty)))
    }

    fn function_propagates_builtin_error(&self, function_id: FunctionId) -> Result<bool, CompilerError> {
        if self.hir.start_function == Some(function_id) {
            return Ok(false);
        }
        if self.lane_functions.contains(&function_id) {
            return Ok(true);
        }
        let return_type = self.hir.functions[self.function_index(function_id)?].return_type;
        let Some((_, error_type)) = self.type_environment.fallible_carrier_slots(return_type) else {
            return Ok(false);
        };
        Ok(self.builtin_error == Some(error_type))
    }

    fn target_uses_private_lane(&self, target: &CallTarget) -> Result<bool, CompilerError> {
        match target {
            CallTarget::Local(function_id) => Ok(self.lane_functions.contains(function_id)),
            CallTarget::CrossModule(_) | CallTarget::ModulePrivate(_) | CallTarget::Generated(_) => {
                Ok(self.call_summary(target)?.escapes_builtin_failure)
            }
            CallTarget::External(_) => Ok(false),
        }
    }

    fn call_summary(&self, target: &CallTarget) -> Result<&PublicCallSummary, CompilerError> {
        let summary = match target {
            CallTarget::Local(function) => self.report.analysis.public_call_summaries.get(function),
            CallTarget::CrossModule(origin) => self.hir.imported_call_summaries.get(origin),
            CallTarget::ModulePrivate(identity) => self.hir.module_private_call_summaries.get(identity),
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
            CompilerError::compiler_error(
                "private failure lane requires the builtin Error type",
            )
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

    fn local_type(&self, local_id: LocalId) -> Result<Option<TypeId>, CompilerError> {
        Ok(self
            .hir
            .blocks
            .iter()
            .flat_map(|block| &block.locals)
            .find(|local| local.id == local_id)
            .map(|local| local.ty))
    }

    fn allocate_local(&mut self, block_id: BlockId, ty: TypeId, region: RegionId) -> LocalId {
        let id = LocalId(self.next_local);
        self.next_local += 1;
        self.hir.blocks[block_id.0 as usize].locals.push(HirLocal {
            id,
            ty,
            mutable: true,
            region,
            span: None,
        });
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

    fn allocate_value(&mut self) -> HirValueId {
        let id = HirValueId(self.next_value);
        self.next_value += 1;
        id
    }

    fn load(&mut self, local: LocalId, ty: TypeId, region: RegionId) -> HirExpression {
        HirExpression {
            id: self.allocate_value(),
            kind: HirExpressionKind::Load(HirPlace::Local(local)),
            ty,
            value_kind: ValueKind::RValue,
            region,
            span: None,
        }
    }

    fn unwrap_success(
        &mut self,
        carrier_local: LocalId,
        carrier_type: TypeId,
        success_type: TypeId,
        region: RegionId,
    ) -> HirExpression {
        HirExpression {
            id: self.allocate_value(),
            kind: HirExpressionKind::FallibleUnwrapSuccess {
                result: Box::new(self.load(carrier_local, carrier_type, region)),
            },
            ty: success_type,
            value_kind: ValueKind::RValue,
            region,
            span: None,
        }
    }

    fn unwrap_error(
        &mut self,
        carrier_local: LocalId,
        carrier_type: TypeId,
        error_type: TypeId,
        region: RegionId,
    ) -> HirExpression {
        HirExpression {
            id: self.allocate_value(),
            kind: HirExpressionKind::FallibleUnwrapError {
                result: Box::new(self.load(carrier_local, carrier_type, region)),
            },
            ty: error_type,
            value_kind: ValueKind::RValue,
            region,
            span: None,
        }
    }
}

struct Producer {
    scalar_local: Option<LocalId>,
    success_type: TypeId,
}

fn private_lane_functions(
    hir: &HirModule,
    report: &BorrowCheckReport,
) -> Result<FxHashSet<FunctionId>, CompilerError> {
    let mut lane_functions = FxHashSet::default();
    for function in &hir.functions {
        let facts = hir.function_failure_facts.get(&function.id).ok_or_else(|| {
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

fn note_block_ids(block: &HirBlock, next_local: &mut u32, next_node: &mut u32, next_value: &mut u32) {
    for local in &block.locals {
        *next_local = (*next_local).max(local.id.0.saturating_add(1));
    }
    for statement in &block.statements {
        *next_node = (*next_node).max(statement.id.0.saturating_add(1));
        note_statement_ids(&statement.kind, next_value);
    }
    note_terminator_ids(&block.terminator, next_value);
}

fn note_statement_ids(kind: &HirStatementKind, next_value: &mut u32) {
    match kind {
        HirStatementKind::Assign { value, .. }
        | HirStatementKind::Expr(value)
        | HirStatementKind::PushRuntimeFragment { value, .. }
        | HirStatementKind::CastOp { source: value, .. }
        | HirStatementKind::FormatFloat { source: value, .. }
        | HirStatementKind::ValidateFloat { source: value, .. } => note_expression_id(value, next_value),
        HirStatementKind::Call { args, .. } => {
            for argument in args {
                note_expression_id(argument, next_value);
            }
        }
        HirStatementKind::MapOp { receiver, args, .. } => {
            note_expression_id(receiver, next_value);
            for argument in args {
                note_expression_id(argument, next_value);
            }
        }
        HirStatementKind::NumericOp { operands, .. } => match operands {
            HirNumericOperands::Unary { operand } => note_expression_id(operand, next_value),
            HirNumericOperands::Binary { left, right } => {
                note_expression_id(left, next_value);
                note_expression_id(right, next_value);
            }
        },
        HirStatementKind::FloatRangeCandidate {
            current, step, end, ascending, ..
        } => {
            note_expression_id(current, next_value);
            note_expression_id(step, next_value);
            note_expression_id(end, next_value);
            note_expression_id(ascending, next_value);
        }
        HirStatementKind::Drop(_) => {}
    }
}

fn note_terminator_ids(terminator: &HirTerminator, next_value: &mut u32) {
    match terminator {
        HirTerminator::Return(value)
        | HirTerminator::ReturnSuccess(value)
        | HirTerminator::ReturnError(value)
        | HirTerminator::If { condition: value, .. }
        | HirTerminator::FallibleBranch { result: value, .. }
        | HirTerminator::AssertFailure { message: value, .. } => note_expression_id(value, next_value),
        HirTerminator::Match { scrutinee, arms } => {
            note_expression_id(scrutinee, next_value);
            for arm in arms {
                note_pattern_ids(&arm.pattern, next_value);
                if let Some(guard) = &arm.guard {
                    note_expression_id(guard, next_value);
                }
            }
        }
        HirTerminator::Jump { .. }
        | HirTerminator::Break { .. }
        | HirTerminator::Continue { .. }
        | HirTerminator::Uninitialized
        | HirTerminator::RuntimeFailure { .. } => {}
    }
}

fn note_pattern_ids(pattern: &HirPattern, next_value: &mut u32) {
    match pattern {
        HirPattern::Literal(value)
        | HirPattern::OptionValue { value }
        | HirPattern::OptionRelational { value, .. }
        | HirPattern::Relational { value, .. } => note_expression_id(value, next_value),
        HirPattern::OptionNone
        | HirPattern::OptionPresent
        | HirPattern::Wildcard
        | HirPattern::ChoiceVariant { .. } => {}
    }
}

fn note_expression_id(expression: &HirExpression, next_value: &mut u32) {
    *next_value = (*next_value).max(expression.id.0.saturating_add(1));
    match &expression.kind {
        HirExpressionKind::BinOp { left, right, .. } => {
            note_expression_id(left, next_value);
            note_expression_id(right, next_value);
        }
        HirExpressionKind::UnaryOp { operand, .. }
        | HirExpressionKind::TupleGet { tuple: operand, .. }
        | HirExpressionKind::FallibleUnwrapSuccess { result: operand }
        | HirExpressionKind::FallibleUnwrapError { result: operand }
        | HirExpressionKind::Cast { source: operand, .. }
        | HirExpressionKind::VariantPayloadGet { source: operand, .. } => {
            note_expression_id(operand, next_value);
        }
        HirExpressionKind::Range { start, end } => {
            note_expression_id(start, next_value);
            note_expression_id(end, next_value);
        }
        HirExpressionKind::StructConstruct { fields, .. } => {
            for (_, field) in fields {
                note_expression_id(field, next_value);
            }
        }
        HirExpressionKind::Collection(items) | HirExpressionKind::TupleConstruct { elements: items } => {
            for item in items {
                note_expression_id(item, next_value);
            }
        }
        HirExpressionKind::VariantConstruct { fields, .. } => {
            for field in fields {
                note_expression_id(&field.value, next_value);
            }
        }
        HirExpressionKind::MapLiteral(entries) => {
            for entry in entries {
                note_expression_id(&entry.key, next_value);
                note_expression_id(&entry.value, next_value);
            }
        }
        HirExpressionKind::Int(_)
        | HirExpressionKind::Float(_)
        | HirExpressionKind::FixedScalar(_)
        | HirExpressionKind::Number(_)
        | HirExpressionKind::Bool(_)
        | HirExpressionKind::Char(_)
        | HirExpressionKind::StringLiteral(_)
        | HirExpressionKind::StructuralString { .. }
        | HirExpressionKind::Load(_)
        | HirExpressionKind::Copy(_) => {}
    }
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
