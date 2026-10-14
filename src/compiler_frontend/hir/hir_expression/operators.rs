//! Operator-specific HIR expression lowering helpers.
//!
//! WHAT: lowers unary and binary AST operators into explicit HIR expression nodes.
//! WHY: keeping operator handling separate makes the core expression lowering loop easier to follow.
//!
//! ## Diagnostic boundary
//!
//! `CompilerError` / `return_hir_transformation_error!` in this module means an internal
//! HIR transformation or lowering invariant failure only.

use crate::compiler_frontend::ast::expressions::expression_kind::Operator;
use crate::compiler_frontend::hir::expression_store::HirConstructionFailure;
use crate::compiler_frontend::hir::hir_builder::HirBuilder;
use crate::compiler_frontend::hir::operators::{HirBinOp, HirUnaryOp};
use crate::compiler_frontend::source::SourceSpan;
use crate::return_hir_transformation_error;

impl<'a> HirBuilder<'a> {
    pub(super) fn lower_bin_op(
        &self,
        op: &Operator,
        span: &Option<SourceSpan>,
    ) -> Result<HirBinOp, HirConstructionFailure> {
        match op {
            Operator::Add
            | Operator::Subtract
            | Operator::Multiply
            | Operator::Divide
            | Operator::IntDivide
            | Operator::Modulus
            | Operator::Exponent => {
                return_hir_transformation_error!(
                    "Arithmetic operators must be lowered through HirStatementKind::NumericOp",
                    self.hir_error_location(span)
                )
            }
            Operator::And => Ok(HirBinOp::And),
            Operator::Or => Ok(HirBinOp::Or),
            Operator::GreaterThan => Ok(HirBinOp::Gt),
            Operator::GreaterThanOrEqual => Ok(HirBinOp::Ge),
            Operator::LessThan => Ok(HirBinOp::Lt),
            Operator::LessThanOrEqual => Ok(HirBinOp::Le),
            Operator::Equality => Ok(HirBinOp::Eq),
            Operator::NotEqual => Ok(HirBinOp::Ne),
            Operator::Not => {
                return_hir_transformation_error!(
                    "'not' cannot be lowered as a binary operator",
                    self.hir_error_location(span)
                )
            }
            Operator::Negate => {
                return_hir_transformation_error!(
                    "Unary negation cannot be lowered as a binary operator",
                    self.hir_error_location(span)
                )
            }
            Operator::Range => {
                return_hir_transformation_error!(
                    "Range operator is lowered as HirExpressionKind::Range",
                    self.hir_error_location(span)
                )
            }
        }
    }

    pub(super) fn lower_unary_op(
        &self,
        op: &Operator,
        span: &Option<SourceSpan>,
    ) -> Result<HirUnaryOp, HirConstructionFailure> {
        match op {
            Operator::Not => Ok(HirUnaryOp::Not),
            Operator::Negate => Ok(HirUnaryOp::Neg),
            _ => {
                return_hir_transformation_error!(
                    format!("Unsupported unary operator: {:?}", op),
                    self.hir_error_location(span)
                )
            }
        }
    }
}
