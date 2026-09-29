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
    AstNode, LoopBindings, RangeEndKind, RangeLoopSpec,
};
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::fixed_scalar::{
    FixedScalar, FixedScalarClass, FixedScalarValue,
};
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::datatypes::numeric_operators::{
    NumericOperator, binary_operation_domain,
};
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
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
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::return_hir_transformation_error;

/// Jump targets of the range-loop CFG pipeline. These are destinations only. The module header
/// explains why the source of each edge is read live instead of stored here.
#[derive(Clone, Copy)]
struct RangeLoopBlocks {
    step_zero_check: BlockId,
    step_zero_failure: BlockId,
    step_abs_check: BlockId,
    step_abs_negate: BlockId,
    header_selector: BlockId,
    header_ascending: BlockId,
    header_descending: BlockId,
    body: BlockId,
    step: BlockId,
    step_ascending_check: BlockId,
    step_descending_check: BlockId,
    step_ascending_distance: BlockId,
    step_descending_distance: BlockId,
    step_distance_check: BlockId,
    step_candidate_direction: BlockId,
    step_candidate_ascending: BlockId,
    step_candidate_descending: BlockId,
    step_candidate_check: BlockId,
    step_candidate_ascending_check: BlockId,
    step_candidate_descending_check: BlockId,
    step_stall_check: BlockId,
    step_stall_failure: BlockId,
    step_commit: BlockId,
    exit: BlockId,
}

#[derive(Clone, Copy)]
struct RangeLoopLocals {
    current: LocalId,
    end: LocalId,
    step: LocalId,
    ascending: LocalId,
    iteration_index: LocalId,
    distance: Option<LocalId>,
    next_value: LocalId,
    candidate_in_range: Option<LocalId>,
}

#[derive(Clone, Copy)]
struct RangeLoopTypes {
    binding: TypeId,
    domain: NumericScalar,
    bool_type: TypeId,
    int_type: TypeId,
}

#[derive(Clone, Copy)]
struct RangeLoopRuntime {
    blocks: RangeLoopBlocks,
    locals: RangeLoopLocals,
    types: RangeLoopTypes,
}

fn uses_float_range_candidate(domain: NumericScalar) -> bool {
    matches!(
        domain,
        NumericScalar::Float | NumericScalar::Fixed(FixedScalar::F32 | FixedScalar::F64)
    )
}

