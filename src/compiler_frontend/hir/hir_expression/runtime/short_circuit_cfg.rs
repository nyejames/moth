//! Short-circuit CFG generation for `and` / `or` operators.
//!
//! WHAT: emits conditional branches, short-circuit blocks and merge blocks so RHS / side effects
//!       are only evaluated when the operator semantics require them.
//! WHY: explicit branch ownership keeps right-hand effects and failure continuations lazy.
//!
//! The operator walk in `hir_expression/expression_walk.rs` lowers the left operand and calls
//! `begin_short_circuit_binary`, lowers the right operand into the RHS block, then calls
//! `finish_short_circuit_binary`.

use crate::compiler_frontend::ast::expressions::expression_store::LogicalOperator;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::hir::expression_store::HirConstructionFailure;
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::hir_builder::HirBuilder;
use crate::compiler_frontend::hir::ids::{BlockId, HirValueId, LocalId};
use crate::compiler_frontend::hir::terminators::{HirJumpArgument, HirTerminator};
use crate::compiler_frontend::source::SourceSpan;

/// Static shape of one short-circuit operator: which side evaluates the RHS and what the
/// short-circuit path produces.
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
    /// Returns the `(then, else)` targets for the condition block.
    fn branch_targets(self, rhs_block: BlockId, short_block: BlockId) -> (BlockId, BlockId) {
        if self.evaluate_rhs_on_true {
            (rhs_block, short_block)
        } else {
            (short_block, rhs_block)
        }
    }
}

static LOGICAL_AND_CFG: ShortCircuitCfgSpec = ShortCircuitCfgSpec {
    evaluate_rhs_on_true: true,
    short_value: false,
    rhs_block_label: "logical-and-rhs",
    short_block_label: "logical-and-short",
    merge_block_label: "logical-and-merge",
    rhs_edge_label: "logical.and.rhs",
    short_edge_label: "logical.and.short",
};

static LOGICAL_OR_CFG: ShortCircuitCfgSpec = ShortCircuitCfgSpec {
    evaluate_rhs_on_true: false,
    short_value: true,
    rhs_block_label: "logical-or-rhs",
    short_block_label: "logical-or-short",
    merge_block_label: "logical-or-merge",
    rhs_edge_label: "logical.or.rhs",
    short_edge_label: "logical.or.short",
};

fn short_circuit_cfg_spec(operation: LogicalOperator) -> &'static ShortCircuitCfgSpec {
    match operation {
        LogicalOperator::And => &LOGICAL_AND_CFG,
        LogicalOperator::Or => &LOGICAL_OR_CFG,
    }
}

/// Blocks and locals created by `begin_short_circuit_binary`, consumed by
/// `finish_short_circuit_binary`.
#[derive(Clone, Copy)]
pub(crate) struct ShortCircuitPlan {
    spec: &'static ShortCircuitCfgSpec,
    short_block: BlockId,
    merge_block: BlockId,
    result_local: LocalId,
    bool_type_id: TypeId,
}

impl<'a> HirBuilder<'a> {
    /// Branches on the lowered left operand and leaves the builder in the RHS block.
    pub(crate) fn begin_short_circuit_binary(
        &mut self,
        lowered_left: HirValueId,
        operation: LogicalOperator,
        result_type: TypeId,
        source_span: &Option<SourceSpan>,
    ) -> Result<ShortCircuitPlan, HirConstructionFailure> {
        let condition_block = self.current_block_id_or_error()?;
        let parent_region = self.current_region_or_error()?;
        let bool_type_id = self.lower_type_id(result_type)?;
        let spec = short_circuit_cfg_spec(operation);

        let rhs_region = self.create_child_region(parent_region);
        let short_region = self.create_child_region(parent_region);
        let rhs_block = self.create_block(rhs_region, source_span, spec.rhs_block_label);
        let short_block = self.create_block(short_region, source_span, spec.short_block_label);
        let merge_block = self.create_block(parent_region, source_span, spec.merge_block_label);

        self.set_current_block(merge_block)?;
        let result_local = self.allocate_temp_local(bool_type_id, None)?;

        self.set_current_block(condition_block)?;
        let (then_block, else_block) = spec.branch_targets(rhs_block, short_block);
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

        self.set_current_block(rhs_block)?;
        Ok(ShortCircuitPlan {
            spec,
            short_block,
            merge_block,
            result_local,
            bool_type_id,
        })
    }

    /// Joins the lowered RHS and the short-circuit path into the merge block.
    ///
    /// Leaves the builder in the merge block and returns a load of the merged result.
    pub(crate) fn finish_short_circuit_binary(
        &mut self,
        plan: ShortCircuitPlan,
        lowered_rhs: HirValueId,
        source_span: &Option<SourceSpan>,
    ) -> Result<HirValueId, HirConstructionFailure> {
        let ShortCircuitPlan {
            spec,
            short_block,
            merge_block,
            result_local,
            bool_type_id,
        } = plan;

        let rhs_merge_argument = self.materialize_short_circuit_jump_argument_local(lowered_rhs)?;
        let rhs_tail = self.current_block_id_or_error()?;
        if !self.block_has_explicit_terminator(rhs_tail)? {
            self.emit_jump_with_args(
                rhs_tail,
                merge_block,
                vec![HirJumpArgument {
                    source: rhs_merge_argument,
                    destination: result_local,
                }],
                source_span,
                spec.rhs_edge_label,
            )?;
        }

        self.set_current_block(short_block)?;
        let short_region = self.current_region_or_error()?;
        let short_value = self.make_expression(
            &None,
            HirExpressionKind::Bool(spec.short_value),
            bool_type_id,
            ValueKind::Const,
            short_region,
        )?;
        let short_merge_argument =
            self.materialize_short_circuit_jump_argument_local(short_value)?;
        self.emit_jump_with_args(
            short_block,
            merge_block,
            vec![HirJumpArgument {
                source: short_merge_argument,
                destination: result_local,
            }],
            source_span,
            spec.short_edge_label,
        )?;

        self.set_current_block(merge_block)?;
        let merge_region = self.current_region_or_error()?;
        self.make_local_load_expression(result_local, bool_type_id, &None, merge_region)
    }
}
