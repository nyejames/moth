//! Value-producing block lowering for HIR statements.
//!
//! WHAT: lowers AST `ThenValue` nodes and value-producing if/match expressions
//! into explicit CFG blocks, result locals, and merge-block loads.
//! WHY: value blocks share a result-local/merge-target protocol that must remain
//! consistent for borrow validation and backend lowering.

use crate::compiler_frontend::ast::expressions::expression_store::ExpressionStore;
use crate::compiler_frontend::ast::statements::value_production::{
    ProducedValues,
    types::{ValueIfBlock, ValueLexicalScope, ValueMatchBlock},
};
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::hir::expression_store::HirConstructionFailure;
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::hir_builder::{HirBuilder, ValueBlockTarget};
use crate::compiler_frontend::hir::hir_expression::LoweredResults;
use crate::compiler_frontend::hir::ids::{BlockId, HirValueId, LocalId, RegionId};
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::statements::{HirStatementKind, HirWriteTarget};
use crate::compiler_frontend::hir::terminators::{HirJumpArgument, HirTerminator};
use crate::compiler_frontend::source::SourceSpan;
use crate::return_hir_transformation_error;

impl<'a> HirBuilder<'a> {
    // -------------------------
    //  ThenValue lowering
    // -------------------------

    /// Lowers a `ThenValue` statement by defining produced expressions in the active
    /// value-block result locals and jumping to the merge block.
    ///
    /// WHAT: intercepts `then` inside value-producing control flow and wires it to the
    ///       shared result locals allocated by the enclosing value block.
    /// WHY: `ThenValue` is valid only inside value-producing if/match/catch; without an
    ///      active target it represents a HIR lowering invariant failure.
    pub(super) fn lower_then_value_statement(
        &mut self,
        expressions: &ExpressionStore,
        produced_values: &ProducedValues,
        span: &Option<SourceSpan>,
        statement_span: Option<SourceSpan>,
    ) -> Result<(), HirConstructionFailure> {
        let maybe_target = self.active_value_block_target.clone();
        if let Some(target) = maybe_target {
            let mut args = Vec::with_capacity(target.result_locals.len());
            // Freeze each expression's reads before the next expression can mutate their places.
            for expression in &produced_values.expressions {
                let values =
                    self.lower_expression_results_to_current_block(expressions.view(*expression)?)?;
                for value in values {
                    let Some(destination) = target.result_locals.get(args.len()).copied() else {
                        return_hir_transformation_error!(
                            "ThenValue result count exceeds active value-block target",
                            self.hir_error_location(span)
                        );
                    };
                    let expected_type = self.local_type_id_or_error(destination, span)?;
                    let value = self.wrap_value_to_optional_type(value, expected_type, span)?;
                    let value = self.materialize_value_block_result(value)?;
                    let source = self.allocate_temp_local(expected_type, None)?;
                    let value_span = self
                        .module
                        .expressions
                        .expression(value)
                        .span
                        .or(statement_span);
                    self.emit_statement_kind_with_span(
                        HirStatementKind::Write {
                            target: HirWriteTarget::DefineLocal(source),
                            value,
                        },
                        span,
                        value_span,
                    )?;
                    args.push(HirJumpArgument {
                        source,
                        destination,
                    });
                }
            }
            if args.len() != target.result_locals.len() {
                return_hir_transformation_error!(
                    "ThenValue result count does not match active value-block target",
                    self.hir_error_location(span)
                );
            }

            let current_block = self.current_block_id_or_error(span)?;
            self.emit_terminator(
                current_block,
                HirTerminator::Jump {
                    target: target.merge_block,
                    args,
                },
                span,
            )?;
            Ok(())
        } else {
            return_hir_transformation_error!(
                "ThenValue encountered without active value block target",
                self.hir_error_location(span)
            )
        }
    }

    /// Convert produced places into values before defining result locals.
    ///
    /// WHAT: a `Place` row names a binding cell, while the value-block result is a newly produced
    ///       value that may retain the same allocation provenance.
    /// WHY: the merge local must not become an alias of the source binding merely because the
    ///      produced value was read from that place.
    fn materialize_value_block_result(
        &mut self,
        value: HirValueId,
    ) -> Result<HirValueId, HirConstructionFailure> {
        let (place, span, ty, region) = {
            let row = self.module.expressions.expression(value);
            let (HirExpressionKind::Load(place), ValueKind::Place) = (&row.kind, row.value_kind)
            else {
                return Ok(value);
            };
            (*place, row.span, row.ty, row.region)
        };
        self.make_expression(
            &span,
            HirExpressionKind::Load(place),
            ty,
            ValueKind::RValue,
            region,
        )
    }