impl<'a> HirBuilder<'a> {
    pub(super) fn lower_while_statement_impl(
        &mut self,
        condition: &Expression,
        body: &[AstNode],
        span_ref: &Option<SourceSpan>,
        span: Option<SourceSpan>,
    ) -> Result<(), CompilerError> {
        self.lower_while_with_body_emitter(condition, span_ref, span, |builder| {
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
        span_ref: &Option<SourceSpan>,
        span: Option<SourceSpan>,
        emit_body: impl FnOnce(&mut HirBuilder<'_>) -> Result<(), CompilerError>,
    ) -> Result<(), CompilerError> {
        let parent_region = self.current_region_or_error(span_ref)?;

        let header_block = self.create_block(parent_region, span_ref, "while-header")?;
        let body_region = self.create_child_region(parent_region);
        let body_block = self.create_block(body_region, span_ref, "while-body")?;
        let exit_block = self.create_block(parent_region, span_ref, "while-exit")?;

        self.emit_jump_from_current_block(header_block, span_ref, "while.enter")?;

        self.set_current_block(header_block, span_ref)?;
        let condition_value = self.lower_expression_value_to_current_block(condition)?;
        let condition_block = self.current_block_id_or_error(span_ref)?;

        self.emit_terminator_with_span(
            condition_block,
            HirTerminator::If {
                condition: condition_value,
                then_block: body_block,
                else_block: exit_block,
            },
            span_ref,
            span,
        )?;

        self.log_control_flow_edge(condition_block, body_block, "while.true");
        self.log_control_flow_edge(condition_block, exit_block, "while.false");

        self.set_current_block(body_block, span_ref)?;
        self.push_loop_targets(exit_block, header_block);
        let body_result = emit_body(self);
        self.pop_loop_targets();
        body_result?;

        let body_tail_block = self.current_block_id_or_error(span_ref)?;
        if !self.block_has_explicit_terminator(body_tail_block, span_ref)? {
            let backedge_block = self.create_block(parent_region, span_ref, "while-backedge")?;
            self.emit_jump_to(
                body_tail_block,
                backedge_block,
                span_ref,
                "while.body.backedge",
            )?;

            self.set_current_block(backedge_block, span_ref)?;
            self.emit_jump_to(backedge_block, header_block, span_ref, "while.backedge")?;
        }

        self.set_current_block(exit_block, span_ref)
    }

    pub(super) fn lower_range_loop_statement_impl(
        &mut self,
        bindings: &LoopBindings,
        range: &RangeLoopSpec,
        body: &[AstNode],
        span_ref: &Option<SourceSpan>,
        span: Option<SourceSpan>,
    ) -> Result<(), CompilerError> {
        self.lower_range_loop_with_body_emitter_and_span(
            bindings,
            range,
            span_ref,
            span,
            |builder| builder.lower_statement_sequence(body),
        )
    }

    pub(crate) fn lower_range_loop_with_body_emitter(
        &mut self,
        bindings: &LoopBindings,
        range: &RangeLoopSpec,
        span_ref: &Option<SourceSpan>,
        mut emit_body: impl FnMut(&mut HirBuilder<'_>) -> Result<(), CompilerError>,
    ) -> Result<(), CompilerError> {
        // Template loops are generated scaffolding: headers stay spanless. Statement
        // loops use the span-aware entry below with the authored `for` span.
        self.lower_range_loop_with_body_emitter_and_span(
            bindings,
            range,
            span_ref,
            None,
            &mut emit_body,
        )
    }

    fn lower_range_loop_with_body_emitter_and_span(
        &mut self,
        bindings: &LoopBindings,
        range: &RangeLoopSpec,
        span_ref: &Option<SourceSpan>,
        span: Option<SourceSpan>,
        mut emit_body: impl FnMut(&mut HirBuilder<'_>) -> Result<(), CompilerError>,
    ) -> Result<(), CompilerError> {
        // Build a CFG pipeline so each update is checked only if a next item remains:
        // zero-step guard -> magnitude normalization -> bounds-safe candidate update.
        let parent_region = self.current_region_or_error(span_ref)?;
        let blocks = self.create_range_loop_blocks(parent_region, span_ref)?;
        let types = self.resolve_range_loop_types(bindings, range, span_ref)?;
        let locals = self.allocate_range_loop_locals(types)?;
        let runtime = RangeLoopRuntime {
            blocks,
            locals,
            types,
        };

        self.initialize_range_loop_state(range, runtime, span_ref)?;

        // Dynamic `by` expressions still need a runtime zero check before entering the loop.
        self.emit_jump_from_current_block(runtime.blocks.step_zero_check, span_ref, "for.enter")?;

        self.emit_range_loop_zero_step_guard(runtime, span_ref)?;
        self.emit_range_loop_step_magnitude_normalization(runtime, span_ref)?;
        self.emit_range_loop_header_checks(range, runtime, span_ref, span)?;
        let step_block_is_reachable =
            self.lower_range_loop_body_with_emitter(bindings, runtime, span_ref, &mut emit_body)?;
        if step_block_is_reachable {
            self.emit_range_loop_step(runtime, range.end_kind, span_ref)?;
        } else {
            self.discard_unreachable_range_step_blocks(blocks, span_ref)?;
        }

        self.set_current_block(runtime.blocks.exit, span_ref)
    }

    fn create_range_loop_blocks(
        &mut self,
        parent_region: RegionId,
        span_ref: &Option<SourceSpan>,
    ) -> Result<RangeLoopBlocks, CompilerError> {
        let step_zero_check = self.create_block(parent_region, span_ref, "for-step-zero-check")?;
        let step_zero_failure =
            self.create_block(parent_region, span_ref, "for-step-zero-failure")?;
        let step_abs_check = self.create_block(parent_region, span_ref, "for-step-abs-check")?;
        let step_abs_negate = self.create_block(parent_region, span_ref, "for-step-abs-negate")?;
        let header_selector = self.create_block(parent_region, span_ref, "for-header-selector")?;
        let header_ascending =
            self.create_block(parent_region, span_ref, "for-header-ascending")?;
        let header_descending =
            self.create_block(parent_region, span_ref, "for-header-descending")?;
        let body_region = self.create_child_region(parent_region);
        let body = self.create_block(body_region, span_ref, "for-body")?;
        let step = self.create_block(parent_region, span_ref, "for-step")?;
        let step_ascending_check =
            self.create_block(parent_region, span_ref, "for-step-ascending-check")?;
        let step_descending_check =
            self.create_block(parent_region, span_ref, "for-step-descending-check")?;
        let step_ascending_distance =
            self.create_block(parent_region, span_ref, "for-step-ascending-distance")?;
        let step_descending_distance =
            self.create_block(parent_region, span_ref, "for-step-descending-distance")?;
        let step_distance_check =
            self.create_block(parent_region, span_ref, "for-step-distance-check")?;
        let step_candidate_direction =
            self.create_block(parent_region, span_ref, "for-step-candidate-direction")?;
        let step_candidate_ascending =
            self.create_block(parent_region, span_ref, "for-step-candidate-ascending")?;
        let step_candidate_descending =
            self.create_block(parent_region, span_ref, "for-step-candidate-descending")?;
        let step_candidate_check =
            self.create_block(parent_region, span_ref, "for-step-candidate-check")?;
        let step_candidate_ascending_check = self.create_block(
            parent_region,
            span_ref,
            "for-step-candidate-ascending-check",
        )?;
        let step_candidate_descending_check = self.create_block(
            parent_region,
            span_ref,
            "for-step-candidate-descending-check",
        )?;
        let step_stall_check =
            self.create_block(parent_region, span_ref, "for-step-stall-check")?;
        let step_stall_failure =
            self.create_block(parent_region, span_ref, "for-step-stall-failure")?;
        let step_commit = self.create_block(parent_region, span_ref, "for-step-commit")?;
        let exit = self.create_block(parent_region, span_ref, "for-exit")?;

        Ok(RangeLoopBlocks {
            step_zero_check,
            step_zero_failure,
            step_abs_check,
            step_abs_negate,
            header_selector,
            header_ascending,
            header_descending,
            body,
            step,
            step_ascending_check,
            step_descending_check,
            step_ascending_distance,
            step_descending_distance,
            step_distance_check,
            step_candidate_direction,
            step_candidate_ascending,
            step_candidate_descending,
            step_candidate_check,
            step_candidate_ascending_check,
            step_candidate_descending_check,
            step_stall_check,
            step_stall_failure,
            step_commit,
            exit,
        })
    }

    fn discard_unreachable_range_step_blocks(
        &mut self,
        blocks: RangeLoopBlocks,
        span_ref: &Option<SourceSpan>,
    ) -> Result<(), CompilerError> {
        for block in [
            blocks.step_ascending_check,
            blocks.step_descending_check,
            blocks.step_ascending_distance,
            blocks.step_descending_distance,
            blocks.step_distance_check,
            blocks.step_candidate_direction,
            blocks.step_candidate_ascending,
            blocks.step_candidate_descending,
            blocks.step_candidate_check,
            blocks.step_candidate_ascending_check,
            blocks.step_candidate_descending_check,
            blocks.step_stall_check,
            blocks.step_stall_failure,
            blocks.step_commit,
        ] {
            if !self.discard_unreachable_empty_block(block, span_ref)? {
                return_hir_transformation_error!(
                    "Unreachable range-loop step block was not empty and unowned",
                    self.hir_error_location(span_ref)
                );
            }
        }

        Ok(())
    }

    fn resolve_range_loop_types(
        &mut self,
        bindings: &LoopBindings,
        range: &RangeLoopSpec,
        span_ref: &Option<SourceSpan>,
    ) -> Result<RangeLoopTypes, CompilerError> {
        let binding = self.range_iteration_type(bindings, range, span_ref)?;
        let domain = NumericScalar::from_type_id(binding, &self.type_environment);

        let Some(domain) = domain else {
            return_hir_transformation_error!(
                "Range-loop item binding must use a numeric scalar type",
                self.hir_error_location(span_ref)
            );
        };

        Ok(RangeLoopTypes {
            binding,
            domain,
            bool_type: builtin_type_ids::BOOL,
            int_type: builtin_type_ids::INT,
        })
    }

    fn numeric_op_for_binding(&self, binding: TypeId, operator: NumericOperator) -> HirNumericOp {
        let domain = NumericScalar::from_type_id(binding, &self.type_environment)
            .expect("range-loop binding is validated as a numeric scalar before lowering");

        HirNumericOp { operator, domain }
    }

    fn allocate_range_loop_locals(
        &mut self,
        types: RangeLoopTypes,
    ) -> Result<RangeLoopLocals, CompilerError> {
        // Allocation order is observable in HIR snapshots and must remain stable.
        let current = self.allocate_temp_local(types.binding, None)?;
        let end = self.allocate_temp_local(types.binding, None)?;
        let step = self.allocate_temp_local(types.binding, None)?;
        let ascending = self.allocate_temp_local(types.bool_type, None)?;
        let iteration_index = self.allocate_temp_local(types.int_type, None)?;
        let distance = if uses_float_range_candidate(types.domain) {
            None
        } else {
            Some(self.allocate_temp_local(types.binding, None)?)
        };
        let next_value = self.allocate_temp_local(types.binding, None)?;
        let candidate_in_range = if uses_float_range_candidate(types.domain) {
            Some(self.allocate_temp_local(types.bool_type, None)?)
        } else {
            None
        };

        Ok(RangeLoopLocals {
            current,
            end,
            step,
            ascending,
            iteration_index,
            distance,
            next_value,
            candidate_in_range,
        })
    }

    /// Snapshots source places so body mutations cannot change the evaluated range state.
    fn snapshot_range_loop_operand(&mut self, value: HirExpression) -> HirExpression {
        match value {
            HirExpression {
                kind: HirExpressionKind::Load(place),
                ty,
                value_kind: ValueKind::Place,
                region,
                span,
                ..
            } => self.make_expression(
                &span,
                HirExpressionKind::Copy(place),
                ty,
                ValueKind::RValue,
                region,
            ),
            value => value,
        }
    }

    fn initialize_range_loop_state(
        &mut self,
        range: &RangeLoopSpec,
        runtime: RangeLoopRuntime,
        span_ref: &Option<SourceSpan>,
    ) -> Result<(), CompilerError> {
        // Generated loop-state spills: `current`/`end`/`step`/`ascending`/`index`
        // temps are compiler scaffolding, so these assignments stay spanless even
        // though the lowered bound values themselves carry expression spans.
        // Only the header bounds checks below carry the authored `for` span.
        let RangeLoopRuntime { locals, types, .. } = runtime;

        let lowered_start = self.lower_expression_value_to_current_block(&range.start)?;
        let lowered_start = self.snapshot_range_loop_operand(lowered_start);
        let lowered_start =
            self.convert_numeric_operand_to_domain(lowered_start, types.domain, span_ref)?;
        self.emit_statement_kind(
            HirStatementKind::Assign {
                target: HirPlace::Local(locals.current),
                value: lowered_start,
            },
            span_ref,
        )?;

        let lowered_end = self.lower_expression_value_to_current_block(&range.end)?;
        let lowered_end = self.snapshot_range_loop_operand(lowered_end);
        let lowered_end =
            self.convert_numeric_operand_to_domain(lowered_end, types.domain, span_ref)?;
        self.emit_statement_kind(
            HirStatementKind::Assign {
                target: HirPlace::Local(locals.end),
                value: lowered_end,
            },
            span_ref,
        )?;

        // `by` is optional for integer ranges; its unit default belongs to the selected domain.
        if let Some(step_expression) = &range.step {
            let lowered_step = self.lower_expression_value_to_current_block(step_expression)?;
            let lowered_step = self.snapshot_range_loop_operand(lowered_step);
            let lowered_step =
                self.convert_numeric_operand_to_domain(lowered_step, types.domain, span_ref)?;

            self.emit_statement_kind(
                HirStatementKind::Assign {
                    target: HirPlace::Local(locals.step),
                    value: lowered_step,
                },
                span_ref,
            )?;
        } else {
            let step_region = self.current_region_or_error(span_ref)?;
            let default_step = self.range_loop_unit_literal(types, span_ref, step_region);

            self.emit_statement_kind(
                HirStatementKind::Assign {
                    target: HirPlace::Local(locals.step),
                    value: default_step,
                },
                span_ref,
            )?;
        }
        let pre_header_region = self.current_region_or_error(span_ref)?;
        let zero_index = self.make_expression(
            span_ref,
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
            span_ref,
        )?;

        let ascending_current = self.make_expression(
            span_ref,
            HirExpressionKind::Load(HirPlace::Local(locals.current)),
            types.binding,
            ValueKind::Place,
            pre_header_region,
        );
        let ascending_end = self.make_expression(
            span_ref,
            HirExpressionKind::Load(HirPlace::Local(locals.end)),
            types.binding,
            ValueKind::Place,
            pre_header_region,
        );
        let ascending_value = self.make_expression(
            span_ref,
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
            span_ref,
        )
    }

    fn emit_range_loop_zero_step_guard(
        &mut self,
        runtime: RangeLoopRuntime,
        span_ref: &Option<SourceSpan>,
    ) -> Result<(), CompilerError> {
        // Generated safety guard: the zero-step check and its `RuntimeFailure`
        // are compiler scaffolding and stay spanless.
        let RangeLoopRuntime {
            blocks,
            locals,
            types,
        } = runtime;
        self.set_current_block(blocks.step_zero_check, span_ref)?;
        let zero_check_region = self.current_region_or_error(span_ref)?;
        let step_for_zero_check = self.make_expression(
            span_ref,
            HirExpressionKind::Load(HirPlace::Local(locals.step)),
            types.binding,
            ValueKind::Place,
            zero_check_region,
        );
        let zero_literal = self.range_loop_zero_literal(types, span_ref, zero_check_region);
        let step_is_zero = self.make_expression(
            span_ref,
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
            span_ref,
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

        self.set_current_block(blocks.step_zero_failure, span_ref)?;
        self.emit_terminator(
            blocks.step_zero_failure,
            HirTerminator::RuntimeFailure {
                message: "Loop step cannot be zero".to_owned(),
            },
            span_ref,
        )
    }

    fn emit_range_loop_step_magnitude_normalization(
        &mut self,
        runtime: RangeLoopRuntime,
        span_ref: &Option<SourceSpan>,
    ) -> Result<(), CompilerError> {
        // Generated normalization: magnitude/direction CFG and checked numeric
        // step updates are compiler scaffolding and stay spanless.
        let RangeLoopRuntime {
            blocks,
            locals,
            types,
        } = runtime;

        self.set_current_block(blocks.step_abs_check, span_ref)?;
        let abs_check_region = self.current_region_or_error(span_ref)?;
        let step_for_abs_check = self.make_expression(
            span_ref,
            HirExpressionKind::Load(HirPlace::Local(locals.step)),
            types.binding,
            ValueKind::Place,
            abs_check_region,
        );
        let abs_zero_literal = self.range_loop_zero_literal(types, span_ref, abs_check_region);
        let step_is_negative = self.make_expression(
            span_ref,
            HirExpressionKind::BinOp {
                left: Box::new(step_for_abs_check),
                op: HirBinOp::Lt,
                right: Box::new(abs_zero_literal),
            },
            types.bool_type,
            ValueKind::RValue,
            abs_check_region,
        );
        self.emit_range_loop_branch(
            step_is_negative,
            blocks.step_abs_negate,
            blocks.header_selector,
            span_ref,
            "for.step.neg",
            "for.step.pos",
        )?;

        // Normalize explicit negative steps to magnitude first.
        self.set_current_block(blocks.step_abs_negate, span_ref)?;
        let abs_negate_region = self.current_region_or_error(span_ref)?;
        let abs_step_current = self.make_expression(
            span_ref,
            HirExpressionKind::Load(HirPlace::Local(locals.step)),
            types.binding,
            ValueKind::Place,
            abs_negate_region,
        );
        let abs_zero = self.range_loop_zero_literal(types, span_ref, abs_negate_region);
        let abs_sub_op = self.numeric_op_for_binding(types.binding, NumericOperator::Subtract);
        self.emit_checked_numeric_assignment(
            locals.step,
            abs_sub_op,
            abs_zero,
            abs_step_current,
            span_ref,
        )?;
        self.emit_jump_from_current_block(blocks.header_selector, span_ref, "for.step.abs.done")
    }

    fn emit_range_loop_header_checks(
        &mut self,
        range: &RangeLoopSpec,
        runtime: RangeLoopRuntime,
        span_ref: &Option<SourceSpan>,
        span: Option<SourceSpan>,
    ) -> Result<(), CompilerError> {
        let RangeLoopRuntime {
            blocks,
            locals,
            types,
        } = runtime;

        self.set_current_block(blocks.header_selector, span_ref)?;
        let header_selector_region = self.current_region_or_error(span_ref)?;
        let ascending_for_header = self.make_expression(
            span_ref,
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
            span_ref,
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
        self.set_current_block(blocks.header_ascending, span_ref)?;
        let header_ascending_region = self.current_region_or_error(span_ref)?;
        let asc_current_value = self.make_expression(
            span_ref,
            HirExpressionKind::Load(HirPlace::Local(locals.current)),
            types.binding,
            ValueKind::Place,
            header_ascending_region,
        );
        let asc_end_value = self.make_expression(
            span_ref,
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
            span_ref,
            HirExpressionKind::BinOp {
                left: Box::new(asc_current_value),
                op: asc_comparison_op,
                right: Box::new(asc_end_value),
            },
            types.bool_type,
            ValueKind::RValue,
            header_ascending_region,
        );
        // Authored loop header: the ascending bounds check carries the `for`
        // statement span. The selector dispatch above is generated scaffolding
        // and stays spanless.
        self.emit_terminator_with_span(
            blocks.header_ascending,
            HirTerminator::If {
                condition: asc_condition,
                then_block: blocks.body,
                else_block: blocks.exit,
            },
            span_ref,
            span.or(range.end.span),
        )?;
        self.log_control_flow_edge(blocks.header_ascending, blocks.body, "for.asc.true");
        self.log_control_flow_edge(blocks.header_ascending, blocks.exit, "for.asc.false");

        self.set_current_block(blocks.header_descending, span_ref)?;
        let header_descending_region = self.current_region_or_error(span_ref)?;
        let desc_current_value = self.make_expression(
            span_ref,
            HirExpressionKind::Load(HirPlace::Local(locals.current)),
            types.binding,
            ValueKind::Place,
            header_descending_region,
        );
        let desc_end_value = self.make_expression(
            span_ref,
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
            span_ref,
            HirExpressionKind::BinOp {
                left: Box::new(desc_current_value),
                op: desc_comparison_op,
                right: Box::new(desc_end_value),
            },
            types.bool_type,
            ValueKind::RValue,
            header_descending_region,
        );
        // Authored loop header: the descending bounds check carries the `for`
        // statement span.
        self.emit_terminator_with_span(
            blocks.header_descending,
            HirTerminator::If {
                condition: desc_condition,
                then_block: blocks.body,
                else_block: blocks.exit,
            },
            span_ref,
            span.or(range.end.span),
        )?;
        self.log_control_flow_edge(blocks.header_descending, blocks.body, "for.desc.true");
        self.log_control_flow_edge(blocks.header_descending, blocks.exit, "for.desc.false");

        Ok(())
    }

    fn lower_range_loop_body_with_emitter(
        &mut self,
        bindings: &LoopBindings,
        runtime: RangeLoopRuntime,
        span_ref: &Option<SourceSpan>,
        emit_body: &mut impl FnMut(&mut HirBuilder<'_>) -> Result<(), CompilerError>,
    ) -> Result<bool, CompilerError> {
        let RangeLoopRuntime {
            blocks,
            locals,
            types,
        } = runtime;

        self.set_current_block(blocks.body, span_ref)?;
        let body_region_id = self.current_region_or_error(span_ref)?;
        let mut visible_bindings = Vec::new();

        if let Some(item_binding) = &bindings.item {
            let body_current_value = self.make_expression(
                span_ref,
                HirExpressionKind::Copy(HirPlace::Local(locals.current)),
                types.binding,
                ValueKind::RValue,
                body_region_id,
            );
            let binding = self.register_loop_binding_local(
                item_binding,
                types.binding,
                body_current_value,
                &visible_bindings,
                span_ref,
            )?;
            visible_bindings.push(binding);
        }

        if let Some(index_binding) = &bindings.index {
            let index_value = self.make_expression(
                span_ref,
                HirExpressionKind::Copy(HirPlace::Local(locals.iteration_index)),
                types.int_type,
                ValueKind::RValue,
                body_region_id,
            );
            let binding = self.register_loop_binding_local(
                index_binding,
                types.int_type,
                index_value,
                &visible_bindings,
                span_ref,
            )?;
            visible_bindings.push(binding);
        }

        self.push_loop_targets(blocks.exit, blocks.step);
        let body_result =
            self.with_temporary_local_bindings(visible_bindings, |builder| emit_body(builder));
        self.pop_loop_targets();
        body_result?;

        let body_tail_block = self.current_block_id_or_error(span_ref)?;
        if !self.block_has_explicit_terminator(body_tail_block, span_ref)? {
            self.emit_jump_to(body_tail_block, blocks.step, span_ref, "for.body.step")?;
        }

        Ok(!self.discard_unreachable_empty_block(blocks.step, span_ref)?)
    }

    fn emit_range_loop_step(
        &mut self,
        runtime: RangeLoopRuntime,
        end_kind: RangeEndKind,
        span_ref: &Option<SourceSpan>,
    ) -> Result<(), CompilerError> {
        let RangeLoopRuntime {
            blocks,
            locals,
            types,
        } = runtime;

        if uses_float_range_candidate(types.domain) {
            self.emit_float_range_candidate_update(runtime, end_kind, span_ref)?;
            self.discard_float_range_precheck_blocks(blocks, end_kind, span_ref)?;
        } else {
            self.emit_range_loop_distance_candidate_precheck(runtime, end_kind, span_ref)?;
        }
        self.set_current_block(blocks.step_stall_check, span_ref)?;
        let stall_region = self.current_region_or_error(span_ref)?;
        let current = self.range_loop_local(locals.current, types.binding, span_ref, stall_region);
        let candidate =
            self.range_loop_local(locals.next_value, types.binding, span_ref, stall_region);
        let stalled = self.range_loop_comparison(
            candidate,
            HirBinOp::Eq,
            current,
            types,
            span_ref,
            stall_region,
        );
        self.emit_range_loop_branch(
            stalled,
            blocks.step_stall_failure,
            blocks.step_commit,
            span_ref,
            "for.step.stalled",
            "for.step.advances",
        )?;

        self.set_current_block(blocks.step_stall_failure, span_ref)?;
        self.emit_terminator(
            blocks.step_stall_failure,
            HirTerminator::RuntimeFailure {
                message: "Floating-point range step made no progress".to_owned(),
            },
            span_ref,
        )?;

        self.set_current_block(blocks.step_commit, span_ref)?;
        let commit_region = self.current_region_or_error(span_ref)?;
        let candidate = self.make_expression(
            span_ref,
            HirExpressionKind::Copy(HirPlace::Local(locals.next_value)),
            types.binding,
            ValueKind::RValue,
            commit_region,
        );
        self.emit_statement_kind(
            HirStatementKind::Assign {
                target: HirPlace::Local(locals.current),
                value: candidate,
            },
            span_ref,
        )?;

        let index_current = self.range_loop_local(
            locals.iteration_index,
            types.int_type,
            span_ref,
            commit_region,
        );
        let index_delta = self.make_expression(
            span_ref,
            HirExpressionKind::Int(1),
            types.int_type,
            ValueKind::Const,
            commit_region,
        );
        self.emit_checked_numeric_assignment(
            locals.iteration_index,
            HirNumericOp {
                operator: NumericOperator::Add,
                domain: NumericScalar::Int,
            },
            index_current,
            index_delta,
            span_ref,
        )?;
        self.emit_jump_from_current_block(blocks.header_selector, span_ref, "for.backedge")
    }

    fn emit_range_loop_distance_candidate_precheck(
        &mut self,
        runtime: RangeLoopRuntime,
        end_kind: RangeEndKind,
        span_ref: &Option<SourceSpan>,
    ) -> Result<(), CompilerError> {
        let RangeLoopRuntime {
            blocks,
            locals,
            types,
        } = runtime;

        let distance_local = locals.distance.ok_or_else(|| {
            CompilerError::compiler_error(
                "Range-loop distance lowering is missing its distance local",
            )
        })?;
        self.set_current_block(blocks.step, span_ref)?;
        let step_region = self.current_region_or_error(span_ref)?;
        let ascending =
            self.range_loop_local(locals.ascending, types.bool_type, span_ref, step_region);
        self.emit_range_loop_branch(
            ascending,
            blocks.step_ascending_check,
            blocks.step_descending_check,
            span_ref,
            "for.step.asc",
            "for.step.desc",
        )?;

        self.emit_range_loop_step_direction_check(runtime, true, span_ref)?;
        self.emit_range_loop_step_direction_check(runtime, false, span_ref)?;
        self.emit_range_loop_distance_update(runtime, true, span_ref)?;
        self.emit_range_loop_distance_update(runtime, false, span_ref)?;

        self.set_current_block(blocks.step_distance_check, span_ref)?;
        let distance_region = self.current_region_or_error(span_ref)?;
        let step_value =
            self.range_loop_local(locals.step, types.binding, span_ref, distance_region);
        let distance_value =
            self.range_loop_local(distance_local, types.binding, span_ref, distance_region);
        let within_distance_op = match end_kind {
            RangeEndKind::Exclusive => HirBinOp::Lt,
            RangeEndKind::Inclusive => HirBinOp::Le,
        };
        let within_distance = self.range_loop_comparison(
            step_value,
            within_distance_op,
            distance_value,
            types,
            span_ref,
            distance_region,
        );
        self.emit_range_loop_branch(
            within_distance,
            blocks.step_candidate_direction,
            blocks.exit,
            span_ref,
            "for.step.has-next",
            "for.step.done",
        )?;

        self.set_current_block(blocks.step_candidate_direction, span_ref)?;
        let candidate_direction_region = self.current_region_or_error(span_ref)?;
        let ascending = self.range_loop_local(
            locals.ascending,
            types.bool_type,
            span_ref,
            candidate_direction_region,
        );
        self.emit_range_loop_branch(
            ascending,
            blocks.step_candidate_ascending,
            blocks.step_candidate_descending,
            span_ref,
            "for.step.candidate-asc",
            "for.step.candidate-desc",
        )?;

        self.emit_range_loop_candidate_update(runtime, true, span_ref)?;
        self.emit_range_loop_candidate_update(runtime, false, span_ref)?;

        self.set_current_block(blocks.step_candidate_check, span_ref)?;
        let candidate_check_region = self.current_region_or_error(span_ref)?;
        let ascending = self.range_loop_local(
            locals.ascending,
            types.bool_type,
            span_ref,
            candidate_check_region,
        );
        self.emit_range_loop_branch(
            ascending,
            blocks.step_candidate_ascending_check,
            blocks.step_candidate_descending_check,
            span_ref,
            "for.step.check-asc",
            "for.step.check-desc",
        )?;

        self.emit_range_loop_candidate_bound_check(runtime, end_kind, true, span_ref)?;
        self.emit_range_loop_candidate_bound_check(runtime, end_kind, false, span_ref)?;

        Ok(())
    }

    fn emit_float_range_candidate_update(
        &mut self,
        runtime: RangeLoopRuntime,
        end_kind: RangeEndKind,
        span_ref: &Option<SourceSpan>,
    ) -> Result<(), CompilerError> {
        let RangeLoopRuntime {
            blocks,
            locals,
            types,
        } = runtime;
        self.set_current_block(blocks.step, span_ref)?;
        if matches!(end_kind, RangeEndKind::Inclusive) {
            let step_region = self.current_region_or_error(span_ref)?;
            let current =
                self.range_loop_local(locals.current, types.binding, span_ref, step_region);
            let end = self.range_loop_local(locals.end, types.binding, span_ref, step_region);
            let at_endpoint = self.range_loop_comparison(
                current,
                HirBinOp::Eq,
                end,
                types,
                span_ref,
                step_region,
            );
            self.emit_range_loop_branch(
                at_endpoint,
                blocks.exit,
                blocks.step_candidate_check,
                span_ref,
                "for.step.inclusive-end",
                "for.step.before-inclusive-end",
            )?;
            self.set_current_block(blocks.step_candidate_check, span_ref)?;
        }

        let candidate_region = self.current_region_or_error(span_ref)?;
        let current =
            self.range_loop_local(locals.current, types.binding, span_ref, candidate_region);
        let step = self.range_loop_local(locals.step, types.binding, span_ref, candidate_region);
        let end = self.range_loop_local(locals.end, types.binding, span_ref, candidate_region);
        let ascending = self.range_loop_local(
            locals.ascending,
            types.bool_type,
            span_ref,
            candidate_region,
        );
        let in_range_result = locals.candidate_in_range.ok_or_else(|| {
            CompilerError::compiler_error(
                "Float range candidate lowering is missing its Bool result local",
            )
        })?;

        self.emit_statement_kind(
            HirStatementKind::FloatRangeCandidate {
                current,
                step,
                end,
                ascending,
                inclusive: matches!(end_kind, RangeEndKind::Inclusive),
                domain: types.domain,
                candidate_result: locals.next_value,
                in_range_result,
            },
            span_ref,
        )?;

        let in_range =
            self.range_loop_local(in_range_result, types.bool_type, span_ref, candidate_region);
        self.emit_range_loop_branch(
            in_range,
            blocks.step_stall_check,
            blocks.exit,
            span_ref,
            "for.step.candidate-in-range",
            "for.step.candidate-out",
        )
    }

    fn discard_float_range_precheck_blocks(
        &mut self,
        blocks: RangeLoopBlocks,
        end_kind: RangeEndKind,
        span_ref: &Option<SourceSpan>,
    ) -> Result<(), CompilerError> {
        for block in [
            blocks.step_ascending_check,
            blocks.step_descending_check,
            blocks.step_ascending_distance,
            blocks.step_descending_distance,
            blocks.step_distance_check,
            blocks.step_candidate_direction,
            blocks.step_candidate_ascending,
            blocks.step_candidate_descending,
            blocks.step_candidate_ascending_check,
            blocks.step_candidate_descending_check,
        ] {
            if !self.discard_unreachable_empty_block(block, span_ref)? {
                return_hir_transformation_error!(
                    "Unused float range-loop distance block was not empty and unowned",
                    self.hir_error_location(span_ref)
                );
            }
        }

        if matches!(end_kind, RangeEndKind::Exclusive)
            && !self.discard_unreachable_empty_block(blocks.step_candidate_check, span_ref)?
        {
            return_hir_transformation_error!(
                "Unused float range-loop distance block was not empty and unowned",
                self.hir_error_location(span_ref)
            );
        }

        Ok(())
    }

    fn emit_range_loop_step_direction_check(
        &mut self,
        runtime: RangeLoopRuntime,
        ascending: bool,
        span_ref: &Option<SourceSpan>,
    ) -> Result<(), CompilerError> {
        let RangeLoopRuntime {
            blocks,
            locals,
            types,
        } = runtime;
        let (block, distance_block) = if ascending {
            (blocks.step_ascending_check, blocks.step_ascending_distance)
        } else {
            (
                blocks.step_descending_check,
                blocks.step_descending_distance,
            )
        };

        self.set_current_block(block, span_ref)?;
        let region = self.current_region_or_error(span_ref)?;
        let crosses_zero = if matches!(
            types.domain,
            NumericScalar::Fixed(scalar)
                if scalar.class() == FixedScalarClass::UnsignedInteger
        ) {
            self.make_expression(
                span_ref,
                HirExpressionKind::Bool(false),
                types.bool_type,
                ValueKind::Const,
                region,
            )
        } else {
            let current = self.range_loop_local(locals.current, types.binding, span_ref, region);
            let end = self.range_loop_local(locals.end, types.binding, span_ref, region);
            let zero_for_current = self.range_loop_zero_literal(types, span_ref, region);
            let current_sign = self.range_loop_comparison(
                current,
                if ascending {
                    HirBinOp::Lt
                } else {
                    HirBinOp::Ge
                },
                zero_for_current,
                types,
                span_ref,
                region,
            );
            let zero_for_end = self.range_loop_zero_literal(types, span_ref, region);
            let end_sign = self.range_loop_comparison(
                zero_for_end,
                if ascending {
                    HirBinOp::Le
                } else {
                    HirBinOp::Ge
                },
                end,
                types,
                span_ref,
                region,
            );
            self.make_expression(
                span_ref,
                HirExpressionKind::BinOp {
                    left: Box::new(current_sign),
                    op: HirBinOp::And,
                    right: Box::new(end_sign),
                },
                types.bool_type,
                ValueKind::RValue,
                region,
            )
        };
        self.emit_range_loop_branch(
            crosses_zero,
            blocks.step_candidate_direction,
            distance_block,
            span_ref,
            if ascending {
                "for.step.asc.cross-zero"
            } else {
                "for.step.desc.cross-zero"
            },
            if ascending {
                "for.step.asc.distance"
            } else {
                "for.step.desc.distance"
            },
        )
    }

    fn emit_range_loop_distance_update(
        &mut self,
        runtime: RangeLoopRuntime,
        ascending: bool,
        span_ref: &Option<SourceSpan>,
    ) -> Result<(), CompilerError> {
        let RangeLoopRuntime {
            blocks,
            locals,
            types,
        } = runtime;
        let distance_local = locals.distance.ok_or_else(|| {
            CompilerError::compiler_error(
                "Range-loop distance lowering is missing its distance local",
            )
        })?;
        let block = if ascending {
            blocks.step_ascending_distance
        } else {
            blocks.step_descending_distance
        };
        self.set_current_block(block, span_ref)?;
        let region = self.current_region_or_error(span_ref)?;
        let current = self.range_loop_local(locals.current, types.binding, span_ref, region);
        let end = self.range_loop_local(locals.end, types.binding, span_ref, region);
        let (left, right) = if ascending {
            (end, current)
        } else {
            (current, end)
        };
        let subtract = self.numeric_op_for_binding(types.binding, NumericOperator::Subtract);
        self.emit_checked_numeric_assignment(distance_local, subtract, left, right, span_ref)?;
        self.emit_jump_from_current_block(blocks.step_distance_check, span_ref, "for.step.distance")
    }

    fn emit_range_loop_candidate_update(
        &mut self,
        runtime: RangeLoopRuntime,
        ascending: bool,
        span_ref: &Option<SourceSpan>,
    ) -> Result<(), CompilerError> {
        let RangeLoopRuntime {
            blocks,
            locals,
            types,
        } = runtime;
        let block = if ascending {
            blocks.step_candidate_ascending
        } else {
            blocks.step_candidate_descending
        };
        self.set_current_block(block, span_ref)?;
        let region = self.current_region_or_error(span_ref)?;
        let current = self.range_loop_local(locals.current, types.binding, span_ref, region);
        let step = self.range_loop_local(locals.step, types.binding, span_ref, region);
        let operator = if ascending {
            NumericOperator::Add
        } else {
            NumericOperator::Subtract
        };
        let operation = self.numeric_op_for_binding(types.binding, operator);
        self.emit_checked_numeric_assignment(
            locals.next_value,
            operation,
            current,
            step,
            span_ref,
        )?;
        self.emit_jump_from_current_block(
            blocks.step_candidate_check,
            span_ref,
            "for.step.candidate",
        )
    }

    fn emit_range_loop_candidate_bound_check(
        &mut self,
        runtime: RangeLoopRuntime,
        end_kind: RangeEndKind,
        ascending: bool,
        span_ref: &Option<SourceSpan>,
    ) -> Result<(), CompilerError> {
        let RangeLoopRuntime {
            blocks,
            locals,
            types,
        } = runtime;
        let block = if ascending {
            blocks.step_candidate_ascending_check
        } else {
            blocks.step_candidate_descending_check
        };
        self.set_current_block(block, span_ref)?;
        let region = self.current_region_or_error(span_ref)?;
        let candidate = self.range_loop_local(locals.next_value, types.binding, span_ref, region);
        let end = self.range_loop_local(locals.end, types.binding, span_ref, region);
        let within_bound_op = match (ascending, end_kind) {
            (true, RangeEndKind::Exclusive) => HirBinOp::Lt,
            (true, RangeEndKind::Inclusive) => HirBinOp::Le,
            (false, RangeEndKind::Exclusive) => HirBinOp::Gt,
            (false, RangeEndKind::Inclusive) => HirBinOp::Ge,
        };
        let within_bound =
            self.range_loop_comparison(candidate, within_bound_op, end, types, span_ref, region);
        self.emit_range_loop_branch(
            within_bound,
            blocks.step_stall_check,
            blocks.exit,
            span_ref,
            "for.step.candidate-in-range",
            "for.step.candidate-out",
        )
    }

    fn range_loop_local(
        &mut self,
        local: LocalId,
        ty: TypeId,
        span_ref: &Option<SourceSpan>,
        region: RegionId,
    ) -> HirExpression {
        self.make_expression(
            span_ref,
            HirExpressionKind::Load(HirPlace::Local(local)),
            ty,
            ValueKind::Place,
            region,
        )
    }

    fn range_loop_comparison(
        &mut self,
        left: HirExpression,
        operator: HirBinOp,
        right: HirExpression,
        types: RangeLoopTypes,
        span_ref: &Option<SourceSpan>,
        region: RegionId,
    ) -> HirExpression {
        self.make_expression(
            span_ref,
            HirExpressionKind::BinOp {
                left: Box::new(left),
                op: operator,
                right: Box::new(right),
            },
            types.bool_type,
            ValueKind::RValue,
            region,
        )
    }

    fn emit_range_loop_branch(
        &mut self,
        condition: HirExpression,
        then_block: BlockId,
        else_block: BlockId,
        span_ref: &Option<SourceSpan>,
        then_label: &str,
        else_label: &str,
    ) -> Result<(), CompilerError> {
        let source_block = self.current_block_id_or_error(span_ref)?;
        self.emit_terminator(
            source_block,
            HirTerminator::If {
                condition,
                then_block,
                else_block,
            },
            span_ref,
        )?;
        self.log_control_flow_edge(source_block, then_block, then_label);
        self.log_control_flow_edge(source_block, else_block, else_label);
        Ok(())
    }

    fn range_loop_zero_literal(
        &mut self,
        types: RangeLoopTypes,
        span_ref: &Option<SourceSpan>,
        region: RegionId,
    ) -> HirExpression {
        let kind = match types.domain {
            NumericScalar::Int => HirExpressionKind::Int(0),
            NumericScalar::Float => HirExpressionKind::Float(0.0),
            NumericScalar::Fixed(scalar) => {
                let zero = match scalar.class() {
                    FixedScalarClass::SignedInteger => FixedScalarValue::signed(scalar, 0),
                    FixedScalarClass::UnsignedInteger => FixedScalarValue::unsigned(scalar, 0),
                    FixedScalarClass::BinaryFloat => FixedScalarValue::binary_float(scalar, 0.0),
                    FixedScalarClass::Octet => None,
                }
                .expect("range-loop numeric domain has a representable zero");
                HirExpressionKind::FixedScalar(zero)
            }
        };
        self.make_expression(span_ref, kind, types.binding, ValueKind::Const, region)
    }

    fn range_loop_unit_literal(
        &mut self,
        types: RangeLoopTypes,
        span_ref: &Option<SourceSpan>,
        region: RegionId,
    ) -> HirExpression {
        let kind = match types.domain {
            NumericScalar::Int => HirExpressionKind::Int(1),
            NumericScalar::Float => HirExpressionKind::Float(1.0),
            NumericScalar::Fixed(scalar) => {
                let one = match scalar.class() {
                    FixedScalarClass::SignedInteger => FixedScalarValue::signed(scalar, 1),
                    FixedScalarClass::UnsignedInteger => FixedScalarValue::unsigned(scalar, 1),
                    FixedScalarClass::BinaryFloat => FixedScalarValue::binary_float(scalar, 1.0),
                    FixedScalarClass::Octet => None,
                }
                .expect("range-loop numeric domain has a representable unit step");
                HirExpressionKind::FixedScalar(one)
            }
        };
        self.make_expression(span_ref, kind, types.binding, ValueKind::Const, region)
    }

    pub(super) fn lower_collection_loop_statement_impl(
        &mut self,
        bindings: &LoopBindings,
        iterable: &Expression,
        body: &[AstNode],
        span_ref: &Option<SourceSpan>,
        span: Option<SourceSpan>,
    ) -> Result<(), CompilerError> {
        self.lower_collection_loop_with_body_emitter_and_span(
            bindings,
            iterable,
            span_ref,
            span,
            |builder| builder.lower_statement_sequence(body),
        )
    }

    pub(crate) fn lower_collection_loop_with_body_emitter(
        &mut self,
        bindings: &LoopBindings,
        iterable: &Expression,
        span_ref: &Option<SourceSpan>,
        mut emit_body: impl FnMut(&mut HirBuilder<'_>) -> Result<(), CompilerError>,
    ) -> Result<(), CompilerError> {
        // Template loops are generated scaffolding: headers stay spanless. Statement
        // loops use the span-aware entry below with the authored `for` span.
        self.lower_collection_loop_with_body_emitter_and_span(
            bindings,
            iterable,
            span_ref,
            None,
            &mut emit_body,
        )
    }

    fn lower_collection_loop_with_body_emitter_and_span(
        &mut self,
        bindings: &LoopBindings,
        iterable: &Expression,
        span_ref: &Option<SourceSpan>,
        span: Option<SourceSpan>,
        mut emit_body: impl FnMut(&mut HirBuilder<'_>) -> Result<(), CompilerError>,
    ) -> Result<(), CompilerError> {
        // Generated collection spills: the iterable/length/index temps, the length
        // call, and step/jump edges are compiler scaffolding and stay spanless.
        // Only the header bounds check below carries the authored `for` span.
        let parent_region = self.current_region_or_error(span_ref)?;

        let header_block = self.create_block(parent_region, span_ref, "loop-collection-header")?;
        let body_region = self.create_child_region(parent_region);
        let body_block = self.create_block(body_region, span_ref, "loop-collection-body")?;
        let step_block = self.create_block(parent_region, span_ref, "loop-collection-step")?;
        let exit_block = self.create_block(parent_region, span_ref, "loop-collection-exit")?;

        let (iterable_type, element_type) = self.collection_iteration_types(iterable, span_ref)?;

        let bool_ty = builtin_type_ids::BOOL;
        let int_ty: TypeId = builtin_type_ids::INT;
        let iterable_local = self.allocate_temp_local(iterable_type, None)?;
        let length_local = self.allocate_temp_local(int_ty, None)?;
        let iteration_index_local = self.allocate_temp_local(int_ty, None)?;

        let lowered_iterable = self.lower_expression_value_to_current_block(iterable)?;
        self.emit_statement_kind(
            HirStatementKind::Assign {
                target: HirPlace::Local(iterable_local),
                value: lowered_iterable,
            },
            span_ref,
        )?;

        let pre_header_region = self.current_region_or_error(span_ref)?;
        let zero_index = self.make_expression(
            span_ref,
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
            span_ref,
        )?;

        let iterable_for_length = self.make_expression(
            span_ref,
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
            span_ref,
        )?;

        self.emit_jump_from_current_block(header_block, span_ref, "loop.collection.enter")?;

        self.set_current_block(header_block, span_ref)?;
        let header_region = self.current_region_or_error(span_ref)?;
        let current_index = self.make_expression(
            span_ref,
            HirExpressionKind::Load(HirPlace::Local(iteration_index_local)),
            int_ty,
            ValueKind::Place,
            header_region,
        );
        let collection_length = self.make_expression(
            span_ref,
            HirExpressionKind::Load(HirPlace::Local(length_local)),
            int_ty,
            ValueKind::Place,
            header_region,
        );
        let continue_condition = self.make_expression(
            span_ref,
            HirExpressionKind::BinOp {
                left: Box::new(current_index),
                op: HirBinOp::Lt,
                right: Box::new(collection_length),
            },
            bool_ty,
            ValueKind::RValue,
            header_region,
        );
        // Authored loop header: the bounds check carries the `for` statement span,
        // falling back to the iterable span when identity-free. Init spills, the
        // length call, and jumps above are generated scaffolding and stay spanless.
        self.emit_terminator_with_span(
            header_block,
            HirTerminator::If {
                condition: continue_condition,
                then_block: body_block,
                else_block: exit_block,
            },
            span_ref,
            span.or(iterable.span),
        )?;
        self.log_control_flow_edge(header_block, body_block, "loop.collection.true");
        self.log_control_flow_edge(header_block, exit_block, "loop.collection.false");

        self.set_current_block(body_block, span_ref)?;
        let body_region_id = self.current_region_or_error(span_ref)?;
        let mut visible_bindings = Vec::new();

        if let Some(item_binding) = &bindings.item {
            let item_index = self.make_expression(
                span_ref,
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
                span_ref,
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
                span_ref,
            )?;
            visible_bindings.push(binding);
        }

        if let Some(index_binding) = &bindings.index {
            let user_index_value = self.make_expression(
                span_ref,
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
                span_ref,
            )?;
            visible_bindings.push(binding);
        }

        self.push_loop_targets(exit_block, step_block);
        let body_result =
            self.with_temporary_local_bindings(visible_bindings, |builder| emit_body(builder));
        self.pop_loop_targets();
        body_result?;

        let body_tail_block = self.current_block_id_or_error(span_ref)?;
        if !self.block_has_explicit_terminator(body_tail_block, span_ref)? {
            self.emit_jump_to(
                body_tail_block,
                step_block,
                span_ref,
                "loop.collection.body.step",
            )?;
        }

        if self.discard_unreachable_empty_block(step_block, span_ref)? {
            return self.set_current_block(exit_block, span_ref);
        }

        self.set_current_block(step_block, span_ref)?;
        let step_region = self.current_region_or_error(span_ref)?;
        let step_current = self.make_expression(
            span_ref,
            HirExpressionKind::Load(HirPlace::Local(iteration_index_local)),
            int_ty,
            ValueKind::Place,
            step_region,
        );
        let step_delta = self.make_expression(
            span_ref,
            HirExpressionKind::Int(1),
            int_ty,
            ValueKind::Const,
            step_region,
        );
        self.emit_checked_numeric_assignment(
            iteration_index_local,
            HirNumericOp {
                operator: NumericOperator::Add,
                domain: NumericScalar::Int,
            },
            step_current,
            step_delta,
            span_ref,
        )?;
        self.emit_jump_from_current_block(header_block, span_ref, "loop.collection.backedge")?;

        self.set_current_block(exit_block, span_ref)
    }

    fn register_loop_binding_local(
        &mut self,
        binding: &crate::compiler_frontend::ast::ast_nodes::Declaration,
        ty: TypeId,
        value: HirExpression,
        visible_bindings: &[(PathId, LocalId)],
        span_ref: &Option<SourceSpan>,
    ) -> Result<(PathId, LocalId), CompilerError> {
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
                self.hir_error_location(span_ref)
            );
        }

        let region = self.current_region_or_error(span_ref)?;
        let block_id = self.current_block_id_or_error(span_ref)?;
        let local_id = self.allocate_local_id();
        // Authored loop binding uses the explicit `Declaration.binding_span`
        // (the binding-name anchor), never an initializer span.
        let local = HirLocal {
            id: local_id,
            ty,
            mutable: false,
            region,
            span: binding.binding_span,
        };

        self.side_table.map_local_source(&local);
        self.register_local_in_block(block_id, local, span_ref)?;
        self.side_table.bind_local_name(local_id, binding.id);
        self.side_table
            .bind_local_origin(local_id, HirLocalOriginKind::User, None, None);
        self.side_table
            .map_ast_to_hir(*span_ref, HirLocation::Local(local_id));

        // Authored binding materialization carries the explicit binding span.
        // Jumps and numeric step updates elsewhere stay spanless as generated
        // scaffolding.
        self.emit_statement_kind_with_span(
            HirStatementKind::Assign {
                target: HirPlace::Local(local_id),
                value,
            },
            span_ref,
            binding.binding_span,
        )?;

        Ok((binding.id, local_id))
    }

    fn range_iteration_type(
        &mut self,
        bindings: &LoopBindings,
        range: &RangeLoopSpec,
        span_ref: &Option<SourceSpan>,
    ) -> Result<TypeId, CompilerError> {
        if let Some(item_binding) = &bindings.item {
            return self.lower_type_id(item_binding.value.type_id, span_ref);
        }

        let start_ty = self.lower_type_id(range.start.type_id, span_ref)?;
        let end_ty = self.lower_type_id(range.end.type_id, span_ref)?;
        let step_ty = range
            .step
            .as_ref()
            .map(|step| self.lower_type_id(step.type_id, span_ref))
            .transpose()?;

        let Some(start_domain) = NumericScalar::from_type_id(start_ty, &self.type_environment)
        else {
            return_hir_transformation_error!(
                "Range loop start did not lower to a numeric scalar type",
                self.hir_error_location(span_ref)
            );
        };
        let Some(end_domain) = NumericScalar::from_type_id(end_ty, &self.type_environment) else {
            return_hir_transformation_error!(
                "Range loop end did not lower to a numeric scalar type",
                self.hir_error_location(span_ref)
            );
        };
        let Some(mut domain) =
            binary_operation_domain(NumericOperator::Add, start_domain, end_domain)
        else {
            return_hir_transformation_error!(
                "Range loop bounds have no common numeric promotion domain",
                self.hir_error_location(span_ref)
            );
        };

        if let Some(step_ty) = step_ty {
            let Some(step_domain) = NumericScalar::from_type_id(step_ty, &self.type_environment)
            else {
                return_hir_transformation_error!(
                    "Range loop step did not lower to a numeric scalar type",
                    self.hir_error_location(span_ref)
                );
            };
            let Some(promoted) = binary_operation_domain(NumericOperator::Add, domain, step_domain)
            else {
                return_hir_transformation_error!(
                    "Range loop step has no common numeric promotion domain",
                    self.hir_error_location(span_ref)
                );
            };
            domain = promoted;
        }

        Ok(domain.type_id(&self.type_environment))
    }

    fn collection_iteration_types(
        &mut self,
        iterable: &Expression,
        span_ref: &Option<SourceSpan>,
    ) -> Result<(TypeId, TypeId), CompilerError> {
        // Frontend type resolution canonicalizes reference wrappers, so collection loops
        // accept either direct collections or shared references to collections uniformly.
        let iterable_type = self.lower_type_id(iterable.type_id, span_ref)?;
        let element = match self.type_environment.collection_element_type(iterable_type) {
            Some(element) => element,
            None => {
                return_hir_transformation_error!(
                    "Collection loop iterable did not lower to a collection HIR type",
                    self.hir_error_location(span_ref)
                );
            }
        };

        Ok((iterable_type, element))
    }
}
