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
use crate::compiler_frontend::ast::expressions::expression_store::{
    ExprId, ExpressionStore, ExpressionView,
};
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::datatypes::numeric_operators::NumericOperator;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::external_packages::{CallTarget, ExternalFunctionId};
use crate::compiler_frontend::hir::blocks::HirLocal;
use crate::compiler_frontend::hir::expression_store::HirConstructionFailure;
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::hir_builder::{HirBuilder, LoopExitPolicy};
use crate::compiler_frontend::hir::hir_side_table::{HirLocalOriginKind, HirLocation};
use crate::compiler_frontend::hir::ids::{BlockId, HirValueId, LocalId, RegionId};
use crate::compiler_frontend::hir::numeric::{
    HirNumericOp, HirNumericOperands, LoopStepFailureCause, NumericFailureMode,
};
use crate::compiler_frontend::hir::operators::HirBinOp;
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::statements::{
    HirLocalDestination, HirStatementKind, HirWriteTarget,
};
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathId;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarClass, FixedScalarValue};

/// Jump targets of the range-loop CFG pipeline. These are destinations only. The module header
/// explains why the source of each edge is read live instead of stored here.
#[derive(Clone, Copy)]
struct RangeLoopBlocks {
    step_zero_check: BlockId,
    step_zero_failure: BlockId,
    /// Receives the non-zero step, records `ascending = step > 0` and selects the header edge.
    header_selector: BlockId,
    header_ascending: BlockId,
    header_descending: BlockId,
    body: BlockId,
    /// Inclusive-endpoint check and the candidate producer.
    step: BlockId,
    step_candidate: BlockId,
    /// Binary-float counters alone need the rounded-result no-progress guard.
    step_stall: Option<RangeLoopStallBlocks>,
    step_commit: BlockId,
    exit: BlockId,
}

#[derive(Clone, Copy)]
struct RangeLoopStallBlocks {
    check: BlockId,
    failure: BlockId,
}

#[derive(Clone, Copy)]
struct RangeLoopLocals {
    current: LocalId,
    end: LocalId,
    step: LoopStepOperand,
    ascending: LocalId,
    iteration_index: LocalId,
    next_value: LocalId,
    candidate_in_range: LocalId,
}

#[derive(Clone, Copy)]
struct RangeLoopTypes {
    binding: TypeId,
    domain: NumericScalar,
    bool_type: TypeId,
    int_type: TypeId,
}

/// The step's own completed type and scalar before its temporary is allocated.
///
/// WHY: the step type is resolved with the counter types but its local must still be allocated
///      in the established local order.
#[derive(Clone, Copy)]
struct LoopStepType {
    ty: TypeId,
    scalar: FixedScalar,
}

#[derive(Clone, Copy)]
struct RangeLoopRuntime {
    blocks: RangeLoopBlocks,
    locals: RangeLoopLocals,
    types: RangeLoopTypes,
}

/// One evaluated loop step and the fixed type it keeps across both loop kinds.
///
/// WHAT: pairs the step's temporary with its own completed type and numeric scalar.
/// WHY: range and collection loops guard the same zero step, test the same positive sign and
///      compare against the same zero literal without converting the step to a counter type.
#[derive(Clone, Copy)]
struct LoopStepOperand {
    local: LocalId,
    ty: TypeId,
    scalar: FixedScalar,
}

/// Jump targets shared by both collection-loop modes.
#[derive(Clone, Copy)]
struct CollectionLoopBlocks {
    body: BlockId,
    step: BlockId,
    exit: BlockId,
}

/// The collection loop's one execution mode.
///
/// The default `+1` loop keeps a lean repeated header comparing the index with the captured
/// length. An explicit step needs the stepped pipeline instead: guard the evaluated step, prove
/// the source nonempty, select the first and final index by sign, then advance with one
/// `RangeCandidate` per iteration. The variants are mutually exclusive by construction, so
/// lowering matches this field once per seam instead of re-checking independent option fields.
#[derive(Clone, Copy)]
enum CollectionLoopMode {
    Default {
        /// Repeated header. Stepped loops commit the candidate and re-enter the body without
        /// re-testing a header.
        header: BlockId,
    },
    Stepped {
        /// Authored step expression, evaluated exactly once after the source and length snapshot.
        step: ExprId,
        blocks: CollectionLoopStepBlocks,
        locals: CollectionLoopSteppedLocals,
    },
}

#[derive(Clone, Copy)]
struct CollectionLoopStepBlocks {
    zero_check: BlockId,
    zero_failure: BlockId,
    empty_check: BlockId,
    seed_selector: BlockId,
    seed_positive: BlockId,
    seed_negative: BlockId,
    step_candidate: BlockId,
    step_commit: BlockId,
}

#[derive(Clone, Copy)]
struct CollectionLoopLocals {
    /// Holds the source once. When the source is a place its definition is the single
    /// `Load`-of-place write that borrow checking follows to the source root.
    iterable: LocalId,
    length: LocalId,
    /// Actual source index, also the second loop binding when present.
    index: LocalId,
}

#[derive(Clone, Copy)]
struct CollectionLoopSteppedLocals {
    /// Inclusive final source index selected by the step sign.
    last: LocalId,
    step: LoopStepOperand,
    next_value: LocalId,
    candidate_in_range: LocalId,
}

#[derive(Clone, Copy)]
struct CollectionLoopTypes {
    iterable: TypeId,
    element: TypeId,
    /// Exact `I32` source-index and captured-length identity.
    index: TypeId,
    bool_type: TypeId,
}

#[derive(Clone, Copy)]
struct CollectionLoopRuntime {
    blocks: CollectionLoopBlocks,
    locals: CollectionLoopLocals,
    mode: CollectionLoopMode,
    types: CollectionLoopTypes,
}

impl<'a> HirBuilder<'a> {
    pub(super) fn lower_while_statement_impl(
        &mut self,
        expressions: &ExpressionStore,
        condition: ExpressionView<'_>,
        body: &[AstNode],
        span_ref: &Option<SourceSpan>,
        span: Option<SourceSpan>,
    ) -> Result<(), HirConstructionFailure> {
        self.lower_while_with_body_emitter(
            condition,
            LoopExitPolicy::Statement,
            span_ref,
            span,
            |builder| builder.lower_statement_sequence(expressions, body),
        )
    }