    // -------------------------
    //  Result-local allocation
    // -------------------------

    /// Allocates one hidden result local per expected value-block slot.
    ///
    /// WHAT: creates one destination that each producing edge defines by Jump argument.
    /// WHY: joins retain slot identity without creating a semantic aggregate.
    fn allocate_value_block_result_locals(
        &mut self,
        result_type_ids: &[TypeId],
        merge_block: BlockId,
        span: &Option<SourceSpan>,
    ) -> Result<Vec<LocalId>, HirConstructionFailure> {
        let source_block = self.current_block_id_or_error(span)?;
        self.set_current_block(merge_block, span)?;
        let mut result_locals = Vec::with_capacity(result_type_ids.len());
        for type_id in result_type_ids {
            let lowered_ty = self.lower_type_id(*type_id, span)?;
            let local = self.allocate_temp_local(lowered_ty, None)?;
            result_locals.push(local);
        }
        self.set_current_block(source_block, span)?;
        Ok(result_locals)
    }

    // -------------------------
    //  Value-if lowering
    // -------------------------

    /// Lowers a value-producing `if` expression into explicit CFG blocks.
    ///
    /// WHAT: allocates per-slot destinations and lowers both branches into a shared merge.
    /// WHY: ordinary Jump arguments define the joined values on each producing edge.
    pub(crate) fn lower_value_block_if(
        &mut self,
        expressions: &ExpressionStore,
        value_if: &ValueIfBlock,
        span: &Option<SourceSpan>,
        result_type_ids: &[TypeId],
    ) -> Result<LoweredResults, HirConstructionFailure> {
        if self
            .expression_constant_bool(expressions.view(value_if.condition)?)
            .is_some()
        {
            return_hir_transformation_error!(
                "Stage 4 passed a statically decided value-producing Bool `if` to HIR",
                self.hir_error_location(span)
            );
        }
        let parent_region = self.current_region_or_error(span)?;

        let merge_block = self.create_block(parent_region, span, "value-if-merge")?;
        let result_locals =
            self.allocate_value_block_result_locals(result_type_ids, merge_block, span)?;

        let condition_value =
            self.lower_expression_value_to_current_block(expressions.view(value_if.condition)?)?;
        let condition_block = self.current_block_id_or_error(span)?;

        let then_region = self.create_child_region(parent_region);
        let else_region = self.create_child_region(parent_region);
        let then_block = self.create_block(then_region, span, "value-if-then")?;
        let else_block = self.create_block(else_region, span, "value-if-else")?;

        // Authored value-if header: the condition branch carries the authored
        // condition span. Merge jumps below are generated CFG and stay spanless.
        self.emit_terminator_with_span(
            condition_block,
            HirTerminator::If {
                condition: condition_value,
                then_block,
                else_block,
            },
            span,
            expressions.view(value_if.condition)?.span(),
        )?;

        self.log_control_flow_edge(condition_block, then_block, "value-if.true");
        self.log_control_flow_edge(condition_block, else_block, "value-if.false");

        self.set_current_block(then_block, span)?;
        self.with_active_value_block_target(
            ValueBlockTarget {
                result_locals: result_locals.clone(),
                merge_block,
            },
            |builder| builder.lower_statement_sequence(expressions, &value_if.then_body),
        )?;

        let then_tail_block = self.current_block_id_or_error(span)?;
        let then_terminated = self.block_has_explicit_terminator(then_tail_block, span)?;
        if !then_terminated {
            self.emit_jump_to(then_tail_block, merge_block, span, "value-if.then.merge")?;
        }

        self.set_current_block(else_block, span)?;
        self.with_active_value_block_target(
            ValueBlockTarget {
                result_locals: result_locals.clone(),
                merge_block,
            },
            |builder| builder.lower_statement_sequence(expressions, &value_if.else_body),
        )?;

        let else_tail_block = self.current_block_id_or_error(span)?;
        let else_terminated = self.block_has_explicit_terminator(else_tail_block, span)?;
        if !else_terminated {
            self.emit_jump_to(else_tail_block, merge_block, span, "value-if.else.merge")?;
        }

        self.set_current_block(merge_block, span)?;

        let values = self.value_block_result_expressions(
            &result_locals,
            result_type_ids,
            span,
            parent_region,
        )?;

        Ok(LoweredResults {
            prelude: vec![],
            values,
        })
    }

