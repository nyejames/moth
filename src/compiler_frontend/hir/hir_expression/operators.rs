//! Operator-specific HIR expression lowering helpers.
//!
//! WHAT: lowers unary and binary AST operators into explicit HIR expression nodes.
//! WHY: keeping operator handling separate makes the core expression lowering loop easier to follow.
//!
//! ## Diagnostic boundary
//!
//! `CompilerError` / `return_hir_transformation_error!` in this module means an internal
//! HIR transformation or lowering invariant failure only.

use crate::compiler_frontend::ast::expressions::expression::Operator;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::hir::hir_builder::HirBuilder;
use crate::compiler_frontend::hir::operators::{HirBinOp, HirUnaryOp};
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::return_hir_transformation_error;

impl<'a> HirBuilder<'a> {
    pub(super) fn lower_bin_op(
        &self,
        op: &Operator,
        location: &SourceLocation,
    ) -> Result<HirBinOp, CompilerError> {
        match op {
            Operator::Add => Ok(HirBinOp::Add),
            Operator::Subtract => Ok(HirBinOp::Sub),
            Operator::Multiply => Ok(HirBinOp::Mul),
            Operator::Divide => Ok(HirBinOp::Div),
            Operator::IntDivide => Ok(HirBinOp::IntDiv),
            Operator::Modulus => Ok(HirBinOp::Mod),
            Operator::Exponent => Ok(HirBinOp::Exponent),
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
                    self.hir_error_location(location)
                )
            }
            Operator::Negate => {
                return_hir_transformation_error!(
                    "Unary negation cannot be lowered as a binary operator",
                    self.hir_error_location(location)
                )
            }
            Operator::Range => {
                return_hir_transformation_error!(
                    "Range operator is lowered as HirExpressionKind::Range",
                    self.hir_error_location(location)
                )
            }
        }
    }

    pub(super) fn lower_unary_op(
        &self,
        op: &Operator,
        location: &SourceLocation,
    ) -> Result<HirUnaryOp, CompilerError> {
        match op {
            Operator::Not => Ok(HirUnaryOp::Not),
            Operator::Negate => Ok(HirUnaryOp::Neg),
            _ => {
                return_hir_transformation_error!(
                    format!("Unsupported unary operator: {:?}", op),
                    self.hir_error_location(location)
                )
            }
        }
    }

    // WHAT: Infers binary-op result kinds for lowered runtime expressions.
    // WHY: Runtime RPN lowering needs a final type for each expression node.
    pub(super) fn infer_binop_result_type(
        &mut self,
        left: TypeId,
        right: TypeId,
        op: HirBinOp,
    ) -> TypeId {
        match op {
            HirBinOp::Eq
            | HirBinOp::Ne
            | HirBinOp::Lt
            | HirBinOp::Le
            | HirBinOp::Gt
            | HirBinOp::Ge
            | HirBinOp::And
            | HirBinOp::Or => builtin_type_ids::BOOL,

            // Checked numeric arithmetic is lowered through `HirStatementKind::NumericOp`, so the
            // only source-level plain binary operators that still reach this path are comparisons
            // and booleans. Runtime template appends use a distinct compiler-owned operator.
            HirBinOp::StringAppend => self.type_environment.builtins().string,
            HirBinOp::Add => {
                let float = self.type_environment.builtins().float;

                if left == float || right == float {
                    float
                } else {
                    left
                }
            }

            HirBinOp::Sub | HirBinOp::Mul | HirBinOp::Mod | HirBinOp::Exponent => left,

            HirBinOp::Div => self.type_environment.builtins().float,

            HirBinOp::IntDiv => builtin_type_ids::INT,
        }
    }
}
