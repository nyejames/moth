//! Iterative lowering for operator expression trees.
//!
//! WHAT: walks numeric, logical, comparison and range operator nodes with one explicit heap
//!       stack per walk. Non-operator children still lower through `lower_expression`.
//! WHY: the completed operator tree can be arbitrarily deep, so lowering must not recurse once
//!      per authored operator edge. Entry and finish steps run in evaluation order so diagnostics
//!      and emitted HIR follow the operand and operator sequence.

use crate::compiler_frontend::ast::expressions::expression_store::{
    ExpressionOperationView, ExpressionView, LogicalOperator,
};
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::hir::expression_store::HirConstructionFailure;
use crate::compiler_frontend::hir::hir_builder::HirBuilder;
use crate::compiler_frontend::hir::ids::HirValueId;
use crate::compiler_frontend::hir::statements::HirStatement;

use super::LoweredExpression;
use super::runtime::short_circuit_cfg::ShortCircuitPlan;

#[cfg(feature = "benchmark_counters")]
use crate::compiler_frontend::instrumentation::{FrontendCounter, increment_frontend_counter};

/// Where a completed operand's prelude and value go.
#[derive(Clone, Copy)]
enum OperandDelivery {
    /// The operand's prelude is emitted into the current block before the parent continues.
    CurrentBlock,
    /// The operand's prelude is queued on the parent, in the same order as
    /// `lower_child_expression_for_parent`.
    PendingPrelude,
}

/// How an operand is lowered, matching the per-operation child helpers used before the walk.
#[derive(Clone, Copy)]
enum OperandPolicy {
    /// Always lowered with its prelude emitted into the current block.
    AlwaysCurrentBlock,
    /// Lowered like `lower_child_expression_for_parent`: the prelude is queued unless the child
    /// needs the current block.
    PreludeUnlessCurrentBlockNeeded,
}

/// Per-operation state that lives beside the expression while its operands are lowered.
#[derive(Clone, Copy)]
enum OperatorState {
    /// No state beyond the expression itself.
    Plain,
    /// `and` / `or` before its left operand is delivered. The result type is captured before the
    /// left operand lowers, matching the order of the original logical lowering.
    LogicalLeft {
        operation: LogicalOperator,
        result_type: TypeId,
    },
    /// `and` / `or` after the left operand has branched and the right operand is being lowered.
    LogicalRight(ShortCircuitPlan),
}

/// One operator expression with its operands still in progress.
struct OperatorFrame<'e> {
    expression: ExpressionView<'e>,
    delivery: OperandDelivery,
    next_operand: u8,
    prelude: Vec<HirStatement>,
    state: OperatorState,
    /// Only successful `not` frames inherit a scan result, never forced current-block delivery.
    logical_not_needs_current_block: Option<bool>,
}

/// Returns whether `expression` is an operator node handled by the operator walk.
pub(super) fn is_operator_expression(expression: ExpressionView<'_>) -> bool {
    matches!(
        expression.operation(),
        ExpressionOperationView::NumericUnary { .. }
            | ExpressionOperationView::NumericBinary { .. }
            | ExpressionOperationView::LogicalNot { .. }
            | ExpressionOperationView::LogicalBinary { .. }
            | ExpressionOperationView::Compare { .. }
            | ExpressionOperationView::Range { .. }
    )
}

/// Error-typed operators keep the fallible lowering path, so they are not walked here.
fn is_walked_operator(expression: ExpressionView<'_>) -> bool {
    is_operator_expression(expression) && expression.error_type().is_none()
}

