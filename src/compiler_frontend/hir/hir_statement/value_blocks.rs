//! Value-producing block lowering for HIR statements.
//!
//! WHAT: lowers AST `ThenValue` nodes and value-producing if/match expressions
//! into explicit CFG blocks, result locals, and merge-block loads.
//! WHY: value blocks share a result-local/merge-target protocol that must remain
//! consistent for borrow validation and backend lowering.

use crate::compiler_frontend::ast::ast_nodes::SourceLocation;
use crate::compiler_frontend::ast::expressions::expression::ExpressionKind;
use crate::compiler_frontend::ast::statements::value_production::{
    ProducedValues,
    types::{ValueIfBlock, ValueMatchBlock, ValueScopedBlock},
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::hir::expressions::{HirExpression, HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::hir_builder::{HirBuilder, ValueBlockTarget};
use crate::compiler_frontend::hir::hir_expression::LoweredExpression;
use crate::compiler_frontend::hir::ids::{LocalId, RegionId};
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::statements::HirStatementKind;
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::return_hir_transformation_error;

impl<'a> HirBuilder<'a> {
    // -------------------------
    //  ThenValue lowering
    // -------------------------

    /// Lowers a `ThenValue` statement by assigning produced expressions to the active
    /// value-block result locals and jumping to the merge block.
    ///
    /// WHAT: intercepts `then` inside value-producing control flow and wires it to the
    ///       shared result locals allocated by the enclosing value block.
    /// WHY: `ThenValue` is valid only inside value-producing if/match/catch; without an
    ///      active target it represents a HIR lowering invariant failure.
    pub(super) fn lower_then_value_statement(
        &mut self,
        produced_values: &ProducedValues,
        location: &SourceLocation,
    ) -> Result<(), CompilerError> {
        let maybe_target = self.active_value_block_target.clone();
        if let Some(target) = maybe_target {
            if produced_values.expressions.len() != target.result_locals.len() {
                return_hir_transformation_error!(
                    format!(
                        "ThenValue produced {} expressions but active target expects {} locals",
                        produced_values.expressions.len(),
                        target.result_locals.len()
                    ),
                    self.hir_error_location(location)
                );
            }

            for (expr, result_local) in produced_values
                .expressions
                .iter()
                .zip(target.result_locals.iter())
            {
                let value = self.lower_expression_value_to_current_block(expr)?;
                let value = self.materialize_value_block_result(value, location);
                self.emit_statement_kind(
                    HirStatementKind::Assign {
                        target: HirPlace::Local(*result_local),
                        value,
                    },
                    location,
                )?;
            }

            let current_block = self.current_block_id_or_error(location)?;
            self.emit_terminator(
                current_block,
                HirTerminator::Jump {
                    target: target.merge_block,
                    args: vec![],
                },
                location,
            )?;
            Ok(())
        } else {
            return_hir_transformation_error!(
                "ThenValue encountered without active value block target",
                self.hir_error_location(location)
            )
        }
    }

    /// Convert produced `then` places into plain values before assigning result locals.
    ///
    /// WHAT: value blocks produce values for closed receivers, not alias views. A `then name`
    /// branch should therefore materialize the current value of `name` into the hidden result
    /// local rather than making that result local borrow `name`.
    /// WHY: preserving branch-local aliases makes value-match merges path-dependent (`then name`
    /// aliases while `else "guest"` owns), which is both surprising at the language level and
    /// invalid for the borrow checker join model.
    fn materialize_value_block_result(
        &mut self,
        value: HirExpression,
        location: &SourceLocation,
    ) -> HirExpression {
        match value.kind {
            HirExpressionKind::Load(place) => self.make_expression(
                location,
                HirExpressionKind::Copy(place),
                value.ty,
                ValueKind::RValue,
                value.region,
            ),
            _ => value,
        }
    }

    // -------------------------
    //  Result-local allocation
    // -------------------------

    /// Allocates one hidden result local per expected value-block slot.
    ///
    /// WHAT: creates temporaries that every producing branch will assign to.
    /// WHY: single-result blocks use one local; multi-result blocks use N locals that
    ///      are later folded into an internal `TupleConstruct`.
    fn allocate_value_block_result_locals(
        &mut self,
        result_type_ids: &[TypeId],
        location: &SourceLocation,
    ) -> Result<Vec<LocalId>, CompilerError> {
        let mut result_locals = Vec::with_capacity(result_type_ids.len());
        for type_id in result_type_ids {
            let lowered_ty = self.lower_type_id(*type_id, location)?;
            let local = self.allocate_temp_local(lowered_ty, Some(location.to_owned()))?;
            result_locals.push(local);
        }
        Ok(result_locals)
    }

    // -------------------------
    //  Value-if lowering
    // -------------------------

    /// Lowers a value-producing `if` expression into explicit CFG blocks.
    ///
    /// WHAT: allocates result locals (one per expected slot), creates then/else/merge blocks,
    ///       lowers the condition and both branches, and returns a `Load` or `TupleConstruct`
    ///       of the result locals as the expression value.
    /// WHY: value-producing `if` has no dedicated HIR expression kind; it is represented as
    ///      `HirTerminator::If` with branch bodies that assign to shared locals and jump to merge.
    pub(crate) fn lower_value_block_if(
        &mut self,
        value_if: &ValueIfBlock,
        location: &SourceLocation,
        _result_type_id: TypeId,
    ) -> Result<LoweredExpression, CompilerError> {
        if matches!(value_if.condition.kind, ExpressionKind::Bool(_)) {
            return_hir_transformation_error!(
                "Stage 4 passed a statically decided value-producing Bool `if` to HIR",
                self.hir_error_location(location)
            );
        }
        let parent_region = self.current_region_or_error(location)?;

        let result_locals =
            self.allocate_value_block_result_locals(&value_if.result_type_ids, location)?;
        let merge_block = self.create_block(parent_region, location, "value-if-merge")?;

        let condition_value = self.lower_expression_value_to_current_block(&value_if.condition)?;
        let condition_block = self.current_block_id_or_error(location)?;

        let then_region = self.create_child_region(parent_region);
        let else_region = self.create_child_region(parent_region);
        let then_block = self.create_block(then_region, location, "value-if-then")?;
        let else_block = self.create_block(else_region, location, "value-if-else")?;

        self.emit_terminator(
            condition_block,
            HirTerminator::If {
                condition: condition_value,
                then_block,
                else_block,
            },
            location,
        )?;

        self.log_control_flow_edge(condition_block, then_block, "value-if.true");
        self.log_control_flow_edge(condition_block, else_block, "value-if.false");

        self.set_current_block(then_block, location)?;
        self.with_active_value_block_target(
            ValueBlockTarget {
                result_locals: result_locals.clone(),
                merge_block,
            },
            |builder| builder.lower_statement_sequence(&value_if.then_body),
        )?;

        let then_tail_block = self.current_block_id_or_error(location)?;
        let then_terminated = self.block_has_explicit_terminator(then_tail_block, location)?;
        if !then_terminated {
            self.emit_jump_to(
                then_tail_block,
                merge_block,
                location,
                "value-if.then.merge",
            )?;
        }

        self.set_current_block(else_block, location)?;
        self.with_active_value_block_target(
            ValueBlockTarget {
                result_locals: result_locals.clone(),
                merge_block,
            },
            |builder| builder.lower_statement_sequence(&value_if.else_body),
        )?;

        let else_tail_block = self.current_block_id_or_error(location)?;
        let else_terminated = self.block_has_explicit_terminator(else_tail_block, location)?;
        if !else_terminated {
            self.emit_jump_to(
                else_tail_block,
                merge_block,
                location,
                "value-if.else.merge",
            )?;
        }

        self.set_current_block(merge_block, location)?;

        let value = self.value_block_result_expression(
            &result_locals,
            &value_if.result_type_ids,
            location,
            parent_region,
        )?;

        Ok(LoweredExpression {
            prelude: vec![],
            value,
        })
    }

    /// Lowers one statically selected value-producing body without rebuilding a Bool branch.
    pub(crate) fn lower_value_block_scoped(
        &mut self,
        value_scoped: &ValueScopedBlock,
        location: &SourceLocation,
        _result_type_id: TypeId,
    ) -> Result<LoweredExpression, CompilerError> {
        let entry_block = self.current_block_id_or_error(location)?;
        let parent_region = self.current_region_or_error(location)?;
        let body_region = self.create_child_region(parent_region);
        let body_block = self.create_block(body_region, location, "static-value-if-body")?;
        let merge_block = self.create_block(parent_region, location, "static-value-if-merge")?;
        let result_locals =
            self.allocate_value_block_result_locals(&value_scoped.result_type_ids, location)?;

        self.emit_jump_to(entry_block, body_block, location, "static-value-if.enter")?;
        self.set_current_block(body_block, location)?;
        self.with_active_value_block_target(
            ValueBlockTarget {
                result_locals: result_locals.clone(),
                merge_block,
            },
            |builder| builder.lower_statement_sequence(&value_scoped.body),
        )?;

        let body_tail = self.current_block_id_or_error(location)?;
        if !self.block_has_explicit_terminator(body_tail, location)? {
            self.emit_jump_to(body_tail, merge_block, location, "static-value-if.exit")?;
        }
        self.set_current_block(merge_block, location)?;

        let value = self.value_block_result_expression(
            &result_locals,
            &value_scoped.result_type_ids,
            location,
            parent_region,
        )?;

        Ok(LoweredExpression {
            prelude: vec![],
            value,
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
        value_match: &ValueMatchBlock,
        location: &SourceLocation,
        _result_type_id: TypeId,
    ) -> Result<LoweredExpression, CompilerError> {
        let parent_region = self.current_region_or_error(location)?;

        let result_locals =
            self.allocate_value_block_result_locals(&value_match.result_type_ids, location)?;
        let merge_block = self.create_block(parent_region, location, "value-match-merge")?;

        self.with_active_value_block_target(
            ValueBlockTarget {
                result_locals: result_locals.clone(),
                merge_block,
            },
            |builder| {
                builder.lower_match_statement(
                    &value_match.scrutinee,
                    &value_match.arms,
                    value_match.default.as_deref(),
                    value_match.exhaustiveness,
                    location,
                )
            },
        )?;

        self.set_current_block(merge_block, location)?;

        let value = self.value_block_result_expression(
            &result_locals,
            &value_match.result_type_ids,
            location,
            parent_region,
        )?;

        Ok(LoweredExpression {
            prelude: vec![],
            value,
        })
    }

    // -------------------------
    //  Result expression
    // -------------------------

    /// Builds the expression that represents the value of a completed value block.
    ///
    /// WHAT: for a single result returns a `Load` of the result local; for multi-result
    ///       returns an internal `TupleConstruct` of the result-local loads.
    /// WHY: value blocks are only accepted at closed receiving sites; the tuple is an
    ///      internal HIR shape, not user-visible tuple syntax.
    pub(crate) fn value_block_result_expression(
        &mut self,
        result_locals: &[LocalId],
        result_type_ids: &[TypeId],
        location: &SourceLocation,
        parent_region: RegionId,
    ) -> Result<HirExpression, CompilerError> {
        if result_locals.len() == 1 {
            let result_ty = self.lower_type_id(result_type_ids[0], location)?;
            return Ok(self.make_expression(
                location,
                HirExpressionKind::Load(HirPlace::Local(result_locals[0])),
                result_ty,
                ValueKind::RValue,
                parent_region,
            ));
        }

        let mut elements = Vec::with_capacity(result_locals.len());
        let mut field_types = Vec::with_capacity(result_locals.len());
        for (local, ast_type_id) in result_locals.iter().zip(result_type_ids.iter()) {
            let ty = self.lower_type_id(*ast_type_id, location)?;
            field_types.push(ty);
            let element = self.make_local_load_expression(*local, ty, location, parent_region);
            elements.push(element);
        }
        let tuple_type = self.type_environment.intern_tuple(field_types);
        Ok(self.make_expression(
            location,
            HirExpressionKind::TupleConstruct { elements },
            tuple_type,
            ValueKind::RValue,
            parent_region,
        ))
    }
}
