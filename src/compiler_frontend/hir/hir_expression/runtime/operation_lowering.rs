//! Per-operation HIR emission for resolved numeric, logical, comparison and range operations.
//!
//! WHAT: holds the entry validation and the emission step for each operator. The operator walk in
//!       `hir_expression/expression_walk.rs` owns child traversal and calls these steps once the
//!       operands have been delivered.
//! WHY: separating validation and emission from traversal keeps operation semantics in one place
//!      while the walk stays iterative. Child views stay bound to the AST owner, and shared
//!      semantic nodes are not memoised.

use crate::compiler_frontend::ast::expressions::expression_kind::Operator;
use crate::compiler_frontend::ast::expressions::expression_rpn::{
    ResolvedNumericOperation, ResolvedOperation, ResolvedOperationKind,
};
use crate::compiler_frontend::ast::expressions::expression_store::ExpressionView;
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::hir::expression_store::HirConstructionFailure;
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::hir_builder::HirBuilder;
use crate::compiler_frontend::hir::ids::HirValueId;
use crate::compiler_frontend::hir::numeric::{HirNumericOp, HirNumericOperands};
use crate::compiler_frontend::source::SourceSpan;

impl<'a> HirBuilder<'a> {
    /// Returns the resolved numeric facts for an arithmetic operation.
    pub(crate) fn resolved_numeric_operation<'f>(
        &self,
        facts: &'f ResolvedOperation,
    ) -> Result<&'f ResolvedNumericOperation, HirConstructionFailure> {
        let ResolvedOperationKind::Numeric(numeric) = &facts.kind else {
            return Err(InfrastructureFailure::hir_transformation(
                "Arithmetic reached HIR without resolved numeric facts",
            )
            .into());
        };
        Ok(numeric)
    }

    /// Checks that a range operation carries resolved integer or binary float facts.
    pub(crate) fn validate_range_operation(
        &self,
        facts: &ResolvedOperation,
    ) -> Result<(), HirConstructionFailure> {
        let ResolvedOperationKind::Range { domain } = &facts.kind else {
            return Err(InfrastructureFailure::hir_transformation(
                "Range operation reached HIR without resolved range facts",
            )
            .into());
        };
        if !domain.is_integer() && !domain.is_binary_float() {
            return Err(InfrastructureFailure::hir_transformation(
                "Resolved range operation has an invalid numeric domain",
            )
            .into());
        }
        Ok(())
    }

    /// Emits a checked numeric operation once its operands have been lowered.
    pub(crate) fn finish_resolved_numeric_value(
        &mut self,
        left: HirValueId,
        right: Option<HirValueId>,
        facts: &ResolvedOperation,
    ) -> Result<HirValueId, HirConstructionFailure> {
        let span = &facts.span;
        let numeric = self.resolved_numeric_operation(facts)?;
        let left = self.lower_resolved_numeric_operand(
            left,
            numeric.left_domain,
            numeric.left_conversion,
        )?;
        let operands = match right {
            Some(right) => {
                let Some(right_domain) = numeric.right_domain else {
                    return Err(InfrastructureFailure::hir_transformation(
                        "Binary numeric operation is missing its resolved right domain",
                    )
                    .into());
                };
                let right = self.lower_resolved_numeric_operand(
                    right,
                    right_domain,
                    numeric.right_conversion,
                )?;
                HirNumericOperands::Binary { left, right }
            }
            None => HirNumericOperands::Unary { operand: left },
        };
        let result_type = self.lower_type_id(facts.result_type)?;
        self.emit_checked_numeric_value(
            HirNumericOp {
                operator: numeric.operator,
                domain: numeric.domain,
            },
            operands,
            result_type,
            span,
            numeric.discharged,
        )
    }

    /// Emits a comparison once both operands have been lowered.
    pub(crate) fn finish_resolved_comparison_value(
        &mut self,
        left: HirValueId,
        right: HirValueId,
        facts: &ResolvedOperation,
    ) -> Result<HirValueId, HirConstructionFailure> {
        let span = &facts.span;
        let (left, right) = match &facts.kind {
            ResolvedOperationKind::Comparison(comparison) => {
                let left = self.lower_resolved_numeric_operand(
                    left,
                    comparison.left_domain,
                    comparison.left_conversion,
                )?;
                let right = self.lower_resolved_numeric_operand(
                    right,
                    comparison.right_domain,
                    comparison.right_conversion,
                )?;
                (left, right)
            }
            ResolvedOperationKind::NonNumeric => (left, right),
            _ => {
                return Err(InfrastructureFailure::hir_transformation(
                    "Comparison reached HIR with invalid resolved operation facts",
                )
                .into());
            }
        };

        let region = self.current_region_or_error()?;
        let operation = self.lower_bin_op(&facts.operator)?;
        self.make_expression(
            span,
            HirExpressionKind::BinOp {
                op: operation,
                left,
                right,
            },
            facts.result_type,
            ValueKind::RValue,
            region,
        )
    }

    /// Emits logical `not` once its operand has been lowered.
    pub(crate) fn finish_logical_not_value(
        &mut self,
        operand: HirValueId,
        expression: ExpressionView<'_>,
        span: &Option<SourceSpan>,
    ) -> Result<HirValueId, HirConstructionFailure> {
        let region = self.current_region_or_error()?;
        let operation = self.lower_unary_op(&Operator::Not)?;
        self.make_expression(
            span,
            HirExpressionKind::UnaryOp {
                op: operation,
                operand,
            },
            expression.value_type_id()?,
            ValueKind::RValue,
            region,
        )
    }

    /// Emits a range value once both bounds have been lowered.
    pub(crate) fn finish_range_value(
        &mut self,
        start: HirValueId,
        end: HirValueId,
        facts: &ResolvedOperation,
    ) -> Result<HirValueId, HirConstructionFailure> {
        let span = &facts.span;
        let region = self.current_region_or_error()?;
        self.make_expression(
            span,
            HirExpressionKind::Range { start, end },
            facts.result_type,
            ValueKind::RValue,
            region,
        )
    }
}