    /// Lowers one statically selected value-producing body without rebuilding a Bool branch.
    pub(crate) fn lower_value_lexical_scope(
        &mut self,
        expressions: &ExpressionStore,
        value_lexical_scope: &ValueLexicalScope,
        span: &Option<SourceSpan>,
        result_type_ids: &[TypeId],
    ) -> Result<LoweredResults, HirConstructionFailure> {
        let entry_block = self.current_block_id_or_error(span)?;
        let parent_region = self.current_region_or_error(span)?;
        let body_region = self.create_child_region(parent_region);
        let body_block = self.create_block(body_region, span, "static-value-if-body")?;
        let merge_block = self.create_block(parent_region, span, "static-value-if-merge")?;
        let result_locals =
            self.allocate_value_block_result_locals(result_type_ids, merge_block, span)?;

        self.emit_jump_to(entry_block, body_block, span, "static-value-if.enter")?;
        self.set_current_block(body_block, span)?;
        self.with_active_value_block_target(
            ValueBlockTarget {
                result_locals: result_locals.clone(),
                merge_block,
            },
            |builder| builder.lower_statement_sequence(expressions, &value_lexical_scope.body),
        )?;

        let body_tail = self.current_block_id_or_error(span)?;
        if !self.block_has_explicit_terminator(body_tail, span)? {
            self.emit_jump_to(body_tail, merge_block, span, "static-value-if.exit")?;
        }
        self.set_current_block(merge_block, span)?;

        let values = self.value_block_result_expressions(
            &result_locals,
            result_type_ids,
            span,
            parent_region,
        )?;

        Ok(LoweredResults {
            prelude: vec![],
            values,
        })
    }

    // -------------------------
    //  Value-match lowering
    // -------------------------

    /// Lowers a value-producing full match by reusing statement match CFG lowering.
    ///
    /// WHAT: allocates result locals, lowers the match with `ThenValue` targeting
    /// those locals, then resumes at the value-block merge.
    /// WHY: pattern dispatch, guards, captures, defaults, and no-match runtime failures are
    /// already owned by statement match lowering; the value form only changes what
    /// `then` does inside each arm body.
    pub(crate) fn lower_value_block_match(
        &mut self,
        expressions: &ExpressionStore,
        value_match: &ValueMatchBlock,
        span: &Option<SourceSpan>,
        result_type_ids: &[TypeId],
    ) -> Result<LoweredResults, HirConstructionFailure> {
        let parent_region = self.current_region_or_error(span)?;

        let merge_block = self.create_block(parent_region, span, "value-match-merge")?;
        let result_locals =
            self.allocate_value_block_result_locals(result_type_ids, merge_block, span)?;

        self.with_active_value_block_target(
            ValueBlockTarget {
                result_locals: result_locals.clone(),
                merge_block,
            },
            |builder| {
                // Authored value-match dispatch carries the scrutinee span; the
                // merge resume below is generated CFG and stays spanless.
                builder.lower_match_statement(
                    expressions,
                    expressions.view(value_match.scrutinee)?,
                    &value_match.arms,
                    value_match.default.as_deref(),
                    value_match.exhaustiveness,
                    span,
                    expressions.view(value_match.scrutinee)?.span(),
                )
            },
        )?;

        self.set_current_block(merge_block, span)?;

        let values = self.value_block_result_expressions(
            &result_locals,
            result_type_ids,
            span,
            parent_region,
        )?;

        Ok(LoweredResults {
            prelude: vec![],
            values,
        })
    }

    // -------------------------
    //  Result expression
    // -------------------------

    /// Loads each joined result as a fresh produced value, preserving allocation aliases.
    pub(crate) fn value_block_result_expressions(
        &mut self,
        result_locals: &[LocalId],
        result_type_ids: &[TypeId],
        span: &Option<SourceSpan>,
        parent_region: RegionId,
    ) -> Result<Vec<HirValueId>, HirConstructionFailure> {
        if result_locals.len() != result_type_ids.len() {
            return_hir_transformation_error!(
                "Value-block destination count does not match result types",
                self.hir_error_location(span)
            );
        }
        let mut values = Vec::with_capacity(result_locals.len());
        for (local, ast_type_id) in result_locals.iter().zip(result_type_ids) {
            let ty = self.lower_type_id(*ast_type_id, span)?;
            values.push(self.make_expression(
                span,
                HirExpressionKind::Load(HirPlace::local(*local)),
                ty,
                ValueKind::RValue,
                parent_region,
            )?);
        }
        Ok(values)
    }
}
