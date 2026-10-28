//! Operator-specific HIR expression lowering helpers.
//!
//! WHAT: lowers unary and binary AST operators into explicit HIR expression nodes.
//! WHY: keeping operator handling separate makes the core expression lowering loop easier to follow.
//!
//! ## Diagnostic boundary
//!
//! `InfrastructureFailure::hir_transformation` in this module means an internal
//! HIR transformation or lowering invariant failure only.

use crate::compiler_frontend::ast::expressions::expression_kind::Operator;
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::hir::expression_store::HirConstructionFailure;
use crate::compiler_frontend::hir::hir_builder::HirBuilder;
use crate::compiler_frontend::hir::operators::{HirBinOp, HirUnaryOp};

impl<'a> HirBuilder<'a> {
    pub(super) fn lower_bin_op(&self, op: &Operator) -> Result<HirBinOp, HirConstructionFailure> {
        match op {
            Operator::Add
            | Operator::Subtract
            | Operator::Multiply
            | Operator::Divide
            | Operator::IntDivide
            | Operator::Modulus
            | Operator::Exponent => Err(InfrastructureFailure::hir_transformation(
                "Arithmetic operators must be lowered through HirStatementKind::NumericOp",
            )
            .into()),
            Operator::And => Ok(HirBinOp::And),
            Operator::Or => Ok(HirBinOp::Or),
            Operator::GreaterThan => Ok(HirBinOp::Gt),
            Operator::GreaterThanOrEqual => Ok(HirBinOp::Ge),
            Operator::LessThan => Ok(HirBinOp::Lt),
            Operator::LessThanOrEqual => Ok(HirBinOp::Le),
            Operator::Equality => Ok(HirBinOp::Eq),
            Operator::NotEqual => Ok(HirBinOp::Ne),
            Operator::Not => Err(InfrastructureFailure::hir_transformation(
                "'not' cannot be lowered as a binary operator",
            )
            .into()),
            Operator::Negate => Err(InfrastructureFailure::hir_transformation(
                "Unary negation cannot be lowered as a binary operator",
            )
            .into()),
            Operator::Range => Err(InfrastructureFailure::hir_transformation(
                "Range operator is lowered as HirExpressionKind::Range",
            )
            .into()),
        }
    }

    pub(super) fn lower_unary_op(
        &self,
        op: &Operator,
    ) -> Result<HirUnaryOp, HirConstructionFailure> {
        match op {
            Operator::Not => Ok(HirUnaryOp::Not),
            Operator::Negate => Ok(HirUnaryOp::Neg),
            _ => crate::compiler_bug!(
                CompilerStage::HirConstruction,
                "Unsupported unary operator: {:?}",
                op,
            ),
        }
    }
}