    /// Lowers a conditional loop into CFG while letting callers choose the body emitter.
    ///
    /// Runtime template loops need the same while-header/body/backedge shape as
    /// statement loops, but their body appends render units instead of lowering
    /// statement nodes. Keeping the CFG owner here avoids a second conditional
    /// loop lowering path in the template HIR code.
    pub(crate) fn lower_while_with_body_emitter(
        &mut self,
        condition: ExpressionView<'_>,
        exit_policy: LoopExitPolicy,
        span_ref: &Option<SourceSpan>,
        span: Option<SourceSpan>,
        emit_body: impl FnOnce(&mut HirBuilder<'_>) -> Result<(), HirConstructionFailure>,
    ) -> Result<(), HirConstructionFailure> {
        let parent_region = self.current_region_or_error()?;

        let header_block = self.create_block(parent_region, span_ref, "while-header");
        let body_region = self.create_child_region(parent_region);
        let body_block = self.create_block(body_region, span_ref, "while-body");
        let exit_block = self.create_block(parent_region, span_ref, "while-exit");

        self.emit_jump_from_current_block(header_block, span_ref, "while.enter")?;

        self.set_current_block(header_block)?;

        // Read the same AST constant-store fact the exit analysis used, before lowering the
        // condition. Recognising only HIR's lowered literal would disagree with a named or
        // body-local folded Bool and leave an accepted non-exiting body unterminated.
        let condition_is_folded_true = exit_policy == LoopExitPolicy::Statement
            && self.expression_constant_bool(condition) == Some(true);

        let condition_value = self.lower_expression_value_to_current_block(condition)?;
        let condition_block = self.current_block_id_or_error()?;

        if condition_is_folded_true {
            self.emit_jump_from_current_block(body_block, span_ref, "while.true.unconditional")?;
        } else {
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
        }

        self.set_current_block(body_block)?;
        self.push_loop_targets(exit_block, header_block);
        let body_result = emit_body(self);
        let body_breaks = self.current_loop_saw_break();
        self.pop_loop_targets();
        body_result?;

        let body_tail_block = self.current_block_id_or_error()?;
        if !self.block_has_explicit_terminator(body_tail_block)? {
            let backedge_block = self.create_block(parent_region, span_ref, "while-backedge");
            self.emit_jump_to(
                body_tail_block,
                backedge_block,
                span_ref,
                "while.body.backedge",
            )?;

            self.set_current_block(backedge_block)?;
            self.emit_jump_to(backedge_block, header_block, span_ref, "while.backedge")?;
        }

        if condition_is_folded_true && !body_breaks {
            // Nothing can resume after this loop. Drop the preallocated exit and leave the
            // cursor on the loop header, whose explicit terminator gives the same
            // "current path terminated" state as a return block. Later statements are then
            // not lowered and the function-end check sees an explicit terminator.
            let removed = self.discard_unreachable_empty_block(exit_block)?;
            if !removed {
                return Err(InfrastructureFailure::hir_transformation(
                    "A folded-true loop without a break left a reachable exit block.",
                )
                .into());
            }

            return Ok(self.set_current_block(header_block)?);
        }

        Ok(self.set_current_block(exit_block)?)
    }

    pub(super) fn lower_range_loop_statement_impl(
        &mut self,
        expressions: &ExpressionStore,
        bindings: &LoopBindings,
        range: &RangeLoopSpec,
        body: &[AstNode],
        span_ref: &Option<SourceSpan>,
        span: Option<SourceSpan>,
    ) -> Result<(), HirConstructionFailure> {
        self.lower_range_loop_with_body_emitter_and_span(
            expressions,
            bindings,
            range,
            span_ref,
            span,
            |builder| builder.lower_statement_sequence(expressions, body),
        )
    }

    pub(crate) fn lower_range_loop_with_body_emitter(
        &mut self,
        anchor: ExpressionView<'_>,
        bindings: &LoopBindings,
        range: &RangeLoopSpec,
        span_ref: &Option<SourceSpan>,
        mut emit_body: impl FnMut(&mut HirBuilder<'_>) -> Result<(), HirConstructionFailure>,
    ) -> Result<(), HirConstructionFailure> {
        let expressions = anchor.store();
        // Template loops are generated scaffolding: headers stay spanless. Statement
        // loops use the span-aware entry below with the authored `for` span.
        self.lower_range_loop_with_body_emitter_and_span(
            expressions,
            bindings,
            range,
            span_ref,
            None,
            &mut emit_body,
        )
    }

    fn lower_range_loop_with_body_emitter_and_span(
        &mut self,
        expressions: &ExpressionStore,
        bindings: &LoopBindings,
        range: &RangeLoopSpec,
        span_ref: &Option<SourceSpan>,
        span: Option<SourceSpan>,
        mut emit_body: impl FnMut(&mut HirBuilder<'_>) -> Result<(), HirConstructionFailure>,
    ) -> Result<(), HirConstructionFailure> {
        // Build a CFG pipeline so each update is produced only if a next item remains:
        // zero-step guard -> step-sign direction -> direction header -> candidate update.
        let parent_region = self.current_region_or_error()?;
        // Integer candidate addition is exact, so only a binary-float counter can round a
        // candidate back onto the current value and needs the no-progress guard.
        let uses_stall_guard = range.domain.is_binary_float();
        let blocks = self.create_range_loop_blocks(parent_region, span_ref, uses_stall_guard);
        let (types, step_type) = self.resolve_range_loop_types(expressions, range)?;
        let locals = self.allocate_range_loop_locals(types, step_type)?;
        let runtime = RangeLoopRuntime {
            blocks,
            locals,
            types,
        };

        self.initialize_range_loop_state(expressions, range, runtime, span_ref)?;

        // Dynamic `by` expressions still need a runtime zero check before any bounds check.
        self.emit_jump_from_current_block(runtime.blocks.step_zero_check, span_ref, "for.enter")?;

        let step_span = range
            .step
            .map(|id| expressions.view(id))
            .transpose()?
            .and_then(|step| step.span());
        self.emit_loop_zero_step_guard(
            runtime.locals.step,
            runtime.blocks.step_zero_check,
            runtime.blocks.step_zero_failure,
            runtime.blocks.header_selector,
            span_ref,
            &step_span,
            "for.step.zero",
            "for.step.non_zero",
        )?;
        self.emit_range_loop_direction(runtime, span_ref)?;
        self.emit_range_loop_header_checks(expressions, range, runtime, span_ref, span)?;
        let step_block_is_reachable =
            self.lower_range_loop_body_with_emitter(bindings, runtime, span_ref, &mut emit_body)?;
        if step_block_is_reachable {
            self.emit_range_loop_step(runtime, range.end_kind, span_ref, &step_span)?;
        } else {
            self.discard_unreachable_range_step_blocks(blocks)?;
        }

        Ok(self.set_current_block(runtime.blocks.exit)?)
    }

    fn create_range_loop_blocks(
        &mut self,
        parent_region: RegionId,
        span_ref: &Option<SourceSpan>,
        uses_stall_guard: bool,
    ) -> RangeLoopBlocks {
        let step_zero_check = self.create_block(parent_region, span_ref, "for-step-zero-check");
        let step_zero_failure = self.create_block(parent_region, span_ref, "for-step-zero-failure");
        let header_selector = self.create_block(parent_region, span_ref, "for-header-selector");
        let header_ascending = self.create_block(parent_region, span_ref, "for-header-ascending");
        let header_descending = self.create_block(parent_region, span_ref, "for-header-descending");
        let body_region = self.create_child_region(parent_region);
        let body = self.create_block(body_region, span_ref, "for-body");
        let step = self.create_block(parent_region, span_ref, "for-step");
        let step_candidate = self.create_block(parent_region, span_ref, "for-step-candidate");
        let step_stall = uses_stall_guard.then(|| RangeLoopStallBlocks {
            check: self.create_block(parent_region, span_ref, "for-step-stall-check"),
            failure: self.create_block(parent_region, span_ref, "for-step-stall-failure"),
        });
        let step_commit = self.create_block(parent_region, span_ref, "for-step-commit");
        let exit = self.create_block(parent_region, span_ref, "for-exit");

        RangeLoopBlocks {
            step_zero_check,
            step_zero_failure,
            header_selector,
            header_ascending,
            header_descending,
            body,
            step,
            step_candidate,
            step_stall,
            step_commit,
            exit,
        }
    }

    fn discard_unreachable_range_step_blocks(
        &mut self,
        blocks: RangeLoopBlocks,
    ) -> Result<(), HirConstructionFailure> {
        // Discard in CFG order: each removed step block drops the incoming edges that kept its
        // successor alive. The caller has already discarded `blocks.step` itself.
        let mut step_blocks = vec![blocks.step_candidate];
        if let Some(stall) = blocks.step_stall {
            step_blocks.push(stall.check);
            step_blocks.push(stall.failure);
        }
        step_blocks.push(blocks.step_commit);

        for block in step_blocks {
            if !self.discard_unreachable_empty_block(block)? {
                return Err(InfrastructureFailure::hir_transformation(
                    "Unreachable range-loop step block was not empty and unowned",
                )
                .into());
            }
        }

        Ok(())
    }

    /// Resolves the selected range-loop counter domain, the step's own type and derived types.
    ///
    /// WHAT: reads the bounds-selected counter domain and the completed type of the step
    ///       expression, then derives the counter type, step type, boolean flag type and exact
    ///       `I32` positional-index type.
    /// WHY: counter selection consumes the bounds' canonical types, which exist only at AST
    ///      construction. The step has its own completed type and is never re-derived or coerced
    ///      to the counter here: an integer counter accepts an independent signed integer step,
    ///      and a float counter already coerced its step to the counter type at AST construction.
    fn resolve_range_loop_types(
        &mut self,
        expressions: &ExpressionStore,
        range: &RangeLoopSpec,
    ) -> Result<(RangeLoopTypes, LoopStepType), HirConstructionFailure> {
        let domain = range.domain;
        if matches!(domain, NumericScalar::Number(_)) {
            crate::compiler_bug!(
                CompilerStage::HirConstruction,
                "Range-loop Dec item bindings are unsupported",
            );
        }

        let step_type = match range.step {
            Some(step_expression) => {
                let step_expression_type = expressions.view(step_expression)?.value_type_id()?;
                let step_type = self.lower_type_id(step_expression_type)?;
                let scalar = match NumericScalar::from_type_id(step_type, &self.type_environment) {
                    Some(NumericScalar::Fixed(scalar)) => scalar,
                    _ => {
                        return Err(InfrastructureFailure::hir_transformation(
                            "Range-loop step did not complete with a fixed numeric type",
                        )
                        .into());
                    }
                };
                LoopStepType {
                    ty: step_type,
                    scalar,
                }
            }
            // An omitted integer-range step is positive one of the independent default I32 type.
            None => LoopStepType {
                ty: builtin_type_ids::fixed_scalar(FixedScalar::I32),
                scalar: FixedScalar::I32,
            },
        };

        Ok((
            RangeLoopTypes {
                binding: domain.type_id(&self.type_environment),
                domain,
                bool_type: builtin_type_ids::BOOL,
                // The automatically supplied iteration index uses the language's exact I32
                // positional-index identity; it never follows the range counter's domain.
                int_type: builtin_type_ids::fixed_scalar(FixedScalar::I32),
            },
            step_type,
        ))
    }

    fn allocate_range_loop_locals(
        &mut self,
        types: RangeLoopTypes,
        step_type: LoopStepType,
    ) -> Result<RangeLoopLocals, HirConstructionFailure> {
        // Allocation order is observable in HIR snapshots and must remain stable.
        let current = self.allocate_temp_local(types.binding, None)?;
        let end = self.allocate_temp_local(types.binding, None)?;
        let step = LoopStepOperand {
            local: self.allocate_temp_local(step_type.ty, None)?,
            ty: step_type.ty,
            scalar: step_type.scalar,
        };
        let ascending = self.allocate_temp_local(types.bool_type, None)?;
        let iteration_index = self.allocate_temp_local(types.int_type, None)?;
        let next_value = self.allocate_temp_local(types.binding, None)?;
        let candidate_in_range = self.allocate_temp_local(types.bool_type, None)?;

        Ok(RangeLoopLocals {
            current,
            end,
            step,
            ascending,
            iteration_index,
            next_value,
            candidate_in_range,
        })
    }

    /// Snapshots source places so body mutations cannot change already evaluated loop state.
    ///
    /// Range bounds and explicit collection steps both need the value that the header evaluated
    /// once, not a place that a body mutation could change before a later use.
    fn snapshot_loop_operand(
        &mut self,
        value: HirValueId,
    ) -> Result<HirValueId, HirConstructionFailure> {
        let row = self.module.expressions.expression(value);
        let (HirExpressionKind::Load(place), ValueKind::Place) = (&row.kind, row.value_kind) else {
            return Ok(value);
        };
        let place = *place;
        let ty = row.ty;
        let region = row.region;
        let span = row.span;
        self.make_expression(
            &span,
            HirExpressionKind::Copy(place),
            ty,
            ValueKind::RValue,
            region,
        )
    }

    fn initialize_range_loop_state(
        &mut self,
        expressions: &ExpressionStore,
        range: &RangeLoopSpec,
        runtime: RangeLoopRuntime,
        span_ref: &Option<SourceSpan>,
    ) -> Result<(), HirConstructionFailure> {
        // Generated loop-state spills: `current`/`end`/`step`/`index` temps are compiler
        // scaffolding, so these definitions stay spanless even though the lowered bound values
        // themselves carry expression spans. Only the header bounds checks below carry the
        // authored `for` span.
        let RangeLoopRuntime { locals, types, .. } = runtime;

        let lowered_start =
            self.lower_expression_value_to_current_block(expressions.view(range.start)?)?;
        let lowered_start = self.snapshot_loop_operand(lowered_start)?;
        let lowered_start = self.convert_numeric_operand_to_domain(lowered_start, types.domain)?;
        self.emit_statement_kind(
            HirStatementKind::Write {
                target: HirWriteTarget::DefineLocal(locals.current),
                value: lowered_start,
            },
            span_ref,
        )?;

        let lowered_end =
            self.lower_expression_value_to_current_block(expressions.view(range.end)?)?;
        let lowered_end = self.snapshot_loop_operand(lowered_end)?;
        let lowered_end = self.convert_numeric_operand_to_domain(lowered_end, types.domain)?;
        self.emit_statement_kind(
            HirStatementKind::Write {
                target: HirWriteTarget::DefineLocal(locals.end),
                value: lowered_end,
            },
            span_ref,
        )?;

        // The step keeps its own completed type. Integer counters never convert it to the
        // counter domain, and a float counter's step was already coerced to the counter type at
        // AST construction.
        if let Some(step_expression) = &range.step {
            let lowered_step =
                self.lower_expression_value_to_current_block(expressions.view(*step_expression)?)?;
            let lowered_step = self.snapshot_loop_operand(lowered_step)?;

            self.emit_statement_kind(
                HirStatementKind::Write {
                    target: HirWriteTarget::DefineLocal(locals.step.local),
                    value: lowered_step,
                },
                span_ref,
            )?;
        } else {
            let step_region = self.current_region_or_error()?;
            let default_step = self.range_loop_unit_literal(locals.step, span_ref, step_region)?;

            self.emit_statement_kind(
                HirStatementKind::Write {
                    target: HirWriteTarget::DefineLocal(locals.step.local),
                    value: default_step,
                },
                span_ref,
            )?;
        }

        let index_region = self.current_region_or_error()?;
        let zero_index = self.make_expression(
            span_ref,
            HirExpressionKind::FixedScalar(
                FixedScalarValue::signed(FixedScalar::I32, 0)
                    .expect("zero is representable in I32"),
            ),
            types.int_type,
            ValueKind::Const,
            index_region,
        )?;
        self.emit_statement_kind(
            HirStatementKind::Write {
                target: HirWriteTarget::DefineLocal(locals.iteration_index),
                value: zero_index,
            },
            span_ref,
        )
    }

    /// Fails a dynamic zero step before any loop bound or emptiness check.
    ///
    /// WHAT: branches from `zero_check` to the recoverable built-in step failure or to
    ///       `non_zero_target`.
    /// WHY: range and collection loops share one language-defined failure lane, and a zero step
    ///      must fail even when the loop would visit nothing.
    #[allow(
        clippy::too_many_arguments,
        reason = "the guard names its step value, three CFG destinations, both spans and two edge labels"
    )]
    fn emit_loop_zero_step_guard(
        &mut self,
        step: LoopStepOperand,
        zero_check: BlockId,
        zero_failure: BlockId,
        non_zero_target: BlockId,
        span_ref: &Option<SourceSpan>,
        step_span: &Option<SourceSpan>,
        zero_label: &str,
        non_zero_label: &str,
    ) -> Result<(), HirConstructionFailure> {
        // Retain the authored step span: this is a language-defined failure, not an invariant trap.
        self.set_current_block(zero_check)?;
        let zero_check_region = self.current_region_or_error()?;
        let step_value =
            self.emit_loop_local_load(step.local, step.ty, span_ref, zero_check_region)?;
        let zero_literal = self.emit_loop_step_zero_literal(step, span_ref, zero_check_region)?;
        let step_is_zero = self.emit_loop_comparison(
            step_value,
            HirBinOp::Eq,
            zero_literal,
            builtin_type_ids::BOOL,
            span_ref,
            zero_check_region,
        )?;
        self.emit_terminator(
            zero_check,
            HirTerminator::If {
                condition: step_is_zero,
                then_block: zero_failure,
                else_block: non_zero_target,
            },
            span_ref,
        )?;
        self.log_control_flow_edge(zero_check, zero_failure, zero_label);
        self.log_control_flow_edge(zero_check, non_zero_target, non_zero_label);

        self.set_current_block(zero_failure)?;
        self.emit_loop_step_failure(LoopStepFailureCause::ZeroStep, step_span)
    }

    /// Records the loop direction once from the evaluated non-zero step.
    ///
    /// WHY: the step sign alone selects the header and candidate bounds. The loop never infers a
    ///      direction from its start and end bounds, and the zero guard has already run.
    fn emit_range_loop_direction(
        &mut self,
        runtime: RangeLoopRuntime,
        span_ref: &Option<SourceSpan>,
    ) -> Result<(), HirConstructionFailure> {
        let RangeLoopRuntime {
            blocks,
            locals,
            types,
        } = runtime;

        self.set_current_block(blocks.header_selector)?;
        let direction_region = self.current_region_or_error()?;
        let ascending_value = self.emit_loop_step_positive_test(
            locals.step,
            types.bool_type,
            span_ref,
            direction_region,
        )?;
        self.emit_statement_kind(
            HirStatementKind::Write {
                target: HirWriteTarget::DefineLocal(locals.ascending),
                value: ascending_value,
            },
            span_ref,
        )
    }

    fn emit_range_loop_header_checks(
        &mut self,
        expressions: &ExpressionStore,
        range: &RangeLoopSpec,
        runtime: RangeLoopRuntime,
        span_ref: &Option<SourceSpan>,
        span: Option<SourceSpan>,
    ) -> Result<(), HirConstructionFailure> {
        let RangeLoopRuntime {
            blocks,
            locals,
            types,
        } = runtime;

        self.set_current_block(blocks.header_selector)?;
        let header_selector_region = self.current_region_or_error()?;
        let ascending_for_header = self.make_expression(
            span_ref,
            HirExpressionKind::Load(HirPlace::local(locals.ascending)),
            types.bool_type,
            ValueKind::Place,
            header_selector_region,
        )?;
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
        self.set_current_block(blocks.header_ascending)?;
        let header_ascending_region = self.current_region_or_error()?;
        let asc_current_value = self.make_expression(
            span_ref,
            HirExpressionKind::Load(HirPlace::local(locals.current)),
            types.binding,
            ValueKind::Place,
            header_ascending_region,
        )?;
        let asc_end_value = self.make_expression(
            span_ref,
            HirExpressionKind::Load(HirPlace::local(locals.end)),
            types.binding,
            ValueKind::Place,
            header_ascending_region,
        )?;
        let asc_comparison_op = match range.end_kind {
            RangeEndKind::Exclusive => HirBinOp::Lt,
            RangeEndKind::Inclusive => HirBinOp::Le,
        };
        let asc_condition = self.make_expression(
            span_ref,
            HirExpressionKind::BinOp {
                left: asc_current_value,
                op: asc_comparison_op,
                right: asc_end_value,
            },
            types.bool_type,
            ValueKind::RValue,
            header_ascending_region,
        )?;
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
            span.or(expressions.view(range.end)?.span()),
        )?;
        self.log_control_flow_edge(blocks.header_ascending, blocks.body, "for.asc.true");
        self.log_control_flow_edge(blocks.header_ascending, blocks.exit, "for.asc.false");

        self.set_current_block(blocks.header_descending)?;
        let header_descending_region = self.current_region_or_error()?;
        let desc_current_value = self.make_expression(
            span_ref,
            HirExpressionKind::Load(HirPlace::local(locals.current)),
            types.binding,
            ValueKind::Place,
            header_descending_region,
        )?;
        let desc_end_value = self.make_expression(
            span_ref,
            HirExpressionKind::Load(HirPlace::local(locals.end)),
            types.binding,
            ValueKind::Place,
            header_descending_region,
        )?;
        let desc_comparison_op = match range.end_kind {
            RangeEndKind::Exclusive => HirBinOp::Gt,
            RangeEndKind::Inclusive => HirBinOp::Ge,
        };
        let desc_condition = self.make_expression(
            span_ref,
            HirExpressionKind::BinOp {
                left: desc_current_value,
                op: desc_comparison_op,
                right: desc_end_value,
            },
            types.bool_type,
            ValueKind::RValue,
            header_descending_region,
        )?;
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
            span.or(expressions.view(range.end)?.span()),
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
        emit_body: &mut impl FnMut(&mut HirBuilder<'_>) -> Result<(), HirConstructionFailure>,
    ) -> Result<bool, HirConstructionFailure> {
        let RangeLoopRuntime {
            blocks,
            locals,
            types,
        } = runtime;

        self.set_current_block(blocks.body)?;
        let body_region_id = self.current_region_or_error()?;
        let mut visible_bindings = Vec::new();

        if let Some(item_binding) = &bindings.item {
            let body_current_value = self.make_expression(
                span_ref,
                HirExpressionKind::Copy(HirPlace::local(locals.current)),
                types.binding,
                ValueKind::RValue,
                body_region_id,
            )?;
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
                HirExpressionKind::Copy(HirPlace::local(locals.iteration_index)),
                types.int_type,
                ValueKind::RValue,
                body_region_id,
            )?;
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

        let body_tail_block = self.current_block_id_or_error()?;
        if !self.block_has_explicit_terminator(body_tail_block)? {
            self.emit_jump_to(body_tail_block, blocks.step, span_ref, "for.body.step")?;
        }

        Ok(!self.discard_unreachable_empty_block(blocks.step)?)
    }

    fn emit_range_loop_step(
        &mut self,
        runtime: RangeLoopRuntime,
        end_kind: RangeEndKind,
        span_ref: &Option<SourceSpan>,
        step_span: &Option<SourceSpan>,
    ) -> Result<(), HirConstructionFailure> {
        let RangeLoopRuntime {
            blocks,
            locals,
            types,
        } = runtime;
        let inclusive = matches!(end_kind, RangeEndKind::Inclusive);

        // An inclusive endpoint has no successor, so its equality test runs before the candidate.
        self.set_current_block(blocks.step)?;
        if inclusive {
            let step_region = self.current_region_or_error()?;
            let current =
                self.emit_loop_local_load(locals.current, types.binding, span_ref, step_region)?;
            let end =
                self.emit_loop_local_load(locals.end, types.binding, span_ref, step_region)?;
            let at_endpoint = self.emit_loop_comparison(
                current,
                HirBinOp::Eq,
                end,
                types.bool_type,
                span_ref,
                step_region,
            )?;
            self.emit_loop_branch(
                at_endpoint,
                blocks.exit,
                blocks.step_candidate,
                span_ref,
                "for.step.inclusive-end",
                "for.step.candidate",
            )?;
        } else {
            self.emit_jump_from_current_block(
                blocks.step_candidate,
                span_ref,
                "for.step.candidate",
            )?;
        }

        // One backend-neutral candidate computes the signed successor and reports both its value
        // and whether it lies within the bound in the step's direction. The authored step
        // position is the candidate's location: a target that cannot round this counter domain
        // (an `F16` loop on Wasm) must report the step the author can repair.
        self.set_current_block(blocks.step_candidate)?;
        let candidate_region = self.current_region_or_error()?;
        let current =
            self.emit_loop_local_load(locals.current, types.binding, span_ref, candidate_region)?;
        let step = self.emit_loop_local_load(
            locals.step.local,
            locals.step.ty,
            span_ref,
            candidate_region,
        )?;
        let end =
            self.emit_loop_local_load(locals.end, types.binding, span_ref, candidate_region)?;
        self.emit_statement_kind_with_span(
            HirStatementKind::RangeCandidate {
                current,
                step,
                end,
                inclusive,
                domain: types.domain,
                candidate_result: HirLocalDestination::Define(locals.next_value),
                in_range_result: HirLocalDestination::Define(locals.candidate_in_range),
            },
            span_ref,
            *step_span,
        )?;

        let in_range = self.emit_loop_local_load(
            locals.candidate_in_range,
            types.bool_type,
            span_ref,
            candidate_region,
        )?;
        let in_range_target = blocks
            .step_stall
            .map_or(blocks.step_commit, |stall| stall.check);
        self.emit_loop_branch(
            in_range,
            in_range_target,
            blocks.exit,
            span_ref,
            "for.step.candidate-in-range",
            "for.step.candidate-out",
        )?;

        // Binary floats alone can round a candidate back onto the current value.
        if let Some(stall) = blocks.step_stall {
            self.set_current_block(stall.check)?;
            let stall_region = self.current_region_or_error()?;
            let current =
                self.emit_loop_local_load(locals.current, types.binding, span_ref, stall_region)?;
            let candidate = self.emit_loop_local_load(
                locals.next_value,
                types.binding,
                span_ref,
                stall_region,
            )?;
            let stalled = self.emit_loop_comparison(
                candidate,
                HirBinOp::Eq,
                current,
                types.bool_type,
                span_ref,
                stall_region,
            )?;
            self.emit_loop_branch(
                stalled,
                stall.failure,
                blocks.step_commit,
                span_ref,
                "for.step.stalled",
                "for.step.advances",
            )?;

            self.set_current_block(stall.failure)?;
            self.emit_loop_step_failure(LoopStepFailureCause::NoProgress, step_span)?;
        }

        self.set_current_block(blocks.step_commit)?;
        let commit_region = self.current_region_or_error()?;
        let candidate = self.make_expression(
            span_ref,
            HirExpressionKind::Copy(HirPlace::local(locals.next_value)),
            types.binding,
            ValueKind::RValue,
            commit_region,
        )?;
        self.emit_statement_kind(
            HirStatementKind::Write {
                target: HirWriteTarget::AssignPlace(HirPlace::local(locals.current)),
                value: candidate,
            },
            span_ref,
        )?;

        let index_current = self.emit_loop_local_load(
            locals.iteration_index,
            types.int_type,
            span_ref,
            commit_region,
        )?;
        let index_delta = self.make_expression(
            span_ref,
            HirExpressionKind::FixedScalar(
                FixedScalarValue::signed(FixedScalar::I32, 1).expect("one is representable in I32"),
            ),
            types.int_type,
            ValueKind::Const,
            commit_region,
        )?;
        self.emit_checked_numeric_assignment(
            HirLocalDestination::Update(locals.iteration_index),
            HirNumericOp {
                operator: NumericOperator::Add,
                domain: NumericScalar::Fixed(FixedScalar::I32),
            },
            index_current,
            index_delta,
            span_ref,
        )?;
        // The candidate already proved the bound in the step direction, so the next iteration
        // re-enters the body directly instead of repeating the header test.
        self.emit_jump_from_current_block(blocks.body, span_ref, "for.backedge")
    }

    fn emit_loop_local_load(
        &mut self,
        local: LocalId,
        ty: TypeId,
        span_ref: &Option<SourceSpan>,
        region: RegionId,
    ) -> Result<HirValueId, HirConstructionFailure> {
        self.make_expression(
            span_ref,
            HirExpressionKind::Load(HirPlace::local(local)),
            ty,
            ValueKind::Place,
            region,
        )
    }

    fn emit_loop_comparison(
        &mut self,
        left: HirValueId,
        operator: HirBinOp,
        right: HirValueId,
        bool_type: TypeId,
        span_ref: &Option<SourceSpan>,
        region: RegionId,
    ) -> Result<HirValueId, HirConstructionFailure> {
        self.make_expression(
            span_ref,
            HirExpressionKind::BinOp {
                left,
                op: operator,
                right,
            },
            bool_type,
            ValueKind::RValue,
            region,
        )
    }

    fn emit_loop_branch(
        &mut self,
        condition: HirValueId,
        then_block: BlockId,
        else_block: BlockId,
        span_ref: &Option<SourceSpan>,
        then_label: &str,
        else_label: &str,
    ) -> Result<(), HirConstructionFailure> {
        let source_block = self.current_block_id_or_error()?;
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

    /// A zero of the step's own fixed type, used by the zero guard and sign test.
    fn emit_loop_step_zero_literal(
        &mut self,
        step: LoopStepOperand,
        span_ref: &Option<SourceSpan>,
        region: RegionId,
    ) -> Result<HirValueId, HirConstructionFailure> {
        let zero = match step.scalar.class() {
            FixedScalarClass::SignedInteger => FixedScalarValue::signed(step.scalar, 0),
            FixedScalarClass::BinaryFloat => FixedScalarValue::binary_float(step.scalar, 0.0),
            FixedScalarClass::UnsignedInteger | FixedScalarClass::Octet => {
                return Err(InfrastructureFailure::hir_transformation(
                    "Loop step must be a signed integer or binary float",
                )
                .into());
            }
        }
        .expect("zero is representable in the step's fixed type");
        self.make_expression(
            span_ref,
            HirExpressionKind::FixedScalar(zero),
            step.ty,
            ValueKind::Const,
            region,
        )
    }

    /// Evaluates `step > 0` once from the step's own type.
    ///
    /// WHY: the sign alone selects loop direction. Range loops record it and collection loops
    ///      use it to choose the first source index and inclusive final index.
    fn emit_loop_step_positive_test(
        &mut self,
        step: LoopStepOperand,
        bool_type: TypeId,
        span_ref: &Option<SourceSpan>,
        region: RegionId,
    ) -> Result<HirValueId, HirConstructionFailure> {
        let step_value = self.emit_loop_local_load(step.local, step.ty, span_ref, region)?;
        let zero = self.emit_loop_step_zero_literal(step, span_ref, region)?;
        self.emit_loop_comparison(step_value, HirBinOp::Gt, zero, bool_type, span_ref, region)
    }

    /// The default omitted step: positive one of the independent `I32` step type.
    fn range_loop_unit_literal(
        &mut self,
        step: LoopStepOperand,
        span_ref: &Option<SourceSpan>,
        region: RegionId,
    ) -> Result<HirValueId, HirConstructionFailure> {
        let one = FixedScalarValue::signed(step.scalar, 1)
            .expect("one is representable in the step's fixed type");
        self.make_expression(
            span_ref,
            HirExpressionKind::FixedScalar(one),
            step.ty,
            ValueKind::Const,
            region,
        )
    }

    pub(super) fn lower_collection_loop_statement_impl(
        &mut self,
        bindings: &LoopBindings,
        iterable: ExpressionView<'_>,
        step: Option<ExprId>,
        body: &[AstNode],
        span_ref: &Option<SourceSpan>,
        span: Option<SourceSpan>,
    ) -> Result<(), HirConstructionFailure> {
        let expressions = iterable.store();
        self.lower_collection_loop_with_body_emitter_and_span(
            bindings,
            iterable,
            step,
            span_ref,
            span,
            |builder| builder.lower_statement_sequence(expressions, body),
        )
    }

    pub(crate) fn lower_collection_loop_with_body_emitter(
        &mut self,
        bindings: &LoopBindings,
        iterable: ExpressionView<'_>,
        step: Option<ExprId>,
        span_ref: &Option<SourceSpan>,
        mut emit_body: impl FnMut(&mut HirBuilder<'_>) -> Result<(), HirConstructionFailure>,
    ) -> Result<(), HirConstructionFailure> {
        // Template loops are generated scaffolding: headers stay spanless. Statement
        // loops use the span-aware entry below with the authored `for` span.
        self.lower_collection_loop_with_body_emitter_and_span(
            bindings,
            iterable,
            step,
            span_ref,
            None,
            &mut emit_body,
        )
    }

    /// Builds the one collection-loop CFG shared by statements and runtime templates.
    ///
    /// WHAT: evaluates the source into a compiler temporary, snapshots its length, then either
    ///       installs the lean default `+1` header or the stepped pipeline: one evaluated step,
    ///       zero guard, empty-source exit, sign-selected first and final index, and one
    ///       `RangeCandidate` per advance.
    /// WHY: a collection source and its length must be captured before an authored step, and a
    ///      dynamic zero step must fail even when the loop would visit nothing. Both shapes keep
    ///      the same body and loop targets so `break` and `continue` stay independent of stepping.
    fn lower_collection_loop_with_body_emitter_and_span(
        &mut self,
        bindings: &LoopBindings,
        iterable: ExpressionView<'_>,
        step: Option<ExprId>,
        span_ref: &Option<SourceSpan>,
        span: Option<SourceSpan>,
        mut emit_body: impl FnMut(&mut HirBuilder<'_>) -> Result<(), HirConstructionFailure>,
    ) -> Result<(), HirConstructionFailure> {
        // Generated collection spills: the iterable/length/index temps, the length call, step
        // evaluation and jump edges are compiler scaffolding and stay spanless. Authored spans
        // stay on the header/emptiness check and on loop-step failures.
        let runtime = self.create_collection_loop_runtime(iterable, step, span_ref)?;
        self.capture_collection_loop_source_and_length(iterable, runtime, span_ref)?;

        let header_span = span.or(iterable.span());
        match runtime.mode {
            CollectionLoopMode::Default { header } => {
                self.emit_collection_loop_default_seed(runtime, header, span_ref, header_span)?;
            }
            CollectionLoopMode::Stepped {
                step,
                blocks,
                locals,
            } => {
                let step_view = iterable.store().view(step)?;
                self.emit_collection_loop_stepped_prefix(
                    step_view,
                    runtime,
                    blocks,
                    locals,
                    span_ref,
                    header_span,
                )?;
            }
        }

        let step_block_is_reachable = self.lower_collection_loop_body_with_emitter(
            bindings,
            runtime,
            span_ref,
            &mut emit_body,
        )?;
        if step_block_is_reachable {
            match runtime.mode {
                CollectionLoopMode::Default { header } => {
                    self.emit_collection_loop_default_step(runtime, header, span_ref)?;
                }
                CollectionLoopMode::Stepped {
                    step,
                    blocks,
                    locals,
                } => {
                    // The authored step position anchors the candidate that a target may reject.
                    let step_span = iterable.store().view(step)?.span();
                    self.emit_collection_loop_stepped_step(
                        runtime, blocks, locals, span_ref, &step_span,
                    )?;
                }
            }
        } else if let CollectionLoopMode::Stepped { blocks, .. } = runtime.mode {
            self.discard_unreachable_collection_step_blocks(blocks)?;
        }

        Ok(self.set_current_block(runtime.blocks.exit)?)
    }

    fn create_collection_loop_runtime(
        &mut self,
        iterable: ExpressionView<'_>,
        step: Option<ExprId>,
        span_ref: &Option<SourceSpan>,
    ) -> Result<CollectionLoopRuntime, HirConstructionFailure> {
        let expressions = iterable.store();
        let parent_region = self.current_region_or_error()?;
        let body_region = self.create_child_region(parent_region);
        let body = self.create_block(body_region, span_ref, "loop-collection-body");
        let step_block = self.create_block(parent_region, span_ref, "loop-collection-step");
        let exit = self.create_block(parent_region, span_ref, "loop-collection-exit");

        let (iterable_type, element_type) = self.collection_iteration_types(iterable)?;
        let types = CollectionLoopTypes {
            iterable: iterable_type,
            element: element_type,
            // Collection lengths and positional indices use the language's exact I32 identity.
            index: builtin_type_ids::fixed_scalar(FixedScalar::I32),
            bool_type: builtin_type_ids::BOOL,
        };

        // Allocation order is observable in HIR snapshots and must remain stable.
        let iterable = self.allocate_temp_local(types.iterable, None)?;
        let length = self.allocate_temp_local(types.index, None)?;
        let index = self.allocate_temp_local(types.index, None)?;

        let mode = match step {
            Some(step) => {
                let step_view = expressions.view(step)?;
                let (step_type, step_scalar) =
                    self.resolve_collection_loop_step_types(step_view)?;
                let last = self.allocate_temp_local(types.index, None)?;
                let step_local = self.allocate_temp_local(step_type, None)?;
                let next_value = self.allocate_temp_local(types.index, None)?;
                let candidate_in_range = self.allocate_temp_local(types.bool_type, None)?;
                CollectionLoopMode::Stepped {
                    step,
                    blocks: self.create_collection_loop_step_blocks(parent_region, span_ref),
                    locals: CollectionLoopSteppedLocals {
                        last,
                        step: LoopStepOperand {
                            local: step_local,
                            ty: step_type,
                            scalar: step_scalar,
                        },
                        next_value,
                        candidate_in_range,
                    },
                }
            }
            None => CollectionLoopMode::Default {
                header: self.create_block(parent_region, span_ref, "loop-collection-header"),
            },
        };

        Ok(CollectionLoopRuntime {
            blocks: CollectionLoopBlocks {
                body,
                step: step_block,
                exit,
            },
            locals: CollectionLoopLocals {
                iterable,
                length,
                index,
            },
            mode,
            types,
        })
    }

    fn create_collection_loop_step_blocks(
        &mut self,
        parent_region: RegionId,
        span_ref: &Option<SourceSpan>,
    ) -> CollectionLoopStepBlocks {
        CollectionLoopStepBlocks {
            zero_check: self.create_block(
                parent_region,
                span_ref,
                "loop-collection-step-zero-check",
            ),
            zero_failure: self.create_block(
                parent_region,
                span_ref,
                "loop-collection-step-zero-failure",
            ),
            empty_check: self.create_block(parent_region, span_ref, "loop-collection-empty-check"),
            seed_selector: self.create_block(
                parent_region,
                span_ref,
                "loop-collection-seed-selector",
            ),
            seed_positive: self.create_block(
                parent_region,
                span_ref,
                "loop-collection-seed-positive",
            ),
            seed_negative: self.create_block(
                parent_region,
                span_ref,
                "loop-collection-seed-negative",
            ),
            step_candidate: self.create_block(
                parent_region,
                span_ref,
                "loop-collection-step-candidate",
            ),
            step_commit: self.create_block(parent_region, span_ref, "loop-collection-step-commit"),
        }
    }

    /// Captures the actual source and its length exactly once.
    ///
    /// WHY: the iterable temporary's definition stays the single `Load`-of-place write that the
    ///      borrow checker follows to the source root, so later body mutation conflicts remain
    ///      rejected. The length snapshot also means later source mutation cannot change the
    ///      loop bound.
    fn capture_collection_loop_source_and_length(
        &mut self,
        iterable: ExpressionView<'_>,
        runtime: CollectionLoopRuntime,
        span_ref: &Option<SourceSpan>,
    ) -> Result<(), HirConstructionFailure> {
        let lowered_iterable = self.lower_expression_value_to_current_block(iterable)?;
        self.emit_statement_kind(
            HirStatementKind::Write {
                target: HirWriteTarget::DefineLocal(runtime.locals.iterable),
                value: lowered_iterable,
            },
            span_ref,
        )?;

        let length_region = self.current_region_or_error()?;
        let iterable_for_length = self.emit_loop_local_load(
            runtime.locals.iterable,
            runtime.types.iterable,
            span_ref,
            length_region,
        )?;
        let length_args = self
            .module
            .expressions
            .append_values(&[iterable_for_length], *span_ref)?;
        self.emit_statement_kind(
            HirStatementKind::Call {
                target: CallTarget::External(ExternalFunctionId::CollectionLength),
                args: length_args,
                results: vec![HirLocalDestination::Define(runtime.locals.length)],
            },
            span_ref,
        )
    }

    /// Seeds the default `+1` loop and installs its repeated header.
    ///
    /// WHY: the default stride needs no zero guard, emptiness branch or inclusive final index.
    ///      Its header compares the index with the captured length and its step is a checked add,
    ///      which is the smallest CFG that preserves `continue`, `break` and overflow semantics.
    fn emit_collection_loop_default_seed(
        &mut self,
        runtime: CollectionLoopRuntime,
        header: BlockId,
        span_ref: &Option<SourceSpan>,
        header_span: Option<SourceSpan>,
    ) -> Result<(), HirConstructionFailure> {
        let seed_region = self.current_region_or_error()?;
        let zero_index = self.emit_collection_index_literal(0, span_ref, seed_region)?;
        self.emit_statement_kind(
            HirStatementKind::Write {
                target: HirWriteTarget::DefineLocal(runtime.locals.index),
                value: zero_index,
            },
            span_ref,
        )?;

        self.emit_jump_from_current_block(header, span_ref, "loop.collection.enter")?;

        self.set_current_block(header)?;
        let header_region = self.current_region_or_error()?;
        let current_index = self.emit_loop_local_load(
            runtime.locals.index,
            runtime.types.index,
            span_ref,
            header_region,
        )?;
        let collection_length = self.emit_loop_local_load(
            runtime.locals.length,
            runtime.types.index,
            span_ref,
            header_region,
        )?;
        let continue_condition = self.emit_loop_comparison(
            current_index,
            HirBinOp::Lt,
            collection_length,
            runtime.types.bool_type,
            span_ref,
            header_region,
        )?;
        // Authored loop header: the bounds check carries the `for` statement span,
        // falling back to the iterable span when identity-free.
        self.emit_terminator_with_span(
            header,
            HirTerminator::If {
                condition: continue_condition,
                then_block: runtime.blocks.body,
                else_block: runtime.blocks.exit,
            },
            span_ref,
            header_span,
        )?;
        self.log_control_flow_edge(header, runtime.blocks.body, "loop.collection.true");
        self.log_control_flow_edge(header, runtime.blocks.exit, "loop.collection.false");

        Ok(())
    }

    /// Evaluates and guards an explicit step, proves the source nonempty and seeds by sign.
    ///
    /// Ordering is semantic: source and length were captured by the caller, then the step is
    /// evaluated once, then a zero step fails before any emptiness check, then an empty source
    /// exits, then the sign chooses the first and inclusive final source index.
    fn emit_collection_loop_stepped_prefix(
        &mut self,
        step_view: ExpressionView<'_>,
        runtime: CollectionLoopRuntime,
        stepped_blocks: CollectionLoopStepBlocks,
        stepped_locals: CollectionLoopSteppedLocals,
        span_ref: &Option<SourceSpan>,
        header_span: Option<SourceSpan>,
    ) -> Result<(), HirConstructionFailure> {
        let lowered_step = self.lower_expression_value_to_current_block(step_view)?;
        let lowered_step = self.snapshot_loop_operand(lowered_step)?;
        let step_span = step_view.span();
        self.emit_statement_kind(
            HirStatementKind::Write {
                target: HirWriteTarget::DefineLocal(stepped_locals.step.local),
                value: lowered_step,
            },
            span_ref,
        )?;

        self.emit_jump_from_current_block(
            stepped_blocks.zero_check,
            span_ref,
            "loop.collection.step.enter",
        )?;
        self.emit_loop_zero_step_guard(
            stepped_locals.step,
            stepped_blocks.zero_check,
            stepped_blocks.zero_failure,
            stepped_blocks.empty_check,
            span_ref,
            &step_span,
            "loop.collection.step.zero",
            "loop.collection.step.non_zero",
        )?;

        self.emit_collection_loop_empty_check(runtime, stepped_blocks, span_ref, header_span)?;
        self.emit_collection_loop_sign_seed(runtime, stepped_locals, stepped_blocks, span_ref)
    }

    /// Exits before the first visit when the captured source length is zero.
    ///
    /// WHY: a negative seed would otherwise build the invalid index `length - 1`, and a positive
    ///      seed would enter the body once for an empty source.
    fn emit_collection_loop_empty_check(
        &mut self,
        runtime: CollectionLoopRuntime,
        stepped_blocks: CollectionLoopStepBlocks,
        span_ref: &Option<SourceSpan>,
        header_span: Option<SourceSpan>,
    ) -> Result<(), HirConstructionFailure> {
        self.set_current_block(stepped_blocks.empty_check)?;
        let empty_region = self.current_region_or_error()?;
        let collection_length = self.emit_loop_local_load(
            runtime.locals.length,
            runtime.types.index,
            span_ref,
            empty_region,
        )?;
        let zero_length = self.emit_collection_index_literal(0, span_ref, empty_region)?;
        let is_empty = self.emit_loop_comparison(
            collection_length,
            HirBinOp::Eq,
            zero_length,
            runtime.types.bool_type,
            span_ref,
            empty_region,
        )?;
        self.emit_terminator_with_span(
            stepped_blocks.empty_check,
            HirTerminator::If {
                condition: is_empty,
                then_block: runtime.blocks.exit,
                else_block: stepped_blocks.seed_selector,
            },
            span_ref,
            header_span,
        )?;
        self.log_control_flow_edge(
            stepped_blocks.empty_check,
            runtime.blocks.exit,
            "loop.collection.empty",
        );
        self.log_control_flow_edge(
            stepped_blocks.empty_check,
            stepped_blocks.seed_selector,
            "loop.collection.non_empty",
        );
        Ok(())
    }

    /// Seeds the cursor and inclusive final index from the evaluated step sign.
    ///
    /// WHAT: captures `length - 1` once after the emptiness check, then a positive step starts
    ///       at zero with the captured final index, while a negative step starts at the captured
    ///       final index and resets the final index to zero.
    fn emit_collection_loop_sign_seed(
        &mut self,
        runtime: CollectionLoopRuntime,
        stepped_locals: CollectionLoopSteppedLocals,
        stepped_blocks: CollectionLoopStepBlocks,
        span_ref: &Option<SourceSpan>,
    ) -> Result<(), HirConstructionFailure> {
        self.set_current_block(stepped_blocks.seed_selector)?;
        let selector_region = self.current_region_or_error()?;
        let collection_length = self.emit_loop_local_load(
            runtime.locals.length,
            runtime.types.index,
            span_ref,
            selector_region,
        )?;
        let one = self.emit_collection_index_literal(1, span_ref, selector_region)?;
        // The empty check already proved the captured length is at least one, so `length - 1`
        // is always representable in `I32`. Emitting this through the checked path would add an
        // `IntOverflow` edge the collection loop contract never raises and that definition
        // classification deliberately omits.
        self.emit_numeric_op_statement(
            HirNumericOp {
                operator: NumericOperator::Subtract,
                domain: NumericScalar::Fixed(FixedScalar::I32),
            },
            NumericFailureMode::Infallible,
            HirNumericOperands::Binary {
                left: collection_length,
                right: one,
            },
            HirLocalDestination::Define(stepped_locals.last),
            span_ref,
        )?;

        let step_is_positive = self.emit_loop_step_positive_test(
            stepped_locals.step,
            runtime.types.bool_type,
            span_ref,
            selector_region,
        )?;
        self.emit_loop_branch(
            step_is_positive,
            stepped_blocks.seed_positive,
            stepped_blocks.seed_negative,
            span_ref,
            "loop.collection.step.positive",
            "loop.collection.step.negative",
        )?;

        self.set_current_block(stepped_blocks.seed_positive)?;
        let positive_region = self.current_region_or_error()?;
        let zero_index = self.emit_collection_index_literal(0, span_ref, positive_region)?;
        self.emit_statement_kind(
            HirStatementKind::Write {
                target: HirWriteTarget::DefineLocal(runtime.locals.index),
                value: zero_index,
            },
            span_ref,
        )?;
        self.emit_jump_from_current_block(
            runtime.blocks.body,
            span_ref,
            "loop.collection.seed.body",
        )?;

        self.set_current_block(stepped_blocks.seed_negative)?;
        let negative_region = self.current_region_or_error()?;
        let last_index = self.make_expression(
            span_ref,
            HirExpressionKind::Copy(HirPlace::local(stepped_locals.last)),
            runtime.types.index,
            ValueKind::RValue,
            negative_region,
        )?;
        self.emit_statement_kind(
            HirStatementKind::Write {
                target: HirWriteTarget::DefineLocal(runtime.locals.index),
                value: last_index,
            },
            span_ref,
        )?;
        let zero_last = self.emit_collection_index_literal(0, span_ref, negative_region)?;
        self.emit_statement_kind(
            HirStatementKind::Write {
                target: HirWriteTarget::AssignPlace(HirPlace::local(stepped_locals.last)),
                value: zero_last,
            },
            span_ref,
        )?;
        self.emit_jump_from_current_block(
            runtime.blocks.body,
            span_ref,
            "loop.collection.seed.body",
        )
    }

    fn lower_collection_loop_body_with_emitter(
        &mut self,
        bindings: &LoopBindings,
        runtime: CollectionLoopRuntime,
        span_ref: &Option<SourceSpan>,
        emit_body: &mut impl FnMut(&mut HirBuilder<'_>) -> Result<(), HirConstructionFailure>,
    ) -> Result<bool, HirConstructionFailure> {
        self.set_current_block(runtime.blocks.body)?;
        let body_region_id = self.current_region_or_error()?;
        let mut visible_bindings = Vec::new();

        if let Some(item_binding) = &bindings.item {
            let item_index = self.emit_loop_local_load(
                runtime.locals.index,
                runtime.types.index,
                span_ref,
                body_region_id,
            )?;
            let item_place = HirPlace::local(runtime.locals.iterable).with_index(
                item_index,
                &mut self.module.expressions,
                *span_ref,
            )?;
            let item_value = self.make_expression(
                span_ref,
                HirExpressionKind::Load(item_place),
                runtime.types.element,
                ValueKind::Place,
                body_region_id,
            )?;
            let binding = self.register_loop_binding_local(
                item_binding,
                runtime.types.element,
                item_value,
                &visible_bindings,
                span_ref,
            )?;
            visible_bindings.push(binding);
        }

        if let Some(index_binding) = &bindings.index {
            // The second binding is the actual source index, already the cursor local.
            let user_index_value = self.emit_loop_local_load(
                runtime.locals.index,
                runtime.types.index,
                span_ref,
                body_region_id,
            )?;
            let binding = self.register_loop_binding_local(
                index_binding,
                runtime.types.index,
                user_index_value,
                &visible_bindings,
                span_ref,
            )?;
            visible_bindings.push(binding);
        }

        self.push_loop_targets(runtime.blocks.exit, runtime.blocks.step);
        let body_result =
            self.with_temporary_local_bindings(visible_bindings, |builder| emit_body(builder));
        self.pop_loop_targets();
        body_result?;

        let body_tail_block = self.current_block_id_or_error()?;
        if !self.block_has_explicit_terminator(body_tail_block)? {
            self.emit_jump_to(
                body_tail_block,
                runtime.blocks.step,
                span_ref,
                "loop.collection.body.step",
            )?;
        }

        Ok(!self.discard_unreachable_empty_block(runtime.blocks.step)?)
    }

    /// Advances the default `+1` loop and re-tests its header.
    fn emit_collection_loop_default_step(
        &mut self,
        runtime: CollectionLoopRuntime,
        header: BlockId,
        span_ref: &Option<SourceSpan>,
    ) -> Result<(), HirConstructionFailure> {
        self.set_current_block(runtime.blocks.step)?;
        let step_region = self.current_region_or_error()?;
        let step_current = self.emit_loop_local_load(
            runtime.locals.index,
            runtime.types.index,
            span_ref,
            step_region,
        )?;
        let step_delta = self.emit_collection_index_literal(1, span_ref, step_region)?;
        self.emit_checked_numeric_assignment(
            HirLocalDestination::Update(runtime.locals.index),
            HirNumericOp {
                operator: NumericOperator::Add,
                domain: NumericScalar::Fixed(FixedScalar::I32),
            },
            step_current,
            step_delta,
            span_ref,
        )?;
        self.emit_jump_from_current_block(header, span_ref, "loop.collection.backedge")
    }

    /// Advances a stepped loop without ever constructing an invalid source index.
    ///
    /// WHAT: exits when the cursor is the inclusive final index, otherwise produces one
    ///       `RangeCandidate` over the `I32` index with the step's own type, anchored on the
    ///       authored step position. A candidate outside the final-index bound exits, and an
    ///       in-bound candidate commits the cursor and re-enters the body.
    /// WHY: this is the shared narrow operation that adds a wider signed step to an `I32` index
    ///      exactly, so a huge negative step visits the last element once and exits instead of
    ///      materialising a negative or narrowed index.
    fn emit_collection_loop_stepped_step(
        &mut self,
        runtime: CollectionLoopRuntime,
        stepped_blocks: CollectionLoopStepBlocks,
        stepped_locals: CollectionLoopSteppedLocals,
        span_ref: &Option<SourceSpan>,
        step_span: &Option<SourceSpan>,
    ) -> Result<(), HirConstructionFailure> {
        self.set_current_block(runtime.blocks.step)?;
        let step_region = self.current_region_or_error()?;
        let index = self.emit_loop_local_load(
            runtime.locals.index,
            runtime.types.index,
            span_ref,
            step_region,
        )?;
        let last = self.emit_loop_local_load(
            stepped_locals.last,
            runtime.types.index,
            span_ref,
            step_region,
        )?;
        let at_last = self.emit_loop_comparison(
            index,
            HirBinOp::Eq,
            last,
            runtime.types.bool_type,
            span_ref,
            step_region,
        )?;
        self.emit_loop_branch(
            at_last,
            runtime.blocks.exit,
            stepped_blocks.step_candidate,
            span_ref,
            "loop.collection.step.last",
            "loop.collection.step.candidate",
        )?;

        self.set_current_block(stepped_blocks.step_candidate)?;
        let candidate_region = self.current_region_or_error()?;
        let current = self.emit_loop_local_load(
            runtime.locals.index,
            runtime.types.index,
            span_ref,
            candidate_region,
        )?;
        let step = self.emit_loop_local_load(
            stepped_locals.step.local,
            stepped_locals.step.ty,
            span_ref,
            candidate_region,
        )?;
        let end = self.emit_loop_local_load(
            stepped_locals.last,
            runtime.types.index,
            span_ref,
            candidate_region,
        )?;
        self.emit_statement_kind_with_span(
            HirStatementKind::RangeCandidate {
                current,
                step,
                end,
                inclusive: true,
                domain: NumericScalar::Fixed(FixedScalar::I32),
                candidate_result: HirLocalDestination::Define(stepped_locals.next_value),
                in_range_result: HirLocalDestination::Define(stepped_locals.candidate_in_range),
            },
            span_ref,
            *step_span,
        )?;

        let in_range = self.emit_loop_local_load(
            stepped_locals.candidate_in_range,
            runtime.types.bool_type,
            span_ref,
            candidate_region,
        )?;
        self.emit_loop_branch(
            in_range,
            stepped_blocks.step_commit,
            runtime.blocks.exit,
            span_ref,
            "loop.collection.step.candidate-in-range",
            "loop.collection.step.candidate-out",
        )?;

        self.set_current_block(stepped_blocks.step_commit)?;
        let commit_region = self.current_region_or_error()?;
        let candidate = self.make_expression(
            span_ref,
            HirExpressionKind::Copy(HirPlace::local(stepped_locals.next_value)),
            runtime.types.index,
            ValueKind::RValue,
            commit_region,
        )?;
        self.emit_statement_kind(
            HirStatementKind::Write {
                target: HirWriteTarget::AssignPlace(HirPlace::local(runtime.locals.index)),
                value: candidate,
            },
            span_ref,
        )?;
        // The candidate already proved the final-index bound, so the next iteration re-enters
        // the body directly instead of repeating a header test.
        self.emit_jump_from_current_block(runtime.blocks.body, span_ref, "loop.collection.backedge")
    }

    fn discard_unreachable_collection_step_blocks(
        &mut self,
        blocks: CollectionLoopStepBlocks,
    ) -> Result<(), HirConstructionFailure> {
        // Discard in CFG order: removing the candidate drops the edge that kept the commit alive.
        for block in [blocks.step_candidate, blocks.step_commit] {
            if !self.discard_unreachable_empty_block(block)? {
                return Err(InfrastructureFailure::hir_transformation(
                    "Unreachable collection-loop step block was not empty and unowned",
                )
                .into());
            }
        }
        Ok(())
    }

    fn resolve_collection_loop_step_types(
        &mut self,
        step: ExpressionView<'_>,
    ) -> Result<(TypeId, FixedScalar), HirConstructionFailure> {
        let step_type = self.lower_type_id(step.value_type_id()?)?;
        let step_scalar = match NumericScalar::from_type_id(step_type, &self.type_environment) {
            Some(NumericScalar::Fixed(scalar))
                if scalar.class() == FixedScalarClass::SignedInteger =>
            {
                scalar
            }
            _ => {
                return Err(InfrastructureFailure::hir_transformation(
                    "Collection-loop step did not complete with a signed fixed integer type",
                )
                .into());
            }
        };
        Ok((step_type, step_scalar))
    }

    fn emit_collection_index_literal(
        &mut self,
        value: i32,
        span_ref: &Option<SourceSpan>,
        region: RegionId,
    ) -> Result<HirValueId, HirConstructionFailure> {
        let literal = FixedScalarValue::signed(FixedScalar::I32, i64::from(value))
            .expect("collection index literal is representable in I32");
        self.make_expression(
            span_ref,
            HirExpressionKind::FixedScalar(literal),
            builtin_type_ids::fixed_scalar(FixedScalar::I32),
            ValueKind::Const,
            region,
        )
    }

    fn register_loop_binding_local(
        &mut self,
        binding: &crate::compiler_frontend::ast::ast_nodes::Declaration,
        ty: TypeId,
        value: HirValueId,
        visible_bindings: &[(PathId, LocalId)],
        span_ref: &Option<SourceSpan>,
    ) -> Result<(PathId, LocalId), HirConstructionFailure> {
        // AST scopes already enforce no-shadowing. This guard keeps HIR honest if a
        // malformed AST ever tries to bind a loop name over an already-visible local.
        if self.locals_by_name.contains_key(&binding.id)
            || visible_bindings.iter().any(|(path, _)| path == &binding.id)
        {
            return Err(InfrastructureFailure::hir_transformation(format!(
                "Local '{}' is already declared in this function scope",
                self.symbol_name_for_diagnostics(&binding.id)
            ))
            .into());
        }

        let region = self.current_region_or_error()?;
        let block_id = self.current_block_id_or_error()?;
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
        self.register_local_in_block(block_id, local)?;
        self.side_table.bind_local_name(local_id, binding.id);
        self.side_table
            .bind_local_origin(local_id, HirLocalOriginKind::User, None, None);
        self.side_table
            .map_ast_to_hir(*span_ref, HirLocation::Local(local_id));

        // Authored binding materialization carries the explicit binding span.
        // Jumps and numeric step updates elsewhere stay spanless as generated
        // scaffolding.
        self.emit_statement_kind_with_span(
            HirStatementKind::Write {
                target: HirWriteTarget::DefineLocal(local_id),
                value,
            },
            span_ref,
            binding.binding_span,
        )?;

        Ok((binding.id, local_id))
    }

    fn collection_iteration_types(
        &mut self,
        iterable: ExpressionView<'_>,
    ) -> Result<(TypeId, TypeId), HirConstructionFailure> {
        // Frontend type resolution canonicalizes reference wrappers, so collection loops
        // accept either direct collections or shared references to collections uniformly.
        let iterable_type = self.lower_type_id(iterable.value_type_id()?)?;
        let element = match self.type_environment.collection_element_type(iterable_type) {
            Some(element) => element,
            None => {
                return Err(InfrastructureFailure::hir_transformation(
                    "Collection loop iterable did not lower to a collection HIR type",
                )
                .into());
            }
        };

        Ok((iterable_type, element))
    }
}