/// Returns the operand at `index` for an operator node, with its lowering policy.
fn operator_operand<'e>(
    expression: ExpressionView<'e>,
    index: u8,
) -> Option<(ExpressionView<'e>, OperandPolicy)> {
    let (operands, policy): ([Option<ExpressionView<'e>>; 2], OperandPolicy) =
        match expression.operation() {
            ExpressionOperationView::NumericUnary { operand, .. } => {
                ([Some(operand), None], OperandPolicy::AlwaysCurrentBlock)
            }
            ExpressionOperationView::NumericBinary { left, right, .. }
            | ExpressionOperationView::LogicalBinary { left, right, .. } => {
                ([Some(left), Some(right)], OperandPolicy::AlwaysCurrentBlock)
            }
            ExpressionOperationView::LogicalNot { operand } => (
                [Some(operand), None],
                OperandPolicy::PreludeUnlessCurrentBlockNeeded,
            ),
            ExpressionOperationView::Compare { left, right, .. } => (
                [Some(left), Some(right)],
                OperandPolicy::PreludeUnlessCurrentBlockNeeded,
            ),
            ExpressionOperationView::Range { start, end, .. } => (
                [Some(start), Some(end)],
                OperandPolicy::PreludeUnlessCurrentBlockNeeded,
            ),
            _ => return None,
        };
    let operand = operands.get(usize::from(index)).copied().flatten()?;
    Some((operand, policy))
}

/// Pops the most recently completed operand value.
fn pop_operator_operand(
    values: &mut Vec<HirValueId>,
) -> Result<HirValueId, HirConstructionFailure> {
    values.pop().ok_or_else(|| {
        InfrastructureFailure::internal(
            CompilerStage::HirConstruction,
            "Operator walk lost an operand value",
        )
        .into()
    })
}

impl<'a> HirBuilder<'a> {
    /// Lowers an operator expression root with an explicit work stack.
    pub(super) fn lower_operator_expression(
        &mut self,
        expression: ExpressionView<'_>,
    ) -> Result<LoweredExpression, HirConstructionFailure> {
        let root = self.begin_operator_frame(expression, OperandDelivery::PendingPrelude)?;
        let mut frames = vec![root];
        let mut values = Vec::new();
        loop {
            if let Some(lowered) = self.advance_operator_walk(&mut frames, &mut values)? {
                return Ok(lowered);
            }
        }
    }

    /// Advances the innermost frame by one step.
    ///
    /// Returns the root result once the root frame has finished. Otherwise returns `None`.
    fn advance_operator_walk<'e>(
        &mut self,
        frames: &mut Vec<OperatorFrame<'e>>,
        values: &mut Vec<HirValueId>,
    ) -> Result<Option<LoweredExpression>, HirConstructionFailure> {
        let Some(parent_index) = frames.len().checked_sub(1) else {
            return Err(InfrastructureFailure::internal(
                CompilerStage::HirConstruction,
                "Operator walk has no active frame",
            )
            .into());
        };
        let expression = frames[parent_index].expression;
        let operand_index = frames[parent_index].next_operand;
        let Some((child, policy)) = operator_operand(expression, operand_index) else {
            return self.finish_innermost_operator_frame(frames, values);
        };
        frames[parent_index].next_operand = operand_index + 1;

        // The right operand of `and` / `or` starts after the left operand has been delivered.
        if let (
            1,
            OperatorState::LogicalLeft {
                operation,
                result_type,
            },
        ) = (operand_index, frames[parent_index].state)
        {
            let lowered_left = pop_operator_operand(values)?;
            let plan = self.begin_short_circuit_binary(
                lowered_left,
                operation,
                result_type,
                &expression.span(),
            )?;
            frames[parent_index].state = OperatorState::LogicalRight(plan);
        }

        let mut child_needs_current_block = None;
        let delivery = match policy {
            OperandPolicy::AlwaysCurrentBlock => OperandDelivery::CurrentBlock,
            OperandPolicy::PreludeUnlessCurrentBlockNeeded => {
                let needs_current_block = frames[parent_index]
                    .logical_not_needs_current_block
                    .unwrap_or_else(|| self.expression_needs_current_block_lowering(child));
                child_needs_current_block = Some(needs_current_block);
                if needs_current_block {
                    // Earlier pending statements must land before a child that needs the block.
                    for statement in frames[parent_index].prelude.drain(..) {
                        self.emit_statement_to_current_block(statement)?;
                    }
                    OperandDelivery::CurrentBlock
                } else {
                    OperandDelivery::PendingPrelude
                }
            }
        };

        if is_walked_operator(child) {
            let mut child_frame = self.begin_operator_frame(child, delivery)?;
            if matches!(
                child.operation(),
                ExpressionOperationView::LogicalNot { .. }
            ) {
                // A successful `not` adds no block requirement and lowers its only operand
                // in the same selected view and active-catch context as this scan.
                child_frame.logical_not_needs_current_block = child_needs_current_block;
            }
            frames.push(child_frame);
            return Ok(None);
        }

        let lowered = self.lower_expression(child)?;
        self.deliver_operator_result(&mut frames[parent_index].prelude, values, delivery, lowered)?;
        Ok(None)
    }

    /// Finishes the innermost frame, then delivers its result to the parent.
    ///
    /// Returns the result once the root frame has finished.
    fn finish_innermost_operator_frame<'e>(
        &mut self,
        frames: &mut Vec<OperatorFrame<'e>>,
        values: &mut Vec<HirValueId>,
    ) -> Result<Option<LoweredExpression>, HirConstructionFailure> {
        let Some(frame) = frames.pop() else {
            return Err(InfrastructureFailure::internal(
                CompilerStage::HirConstruction,
                "Operator walk finished without an active frame",
            )
            .into());
        };
        let lowered =
            self.finish_operator_frame(frame.expression, frame.state, frame.prelude, values)?;
        let Some(parent) = frames.last_mut() else {
            return Ok(Some(lowered));
        };
        if frame.expression.success_types().is_empty() {
            // Mirrors `lower_expression`, which requires exactly one success slot for a child.
            self.single_result_value(Vec::new())?;
        }
        self.deliver_operator_result(&mut parent.prelude, values, frame.delivery, lowered)?;
        Ok(None)
    }

    /// Runs the entry steps the dispatcher performs for an operator expression.
    fn begin_operator_frame<'e>(
        &mut self,
        expression: ExpressionView<'e>,
        delivery: OperandDelivery,
    ) -> Result<OperatorFrame<'e>, HirConstructionFailure> {
        #[cfg(feature = "benchmark_counters")]
        increment_frontend_counter(FrontendCounter::CensusHirLowerExpressionEntries);
        self.log_expression_input(expression);
        self.accumulate_function_provenance(expression);

        let state = match expression.operation() {
            ExpressionOperationView::NumericUnary { facts, .. }
            | ExpressionOperationView::NumericBinary { facts, .. } => {
                self.resolved_numeric_operation(facts)?;
                OperatorState::Plain
            }
            ExpressionOperationView::Range { facts, .. } => {
                self.validate_range_operation(facts)?;
                OperatorState::Plain
            }
            ExpressionOperationView::LogicalBinary { operation, .. } => {
                OperatorState::LogicalLeft {
                    operation,
                    result_type: expression.value_type_id()?,
                }
            }
            _ => OperatorState::Plain,
        };

        Ok(OperatorFrame {
            expression,
            delivery,
            next_operand: 0,
            prelude: Vec::new(),
            state,
            logical_not_needs_current_block: None,
        })
    }

    /// Emits the per-operation result for a frame whose operands have all been delivered.
    fn finish_operator_frame(
        &mut self,
        expression: ExpressionView<'_>,
        state: OperatorState,
        prelude: Vec<HirStatement>,
        values: &mut Vec<HirValueId>,
    ) -> Result<LoweredExpression, HirConstructionFailure> {
        let span = expression.span();
        let value = match expression.operation() {
            ExpressionOperationView::NumericUnary { facts, .. } => {
                let operand = pop_operator_operand(values)?;
                self.finish_resolved_numeric_value(operand, None, facts)?
            }
            ExpressionOperationView::NumericBinary { facts, .. } => {
                let right = pop_operator_operand(values)?;
                let left = pop_operator_operand(values)?;
                self.finish_resolved_numeric_value(left, Some(right), facts)?
            }
            ExpressionOperationView::LogicalBinary { .. } => {
                let OperatorState::LogicalRight(plan) = state else {
                    return Err(InfrastructureFailure::hir_transformation(
                        "Logical operation reached HIR without its short-circuit branch",
                    )
                    .into());
                };
                let right = pop_operator_operand(values)?;
                self.finish_short_circuit_binary(plan, right, &span)?
            }
            ExpressionOperationView::LogicalNot { .. } => {
                let operand = pop_operator_operand(values)?;
                self.finish_logical_not_value(operand, expression, &span)?
            }
            ExpressionOperationView::Compare { facts, .. } => {
                let right = pop_operator_operand(values)?;
                let left = pop_operator_operand(values)?;
                self.finish_resolved_comparison_value(left, right, facts)?
            }
            ExpressionOperationView::Range { facts, .. } => {
                let end = pop_operator_operand(values)?;
                let start = pop_operator_operand(values)?;
                self.finish_range_value(start, end, facts)?
            }
            _ => {
                return Err(InfrastructureFailure::hir_transformation(
                    "Non-operator expression reached the operator walk",
                )
                .into());
            }
        };

        let value = self.replace_expression_metadata(value, &span, None, None)?;
        self.log_expression_output(expression, value);
        Ok(LoweredExpression { prelude, value })
    }

    /// Delivers a completed operand to its parent, in the order the recursive lowering used.
    fn deliver_operator_result(
        &mut self,
        pending_prelude: &mut Vec<HirStatement>,
        values: &mut Vec<HirValueId>,
        delivery: OperandDelivery,
        lowered: LoweredExpression,
    ) -> Result<(), HirConstructionFailure> {
        match delivery {
            OperandDelivery::CurrentBlock => {
                for statement in lowered.prelude {
                    self.emit_statement_to_current_block(statement)?;
                }
            }
            OperandDelivery::PendingPrelude => pending_prelude.extend(lowered.prelude),
        }
        values.push(lowered.value);
        Ok(())
    }
}
