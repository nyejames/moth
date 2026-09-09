//! Short-circuit CFG generation for `and` / `or` operators.
//!
//! WHAT: emits conditional branches, short-circuit blocks, and merge blocks so RHS
//!       side effects are only evaluated when the operator semantics require them.
//! WHY: without explicit short-circuit CFG, AST RPN trees would eagerly evaluate
//!      both sides, breaking lazy boolean semantics and causing invalid borrow access.

use crate::compiler_frontend::ast::expressions::expression::Operator;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::hir_builder::HirBuilder;
use crate::compiler_frontend::hir::ids::BlockId;
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::source::SourceSpan;
use crate::return_hir_transformation_error;

use super::super::LoweredExpression;
use super::RuntimeRpnTree;

#[derive(Debug, Clone, Copy)]
struct ShortCircuitCfgSpec {
    evaluate_rhs_on_true: bool,
    short_value: bool,
    rhs_block_label: &'static str,
    short_block_label: &'static str,
    merge_block_label: &'static str,
    rhs_edge_label: &'static str,
    short_edge_label: &'static str,
}

impl ShortCircuitCfgSpec {
    fn branch_targets(self, rhs_block: BlockId, short_block: BlockId) -> (BlockId, BlockId) {
        if self.evaluate_rhs_on_true {
            (rhs_block, short_block)
        } else {
            (short_block, rhs_block)
        }
    }
}

impl<'a> HirBuilder<'a> {
    pub(super) fn lower_short_circuit_binary_expression(
        &mut self,
        left: &RuntimeRpnTree,
        op: &Operator,
        right: &RuntimeRpnTree,
        source_span: &Option<SourceSpan>,
    ) -> Result<LoweredExpression, CompilerError> {
        let lowered_left = self.lower_runtime_tree_value_to_current_block(left, source_span)?;

        let condition_block = self.current_block_id_or_error(source_span)?;
        let parent_region = self.current_region_or_error(source_span)?;
        let bool_ty = builtin_type_ids::BOOL;
        let cfg_spec = self.short_circuit_cfg_spec(op, source_span)?;

        let rhs_region = self.create_child_region(parent_region);
        let short_region = self.create_child_region(parent_region);
        let rhs_block = self.create_block(rhs_region, source_span, cfg_spec.rhs_block_label)?;
        let short_block =
            self.create_block(short_region, source_span, cfg_spec.short_block_label)?;
        let merge_block =
            self.create_block(parent_region, source_span, cfg_spec.merge_block_label)?;
        self.set_current_block(merge_block, source_span)?;
        let result_local = self.allocate_temp_local(bool_ty, None)?;
        self.set_current_block(condition_block, source_span)?;
        let (then_block, else_block) = cfg_spec.branch_targets(rhs_block, short_block);

        self.emit_terminator_with_span(
            condition_block,
            HirTerminator::If {
                condition: lowered_left,
                then_block,
                else_block,
            },
            source_span,
            *source_span,
        )?;

        self.emit_short_circuit_rhs_branch(
            rhs_block,
            merge_block,
            right,
            source_span,
            cfg_spec.rhs_edge_label,
        )?;
        self.emit_short_circuit_constant_branch(
            (short_block, merge_block),
            cfg_spec.short_value,
            bool_ty,
            source_span,
            cfg_spec.short_edge_label,
        )?;

        self.set_current_block(merge_block, source_span)?;
        let merge_region = self.current_region_or_error(source_span)?;
        let value = self.make_local_load_expression(result_local, bool_ty, &None, merge_region);

        Ok(LoweredExpression {
            prelude: vec![],
            value,
        })
    }

    fn short_circuit_cfg_spec(
        &self,
        op: &Operator,
        source_span: &Option<SourceSpan>,
    ) -> Result<ShortCircuitCfgSpec, CompilerError> {
        match op {
            Operator::And => Ok(ShortCircuitCfgSpec {
                evaluate_rhs_on_true: true,
                short_value: false,
                rhs_block_label: "logical-and-rhs",
                short_block_label: "logical-and-short",
                merge_block_label: "logical-and-merge",
                rhs_edge_label: "logical.and.rhs",
                short_edge_label: "logical.and.short",
            }),
            Operator::Or => Ok(ShortCircuitCfgSpec {
                evaluate_rhs_on_true: false,
                short_value: true,
                rhs_block_label: "logical-or-rhs",
                short_block_label: "logical-or-short",
                merge_block_label: "logical-or-merge",
                rhs_edge_label: "logical.or.rhs",
                short_edge_label: "logical.or.short",
            }),
            _ => {
                return_hir_transformation_error!(
                    format!(
                        "Short-circuit CFG requested for non-logical operator {:?}",
                        op
                    ),
                    self.hir_error_location(source_span)
                )
            }
        }
    }

    fn emit_short_circuit_rhs_branch(
        &mut self,
        rhs_block: BlockId,
        merge_block: BlockId,
        rhs: &RuntimeRpnTree,
        source_span: &Option<SourceSpan>,
        edge_label: &str,
    ) -> Result<(), CompilerError> {
        self.set_current_block(rhs_block, source_span)?;

        let lowered_rhs = self.lower_runtime_tree_value_to_current_block(rhs, source_span)?;
        let merge_arg_local =
            self.materialize_short_circuit_jump_argument_local(lowered_rhs, source_span)?;

        let rhs_tail = self.current_block_id_or_error(source_span)?;
        if !self.block_has_explicit_terminator(rhs_tail, source_span)? {
            self.emit_jump_with_args(
                rhs_tail,
                merge_block,
                vec![merge_arg_local],
                source_span,
                edge_label,
            )?;
        }

        Ok(())
    }

    fn emit_short_circuit_constant_branch(
        &mut self,
        branch_blocks: (BlockId, BlockId),
        short_value: bool,
        bool_ty: TypeId,
        source_span: &Option<SourceSpan>,
        edge_label: &str,
    ) -> Result<(), CompilerError> {
        let (short_block, merge_block) = branch_blocks;
        self.set_current_block(short_block, source_span)?;
        let short_region = self.current_region_or_error(source_span)?;
        let short_value_expression = self.make_expression(
            &None,
            HirExpressionKind::Bool(short_value),
            bool_ty,
            ValueKind::Const,
            short_region,
        );
        let merge_arg_local = self
            .materialize_short_circuit_jump_argument_local(short_value_expression, source_span)?;
        self.emit_jump_with_args(
            short_block,
            merge_block,
            vec![merge_arg_local],
            source_span,
            edge_label,
        )
    }
}
