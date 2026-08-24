//! Extracted HIR lowering for loop statements.
//!
//! WHAT: lowers range and collection loops into explicit CFG blocks with deterministic runtime
//! semantics.
//! WHY: loop lowering is the densest control-flow transformation in HIR and benefits from one
//! dedicated module boundary.
//!
//! Every preallocated block id here names a jump *target*. Jump *sources* are always the live
//! continuation block, read through `emit_jump_from_current_block`. Lowering an iterable, a range
//! bound, or a compiler-generated checked update can split the block it started in, so a remembered
//! block id is only valid until the next helper that may emit control flow.

use crate::compiler_frontend::ast::ast_nodes::{
    AstNode, LoopBindings, RangeEndKind, RangeLoopSpec, SourceLocation,
};
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::external_packages::{CallTarget, ExternalFunctionId};
use crate::compiler_frontend::hir::blocks::HirLocal;
use crate::compiler_frontend::hir::expressions::{HirExpression, HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::hir_builder::HirBuilder;
use crate::compiler_frontend::hir::hir_side_table::{HirLocalOriginKind, HirLocation};
use crate::compiler_frontend::hir::ids::{BlockId, LocalId, RegionId};
use crate::compiler_frontend::hir::numeric::HirNumericOp;
use crate::compiler_frontend::hir::operators::HirBinOp;
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::statements::HirStatementKind;
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::return_hir_transformation_error;

/// Jump targets of the range-loop CFG pipeline. These are destinations only. The module header
/// explains why the source of each edge is read live instead of stored here.
#[derive(Clone, Copy)]
struct RangeLoopBlocks {
    step_zero_check: BlockId,
    step_zero_failure: BlockId,
    step_abs_check: BlockId,
    step_abs_negate: BlockId,
    direction_check: BlockId,
    descending_negate: BlockId,
    header_selector: BlockId,
    header_ascending: BlockId,
    header_descending: BlockId,
    body: BlockId,
    step: BlockId,
    exit: BlockId,
}

#[derive(Clone, Copy)]
struct RangeLoopLocals {
    current: LocalId,
    end: LocalId,
    step: LocalId,
    ascending: LocalId,
    iteration_index: LocalId,
}

#[derive(Clone, Copy)]
struct RangeLoopTypes {
    binding: TypeId,
    bool_type: TypeId,
    int_type: TypeId,
    float_type: TypeId,
}

#[derive(Clone, Copy)]
struct RangeLoopRuntime {
    blocks: RangeLoopBlocks,
    locals: RangeLoopLocals,
    types: RangeLoopTypes,
}

impl<'a> HirBuilder<'a> {
    pub(super) fn lower_while_statement_impl(
        &mut self,
        condition: &Expression,
        body: &[AstNode],
        location: &SourceLocation,
    ) -> Result<(), CompilerError> {
        self.lower_while_with_body_emitter(condition, location, |builder| {
            builder.lower_statement_sequence(body)
        })
    }

    /// Lowers a conditional loop into CFG while letting callers choose the body emitter.
    ///
    /// Runtime template loops need the same while-header/body/backedge shape as
    /// statement loops, but their body appends render units instead of lowering
    /// statement nodes. Keeping the CFG owner here avoids a second conditional
    /// loop lowering path in the template HIR code.
    pub(crate) fn lower_while_with_body_emitter(
        &mut self,
        condition: &Expression,
        location: &SourceLocation,
        emit_body: impl FnOnce(&mut HirBuilder<'_>) -> Result<(), CompilerError>,
    ) -> Result<(), CompilerError> {
        let parent_region = self.current_region_or_error(location)?;

        let header_block = self.create_block(parent_region, location, "while-header")?;
        let body_region = self.create_child_region(parent_region);
        let body_block = self.create_block(body_region, location, "while-body")?;
        let exit_block = self.create_block(parent_region, location, "while-exit")?;

        self.emit_jump_from_current_block(header_block, location, "while.enter")?;

        self.set_current_block(header_block, location)?;
        let condition_value = self.lower_expression_value_to_current_block(condition)?;
        let condition_block = self.current_block_id_or_error(location)?;

        self.emit_terminator(
            condition_block,
            HirTerminator::If {
                condition: condition_value,
                then_block: body_block,
                else_block: exit_block,
            },
            location,
        )?;

        self.log_control_flow_edge(condition_block, body_block, "while.true");
        self.log_control_flow_edge(condition_block, exit_block, "while.false");

        self.set_current_block(body_block, location)?;
        self.push_loop_targets(exit_block, header_block);
        let body_result = emit_body(self);
        self.pop_loop_targets();
        body_result?;

        let body_tail_block = self.current_block_id_or_error(location)?;
        if !self.block_has_explicit_terminator(body_tail_block, location)? {
            let backedge_block = self.create_block(parent_region, location, "while-backedge")?;
            self.emit_jump_to(
                body_tail_block,
                backedge_block,
                location,
                "while.body.backedge",
            )?;

            self.set_current_block(backedge_block, location)?;
            self.emit_jump_to(backedge_block, header_block, location, "while.backedge")?;
        }

        self.set_current_block(exit_block, location)
    }

    pub(super) fn lower_range_loop_statement_impl(
        &mut self,
        bindings: &LoopBindings,
        range: &RangeLoopSpec,
        body: &[AstNode],
        location: &SourceLocation,
    ) -> Result<(), CompilerError> {
        self.lower_range_loop_with_body_emitter(bindings, range, location, |builder| {
            builder.lower_statement_sequence(body)
        })
    }

    pub(crate) fn lower_range_loop_with_body_emitter(
        &mut self,
        bindings: &LoopBindings,
        range: &RangeLoopSpec,
        location: &SourceLocation,
        mut emit_body: impl FnMut(&mut HirBuilder<'_>) -> Result<(), CompilerError>,
    ) -> Result<(), CompilerError> {
        // Build an explicit CFG pipeline so runtime range semantics are deterministic:
        // zero-step guard -> step normalization -> direction dispatch -> bounds checks.
        let parent_region = self.current_region_or_error(location)?;
        let blocks = self.create_range_loop_blocks(parent_region, location)?;
        let types = self.resolve_range_loop_types(bindings, range, location)?;
        let locals = self.allocate_range_loop_locals(types, location)?;
        let runtime = RangeLoopRuntime {
            blocks,
            locals,
            types,
        };

        self.initialize_range_loop_state(range, runtime, location)?;

        // Dynamic `by` expressions still need a runtime zero check before entering the loop.
        self.emit_jump_from_current_block(runtime.blocks.step_zero_check, location, "for.enter")?;

        self.emit_range_loop_zero_step_guard(runtime, location)?;
        self.emit_range_loop_step_magnitude_normalization(runtime, location)?;
        self.emit_range_loop_direction_dispatch(runtime, location)?;
        self.emit_range_loop_header_checks(range, runtime, location)?;
        let step_block_is_reachable =
            self.lower_range_loop_body_with_emitter(bindings, runtime, location, &mut emit_body)?;
        if step_block_is_reachable {
            self.emit_range_loop_step(runtime, location)?;
        }

        self.set_current_block(runtime.blocks.exit, location)
    }

    fn create_range_loop_blocks(
        &mut self,
        parent_region: RegionId,
        location: &SourceLocation,
    ) -> Result<RangeLoopBlocks, CompilerError> {
        let step_zero_check = self.create_block(parent_region, location, "for-step-zero-check")?;
        let step_zero_failure =
            self.create_block(parent_region, location, "for-step-zero-failure")?;
        let step_abs_check = self.create_block(parent_region, location, "for-step-abs-check")?;
        let step_abs_negate = self.create_block(parent_region, location, "for-step-abs-negate")?;
        let direction_check = self.create_block(parent_region, location, "for-direction-check")?;
        let descending_negate = self.create_block(parent_region, location, "for-desc-negate")?;
        let header_selector = self.create_block(parent_region, location, "for-header-selector")?;
        let header_ascending =
            self.create_block(parent_region, location, "for-header-ascending")?;
        let header_descending =
            self.create_block(parent_region, location, "for-header-descending")?;
        let body_region = self.create_child_region(parent_region);
        let body = self.create_block(body_region, location, "for-body")?;
        let step = self.create_block(parent_region, location, "for-step")?;
        let exit = self.create_block(parent_region, location, "for-exit")?;

        Ok(RangeLoopBlocks {
            step_zero_check,
            step_zero_failure,
            step_abs_check,
            step_abs_negate,
            direction_check,
            descending_negate,
            header_selector,
            header_ascending,
            header_descending,
            body,
            step,
            exit,
        })
    }

    fn resolve_range_loop_types(
        &mut self,
        bindings: &LoopBindings,
        range: &RangeLoopSpec,
        location: &SourceLocation,
    ) -> Result<RangeLoopTypes, CompilerError> {
        let binding = self.range_iteration_type(bindings, range, location)?;
        let int_id = self.type_environment.builtins().int;
        let float_id = self.type_environment.builtins().float;

        if binding != int_id && binding != float_id {
            return_hir_transformation_error!(
                "Range-loop item binding must be Int or Float",
                self.hir_error_location(location)
            );
        }

        Ok(RangeLoopTypes {
            binding,
            bool_type: builtin_type_ids::BOOL,
            int_type: builtin_type_ids::INT,
            float_type: float_id,
        })
    }

    fn allocate_range_loop_locals(
        &mut self,
        types: RangeLoopTypes,
        location: &SourceLocation,
    ) -> Result<RangeLoopLocals, CompilerError> {
        // Allocation order is observable in HIR snapshots and must remain stable.
        let current = self.allocate_temp_local(types.binding, Some(location.clone()))?;
        let end = self.allocate_temp_local(types.binding, Some(location.clone()))?;
        let step = self.allocate_temp_local(types.binding, Some(location.clone()))?;
        let ascending = self.allocate_temp_local(types.bool_type, Some(location.clone()))?;
        let iteration_index = self.allocate_temp_local(types.int_type, Some(location.clone()))?;

        Ok(RangeLoopLocals {
            current,
            end,
            step,
            ascending,
            iteration_index,
        })
    }

    fn initialize_range_loop_state(
        &mut self,
        range: &RangeLoopSpec,
        runtime: RangeLoopRuntime,
        location: &SourceLocation,
    ) -> Result<(), CompilerError> {
        let RangeLoopRuntime { locals, types, .. } = runtime;

        let lowered_start = self.lower_expression_value_to_current_block(&range.start)?;
        self.emit_statement_kind(
            HirStatementKind::Assign {
                target: HirPlace::Local(locals.current),
                value: lowered_start,
            },
            location,
        )?;

        let lowered_end = self.lower_expression_value_to_current_block(&range.end)?;
        self.emit_statement_kind(
            HirStatementKind::Assign {
                target: HirPlace::Local(locals.end),
                value: lowered_end,
            },
            location,
        )?;

        let pre_header_region = self.current_region_or_error(location)?;
        let zero_index = self.make_expression(
            location,
            HirExpressionKind::Int(0),
            types.int_type,
            ValueKind::Const,
            pre_header_region,
        );
        self.emit_statement_kind(
            HirStatementKind::Assign {
                target: HirPlace::Local(locals.iteration_index),
                value: zero_index,
            },
            location,
        )?;

        // `by` is optional for integer ranges; omitted steps default to +1 / +1.0.
        if let Some(step_expression) = &range.step {
            let lowered_step = self.lower_expression_value_to_current_block(step_expression)?;

            self.emit_statement_kind(
                HirStatementKind::Assign {
                    target: HirPlace::Local(locals.step),
                    value: lowered_step,
                },
                location,
            )?;
        } else {
            let default_step = if types.binding == types.float_type {
                self.make_expression(
                    location,
                    HirExpressionKind::Float(1.0),
                    types.binding,
                    ValueKind::Const,
                    pre_header_region,
                )
            } else {
                self.make_expression(
                    location,
                    HirExpressionKind::Int(1),
                    types.binding,
                    ValueKind::Const,
                    pre_header_region,
                )
            };

            self.emit_statement_kind(
                HirStatementKind::Assign {
                    target: HirPlace::Local(locals.step),
                    value: default_step,
                },
                location,
            )?;
        }

        let ascending_current = self.make_expression(
            location,
            HirExpressionKind::Load(HirPlace::Local(locals.current)),
            types.binding,
            ValueKind::Place,
            pre_header_region,
        );
        let ascending_end = self.make_expression(
            location,
            HirExpressionKind::Load(HirPlace::Local(locals.end)),
            types.binding,
            ValueKind::Place,
            pre_header_region,
        );
        let ascending_value = self.make_expression(
            location,
            HirExpressionKind::BinOp {
                left: Box::new(ascending_current),
                op: HirBinOp::Le,
                right: Box::new(ascending_end),
            },
            types.bool_type,
            ValueKind::RValue,
            pre_header_region,
        );
        self.emit_statement_kind(
            HirStatementKind::Assign {
                target: HirPlace::Local(locals.ascending),
                value: ascending_value,
            },
            location,
        )
    }

    fn emit_range_loop_zero_step_guard(
        &mut self,
        runtime: RangeLoopRuntime,
        location: &SourceLocation,
    ) -> Result<(), CompilerError> {
        let RangeLoopRuntime {
            blocks,
            locals,
            types,
        } = runtime;

        self.set_current_block(blocks.step_zero_check, location)?;
        let zero_check_region = self.current_region_or_error(location)?;
        let step_for_zero_check = self.make_expression(
            location,
            HirExpressionKind::Load(HirPlace::Local(locals.step)),
            types.binding,
            ValueKind::Place,
            zero_check_region,
        );
        let zero_literal = self.range_loop_zero_literal(types, location, zero_check_region);
        let step_is_zero = self.make_expression(
            location,
            HirExpressionKind::BinOp {
                left: Box::new(step_for_zero_check),
                op: HirBinOp::Eq,
                right: Box::new(zero_literal),
            },
            types.bool_type,
            ValueKind::RValue,
            zero_check_region,
        );
        self.emit_terminator(
            blocks.step_zero_check,
            HirTerminator::If {
                condition: step_is_zero,
                then_block: blocks.step_zero_failure,
                else_block: blocks.step_abs_check,
            },
            location,
        )?;
        self.log_control_flow_edge(
            blocks.step_zero_check,
            blocks.step_zero_failure,
            "for.step.zero",
        );
        self.log_control_flow_edge(
            blocks.step_zero_check,
            blocks.step_abs_check,
            "for.step.non_zero",
        );

        self.set_current_block(blocks.step_zero_failure, location)?;
        self.emit_terminator(
            blocks.step_zero_failure,
            HirTerminator::RuntimeFailure {
                message: "Loop step cannot be zero".to_owned(),
            },
            location,
        )
    }

    fn emit_range_loop_step_magnitude_normalization(
        &mut self,
        runtime: RangeLoopRuntime,
        location: &SourceLocation,
    ) -> Result<(), CompilerError> {
        let RangeLoopRuntime {
            blocks,
            locals,
            types,
        } = runtime;

        self.set_current_block(blocks.step_abs_check, location)?;
        let abs_check_region = self.current_region_or_error(location)?;
        let step_for_abs_check = self.make_expression(
            location,
            HirExpressionKind::Load(HirPlace::Local(locals.step)),
            types.binding,
            ValueKind::Place,
            abs_check_region,
        );
        let abs_zero_literal = self.range_loop_zero_literal(types, location, abs_check_region);
        let step_is_negative = self.make_expression(
            location,
            HirExpressionKind::BinOp {
                left: Box::new(step_for_abs_check),
                op: HirBinOp::Lt,
                right: Box::new(abs_zero_literal),
            },
            types.bool_type,
            ValueKind::RValue,
            abs_check_region,
        );
        self.emit_terminator(
            blocks.step_abs_check,
            HirTerminator::If {
                condition: step_is_negative,
                then_block: blocks.step_abs_negate,
                else_block: blocks.direction_check,
            },
            location,
        )?;
        self.log_control_flow_edge(
            blocks.step_abs_check,
            blocks.step_abs_negate,
            "for.step.neg",
        );
        self.log_control_flow_edge(
            blocks.step_abs_check,
            blocks.direction_check,
            "for.step.pos",
        );

        // Normalize explicit negative steps to magnitude first.
        self.set_current_block(blocks.step_abs_negate, location)?;
        let abs_negate_region = self.current_region_or_error(location)?;
        let abs_step_current = self.make_expression(
            location,
            HirExpressionKind::Load(HirPlace::Local(locals.step)),
            types.binding,
            ValueKind::Place,
            abs_negate_region,
        );
        let abs_zero = self.range_loop_zero_literal(types, location, abs_negate_region);
        let abs_sub_op = if types.binding == types.float_type {
            HirNumericOp::FloatSub
        } else {
            HirNumericOp::IntSub
        };
        self.emit_checked_numeric_assignment(
            locals.step,
            abs_sub_op,
            abs_zero,
            abs_step_current,
            location,
        )?;
        self.emit_jump_from_current_block(blocks.direction_check, location, "for.step.abs.done")
    }

    fn emit_range_loop_direction_dispatch(
        &mut self,
        runtime: RangeLoopRuntime,
        location: &SourceLocation,
    ) -> Result<(), CompilerError> {
        let RangeLoopRuntime {
            blocks,
            locals,
            types,
        } = runtime;

        self.set_current_block(blocks.direction_check, location)?;
        let direction_check_region = self.current_region_or_error(location)?;
        let ascending_for_direction = self.make_expression(
            location,
            HirExpressionKind::Load(HirPlace::Local(locals.ascending)),
            types.bool_type,
            ValueKind::Place,
            direction_check_region,
        );
        self.emit_terminator(
            blocks.direction_check,
            HirTerminator::If {
                condition: ascending_for_direction,
                then_block: blocks.header_selector,
                else_block: blocks.descending_negate,
            },
            location,
        )?;
        self.log_control_flow_edge(
            blocks.direction_check,
            blocks.header_selector,
            "for.direction.asc",
        );
        self.log_control_flow_edge(
            blocks.direction_check,
            blocks.descending_negate,
            "for.direction.desc",
        );

        // Apply direction after magnitude normalization so descending loops always decrement.
        self.set_current_block(blocks.descending_negate, location)?;
        let desc_negate_region = self.current_region_or_error(location)?;
        let desc_step_current = self.make_expression(
            location,
            HirExpressionKind::Load(HirPlace::Local(locals.step)),
            types.binding,
            ValueKind::Place,
            desc_negate_region,
        );
        let desc_zero = self.range_loop_zero_literal(types, location, desc_negate_region);
        let desc_sub_op = if types.binding == types.float_type {
            HirNumericOp::FloatSub
        } else {
            HirNumericOp::IntSub
        };
        self.emit_checked_numeric_assignment(
            locals.step,
            desc_sub_op,
            desc_zero,
            desc_step_current,
            location,
        )?;
        self.emit_jump_from_current_block(blocks.header_selector, location, "for.direction.done")
    }

    fn emit_range_loop_header_checks(
        &mut self,
        range: &RangeLoopSpec,
        runtime: RangeLoopRuntime,
        location: &SourceLocation,
    ) -> Result<(), CompilerError> {
        let RangeLoopRuntime {
            blocks,
            locals,
            types,
        } = runtime;

        self.set_current_block(blocks.header_selector, location)?;
        let header_selector_region = self.current_region_or_error(location)?;
        let ascending_for_header = self.make_expression(
            location,
            HirExpressionKind::Load(HirPlace::Local(locals.ascending)),
            types.bool_type,
            ValueKind::Place,
            header_selector_region,
        );
        self.emit_terminator(
            blocks.header_selector,
            HirTerminator::If {
                condition: ascending_for_header,
                then_block: blocks.header_ascending,
                else_block: blocks.header_descending,
            },
            location,
        )?;
        self.log_control_flow_edge(
            blocks.header_selector,
            blocks.header_ascending,
            "for.header.asc",
        );
        self.log_control_flow_edge(
            blocks.header_selector,
            blocks.header_descending,
            "for.header.desc",
        );

        // `to` (exclusive) vs `to &` (inclusive) become strict vs inclusive comparators per direction branch.
        self.set_current_block(blocks.header_ascending, location)?;
        let header_ascending_region = self.current_region_or_error(location)?;
        let asc_current_value = self.make_expression(
            location,
            HirExpressionKind::Load(HirPlace::Local(locals.current)),
            types.binding,
            ValueKind::Place,
            header_ascending_region,
        );
        let asc_end_value = self.make_expression(
            location,
            HirExpressionKind::Load(HirPlace::Local(locals.end)),
            types.binding,
            ValueKind::Place,
            header_ascending_region,
        );
        let asc_comparison_op = match range.end_kind {
            RangeEndKind::Exclusive => HirBinOp::Lt,
            RangeEndKind::Inclusive => HirBinOp::Le,
        };
        let asc_condition = self.make_expression(
            location,
            HirExpressionKind::BinOp {
                left: Box::new(asc_current_value),
                op: asc_comparison_op,
                right: Box::new(asc_end_value),
            },
            types.bool_type,
            ValueKind::RValue,
            header_ascending_region,
        );
        self.emit_terminator(
            blocks.header_ascending,
            HirTerminator::If {
                condition: asc_condition,
                then_block: blocks.body,
                else_block: blocks.exit,
            },
            location,
        )?;
        self.log_control_flow_edge(blocks.header_ascending, blocks.body, "for.asc.true");
        self.log_control_flow_edge(blocks.header_ascending, blocks.exit, "for.asc.false");

        self.set_current_block(blocks.header_descending, location)?;
        let header_descending_region = self.current_region_or_error(location)?;
        let desc_current_value = self.make_expression(
            location,
            HirExpressionKind::Load(HirPlace::Local(locals.current)),
            types.binding,
            ValueKind::Place,
            header_descending_region,
        );
        let desc_end_value = self.make_expression(
            location,
            HirExpressionKind::Load(HirPlace::Local(locals.end)),
            types.binding,
            ValueKind::Place,
            header_descending_region,
        );
        let desc_comparison_op = match range.end_kind {
            RangeEndKind::Exclusive => HirBinOp::Gt,
            RangeEndKind::Inclusive => HirBinOp::Ge,
        };
        let desc_condition = self.make_expression(
            location,
            HirExpressionKind::BinOp {
                left: Box::new(desc_current_value),
                op: desc_comparison_op,
                right: Box::new(desc_end_value),
            },
            types.bool_type,
            ValueKind::RValue,
            header_descending_region,
        );
        self.emit_terminator(
            blocks.header_descending,
            HirTerminator::If {
                condition: desc_condition,
                then_block: blocks.body,
                else_block: blocks.exit,
            },
            location,
        )?;
        self.log_control_flow_edge(blocks.header_descending, blocks.body, "for.desc.true");
        self.log_control_flow_edge(blocks.header_descending, blocks.exit, "for.desc.false");

        Ok(())
    }

    fn lower_range_loop_body_with_emitter(
        &mut self,
        bindings: &LoopBindings,
        runtime: RangeLoopRuntime,
        location: &SourceLocation,
        emit_body: &mut impl FnMut(&mut HirBuilder<'_>) -> Result<(), CompilerError>,
    ) -> Result<bool, CompilerError> {
        let RangeLoopRuntime {
            blocks,
            locals,
            types,
        } = runtime;

        self.set_current_block(blocks.body, location)?;
        let body_region_id = self.current_region_or_error(location)?;
        let mut visible_bindings = Vec::new();

        if let Some(item_binding) = &bindings.item {
            let body_current_value = self.make_expression(
                location,
                HirExpressionKind::Load(HirPlace::Local(locals.current)),
                types.binding,
                ValueKind::Place,
                body_region_id,
            );
            let binding = self.register_loop_binding_local(
                item_binding,
                types.binding,
                body_current_value,
                &visible_bindings,
                location,
            )?;
            visible_bindings.push(binding);
        }

        if let Some(index_binding) = &bindings.index {
            let index_value = self.make_expression(
                location,
                HirExpressionKind::Load(HirPlace::Local(locals.iteration_index)),
                types.int_type,
                ValueKind::Place,
                body_region_id,
            );
            let binding = self.register_loop_binding_local(
                index_binding,
                types.int_type,
                index_value,
                &visible_bindings,
                location,
            )?;
            visible_bindings.push(binding);
        }

        self.push_loop_targets(blocks.exit, blocks.step);
        let body_result =
            self.with_temporary_local_bindings(visible_bindings, |builder| emit_body(builder));
        self.pop_loop_targets();
        body_result?;

        let body_tail_block = self.current_block_id_or_error(location)?;
        if !self.block_has_explicit_terminator(body_tail_block, location)? {
            self.emit_jump_to(body_tail_block, blocks.step, location, "for.body.step")?;
        }

        Ok(!self.discard_unreachable_empty_block(blocks.step, location)?)
    }

    fn emit_range_loop_step(
        &mut self,
        runtime: RangeLoopRuntime,
        location: &SourceLocation,
    ) -> Result<(), CompilerError> {
        let RangeLoopRuntime {
            blocks,
            locals,
            types,
        } = runtime;

        self.set_current_block(blocks.step, location)?;
        let step_region = self.current_region_or_error(location)?;
        let step_current = self.make_expression(
            location,
            HirExpressionKind::Load(HirPlace::Local(locals.current)),
            types.binding,
            ValueKind::Place,
            step_region,
        );
        let step_delta = self.make_expression(
            location,
            HirExpressionKind::Load(HirPlace::Local(locals.step)),
            types.binding,
            ValueKind::Place,
            step_region,
        );
        let current_add_op = if types.binding == types.float_type {
            HirNumericOp::FloatAdd
        } else {
            HirNumericOp::IntAdd
        };
        self.emit_checked_numeric_assignment(
            locals.current,
            current_add_op,
            step_current,
            step_delta,
            location,
        )?;

        let index_current = self.make_expression(
            location,
            HirExpressionKind::Load(HirPlace::Local(locals.iteration_index)),
            types.int_type,
            ValueKind::Place,
            step_region,
        );
        let index_delta = self.make_expression(
            location,
            HirExpressionKind::Int(1),
            types.int_type,
            ValueKind::Const,
            step_region,
        );
        self.emit_checked_numeric_assignment(
            locals.iteration_index,
            HirNumericOp::IntAdd,
            index_current,
            index_delta,
            location,
        )?;
        self.emit_jump_from_current_block(blocks.header_selector, location, "for.backedge")
    }

    fn range_loop_zero_literal(
        &mut self,
        types: RangeLoopTypes,
        location: &SourceLocation,
        region: RegionId,
    ) -> HirExpression {
        if types.binding == types.float_type {
            self.make_expression(
                location,
                HirExpressionKind::Float(0.0),
                types.binding,
                ValueKind::Const,
                region,
            )
        } else {
            self.make_expression(
                location,
                HirExpressionKind::Int(0),
                types.binding,
                ValueKind::Const,
                region,
            )
        }
    }

    pub(super) fn lower_collection_loop_statement_impl(
        &mut self,
        bindings: &LoopBindings,
        iterable: &Expression,
        body: &[AstNode],
        location: &SourceLocation,
    ) -> Result<(), CompilerError> {
        self.lower_collection_loop_with_body_emitter(bindings, iterable, location, |builder| {
            builder.lower_statement_sequence(body)
        })
    }

    pub(crate) fn lower_collection_loop_with_body_emitter(
        &mut self,
        bindings: &LoopBindings,
        iterable: &Expression,
        location: &SourceLocation,
        mut emit_body: impl FnMut(&mut HirBuilder<'_>) -> Result<(), CompilerError>,
    ) -> Result<(), CompilerError> {
        let parent_region = self.current_region_or_error(location)?;

        let header_block = self.create_block(parent_region, location, "loop-collection-header")?;
        let body_region = self.create_child_region(parent_region);
        let body_block = self.create_block(body_region, location, "loop-collection-body")?;
        let step_block = self.create_block(parent_region, location, "loop-collection-step")?;
        let exit_block = self.create_block(parent_region, location, "loop-collection-exit")?;

        let (iterable_type, element_type) = self.collection_iteration_types(iterable, location)?;

        let bool_ty = builtin_type_ids::BOOL;
        let int_ty: TypeId = builtin_type_ids::INT;
        let iterable_local = self.allocate_temp_local(iterable_type, Some(location.to_owned()))?;
        let length_local = self.allocate_temp_local(int_ty, Some(location.to_owned()))?;
        let iteration_index_local = self.allocate_temp_local(int_ty, Some(location.to_owned()))?;

        let lowered_iterable = self.lower_expression_value_to_current_block(iterable)?;
        self.emit_statement_kind(
            HirStatementKind::Assign {
                target: HirPlace::Local(iterable_local),
                value: lowered_iterable,
            },
            location,
        )?;

        let pre_header_region = self.current_region_or_error(location)?;
        let zero_index = self.make_expression(
            location,
            HirExpressionKind::Int(0),
            int_ty,
            ValueKind::Const,
            pre_header_region,
        );
        self.emit_statement_kind(
            HirStatementKind::Assign {
                target: HirPlace::Local(iteration_index_local),
                value: zero_index,
            },
            location,
        )?;

        let iterable_for_length = self.make_expression(
            location,
            HirExpressionKind::Load(HirPlace::Local(iterable_local)),
            iterable_type,
            ValueKind::Place,
            pre_header_region,
        );
        self.emit_statement_kind(
            HirStatementKind::Call {
                target: CallTarget::External(ExternalFunctionId::CollectionLength),
                args: vec![iterable_for_length],
                result: Some(length_local),
            },
            location,
        )?;

        self.emit_jump_from_current_block(header_block, location, "loop.collection.enter")?;

        self.set_current_block(header_block, location)?;
        let header_region = self.current_region_or_error(location)?;
        let current_index = self.make_expression(
            location,
            HirExpressionKind::Load(HirPlace::Local(iteration_index_local)),
            int_ty,
            ValueKind::Place,
            header_region,
        );
        let collection_length = self.make_expression(
            location,
            HirExpressionKind::Load(HirPlace::Local(length_local)),
            int_ty,
            ValueKind::Place,
            header_region,
        );
        let continue_condition = self.make_expression(
            location,
            HirExpressionKind::BinOp {
                left: Box::new(current_index),
                op: HirBinOp::Lt,
                right: Box::new(collection_length),
            },
            bool_ty,
            ValueKind::RValue,
            header_region,
        );
        self.emit_terminator(
            header_block,
            HirTerminator::If {
                condition: continue_condition,
                then_block: body_block,
                else_block: exit_block,
            },
            location,
        )?;
        self.log_control_flow_edge(header_block, body_block, "loop.collection.true");
        self.log_control_flow_edge(header_block, exit_block, "loop.collection.false");

        self.set_current_block(body_block, location)?;
        let body_region_id = self.current_region_or_error(location)?;
        let mut visible_bindings = Vec::new();

        if let Some(item_binding) = &bindings.item {
            let item_index = self.make_expression(
                location,
                HirExpressionKind::Load(HirPlace::Local(iteration_index_local)),
                int_ty,
                ValueKind::Place,
                body_region_id,
            );
            let item_place = HirPlace::Index {
                base: Box::new(HirPlace::Local(iterable_local)),
                index: Box::new(item_index),
            };
            let item_value = self.make_expression(
                location,
                HirExpressionKind::Load(item_place),
                element_type,
                ValueKind::Place,
                body_region_id,
            );
            let binding = self.register_loop_binding_local(
                item_binding,
                element_type,
                item_value,
                &visible_bindings,
                location,
            )?;
            visible_bindings.push(binding);
        }

        if let Some(index_binding) = &bindings.index {
            let user_index_value = self.make_expression(
                location,
                HirExpressionKind::Load(HirPlace::Local(iteration_index_local)),
                int_ty,
                ValueKind::Place,
                body_region_id,
            );
            let binding = self.register_loop_binding_local(
                index_binding,
                int_ty,
                user_index_value,
                &visible_bindings,
                location,
            )?;
            visible_bindings.push(binding);
        }

        self.push_loop_targets(exit_block, step_block);
        let body_result =
            self.with_temporary_local_bindings(visible_bindings, |builder| emit_body(builder));
        self.pop_loop_targets();
        body_result?;

        let body_tail_block = self.current_block_id_or_error(location)?;
        if !self.block_has_explicit_terminator(body_tail_block, location)? {
            self.emit_jump_to(
                body_tail_block,
                step_block,
                location,
                "loop.collection.body.step",
            )?;
        }

        if self.discard_unreachable_empty_block(step_block, location)? {
            return self.set_current_block(exit_block, location);
        }

        self.set_current_block(step_block, location)?;
        let step_region = self.current_region_or_error(location)?;
        let step_current = self.make_expression(
            location,
            HirExpressionKind::Load(HirPlace::Local(iteration_index_local)),
            int_ty,
            ValueKind::Place,
            step_region,
        );
        let step_delta = self.make_expression(
            location,
            HirExpressionKind::Int(1),
            int_ty,
            ValueKind::Const,
            step_region,
        );
        self.emit_checked_numeric_assignment(
            iteration_index_local,
            HirNumericOp::IntAdd,
            step_current,
            step_delta,
            location,
        )?;
        self.emit_jump_from_current_block(header_block, location, "loop.collection.backedge")?;

        self.set_current_block(exit_block, location)
    }

    fn register_loop_binding_local(
        &mut self,
        binding: &crate::compiler_frontend::ast::ast_nodes::Declaration,
        ty: TypeId,
        value: HirExpression,
        visible_bindings: &[(InternedPath, LocalId)],
        location: &SourceLocation,
    ) -> Result<(InternedPath, LocalId), CompilerError> {
        // AST scopes already enforce no-shadowing. This guard keeps HIR honest if a
        // malformed AST ever tries to bind a loop name over an already-visible local.
        if self.locals_by_name.contains_key(&binding.id)
            || visible_bindings.iter().any(|(path, _)| path == &binding.id)
        {
            return_hir_transformation_error!(
                format!(
                    "Local '{}' is already declared in this function scope",
                    self.symbol_name_for_diagnostics(&binding.id)
                ),
                self.hir_error_location(location)
            );
        }

        let region = self.current_region_or_error(location)?;
        let block_id = self.current_block_id_or_error(location)?;
        let local_id = self.allocate_local_id();
        let local = HirLocal {
            id: local_id,
            ty,
            mutable: false,
            region,
            source_info: Some(location.clone()),
        };

        self.side_table.map_local_source(&local);
        self.register_local_in_block(block_id, local, location)?;
        self.side_table
            .bind_local_name(local_id, binding.id.clone());
        self.side_table
            .bind_local_origin(local_id, HirLocalOriginKind::User, None, None);
        self.side_table
            .map_ast_to_hir(location, HirLocation::Local(local_id));

        self.emit_statement_kind(
            HirStatementKind::Assign {
                target: HirPlace::Local(local_id),
                value,
            },
            location,
        )?;

        Ok((binding.id.clone(), local_id))
    }

    fn range_iteration_type(
        &mut self,
        bindings: &LoopBindings,
        range: &RangeLoopSpec,
        location: &SourceLocation,
    ) -> Result<TypeId, CompilerError> {
        if let Some(item_binding) = &bindings.item {
            return self.lower_type_id(item_binding.value.type_id, location);
        }

        let start_ty = self.lower_type_id(range.start.type_id, location)?;
        let end_ty = self.lower_type_id(range.end.type_id, location)?;
        let step_ty = range
            .step
            .as_ref()
            .map(|step| self.lower_type_id(step.type_id, location))
            .transpose()?;

        let is_numeric = |ty: TypeId, this: &Self| {
            let int_id = this.type_environment.builtins().int;
            let float_id = this.type_environment.builtins().float;
            ty == int_id || ty == float_id
        };

        if !is_numeric(start_ty, self) || !is_numeric(end_ty, self) {
            return_hir_transformation_error!(
                "Range loop bounds must lower to numeric HIR types",
                self.hir_error_location(location)
            );
        }

        if let Some(step_ty) = step_ty
            && !is_numeric(step_ty, self)
        {
            return_hir_transformation_error!(
                "Range loop step must lower to a numeric HIR type",
                self.hir_error_location(location)
            );
        }

        let float_id = self.type_environment.builtins().float;
        let uses_float =
            start_ty == float_id || end_ty == float_id || step_ty.is_some_and(|ty| ty == float_id);

        Ok(if uses_float {
            float_id
        } else {
            self.type_environment.builtins().int
        })
    }

    fn collection_iteration_types(
        &mut self,
        iterable: &Expression,
        location: &SourceLocation,
    ) -> Result<(TypeId, TypeId), CompilerError> {
        // Frontend type resolution canonicalizes reference wrappers, so collection loops
        // accept either direct collections or shared references to collections uniformly.
        let iterable_type = self.lower_type_id(iterable.type_id, location)?;
        let element = match self.type_environment.collection_element_type(iterable_type) {
            Some(element) => element,
            None => {
                return_hir_transformation_error!(
                    "Collection loop iterable did not lower to a collection HIR type",
                    self.hir_error_location(location)
                );
            }
        };

        Ok((iterable_type, element))
    }
}
