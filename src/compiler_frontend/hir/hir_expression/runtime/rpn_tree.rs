//! Build a runtime RPN expression tree from expression-owned RPN items.
//!
//! WHAT: converts a flat expression RPN stack into a tree-shaped `RuntimeRpnTree` that
//!       preserves parser precedence while enabling dedicated CFG lowering.
//! WHY: tree-shaped RPN makes short-circuit `and`/`or` lowering possible and keeps
//!      operator arity validation in one place before HIR expression emission.

use crate::compiler_frontend::ast::expressions::expression_rpn::{
    ExpressionRpn, ExpressionRpnItem,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::hir::hir_builder::HirBuilder;
use crate::compiler_frontend::source::SourceSpan;
use crate::return_hir_transformation_error;

use super::RuntimeRpnTree;

impl<'a> HirBuilder<'a> {
    pub(super) fn build_runtime_rpn_tree(
        &self,
        rpn: &ExpressionRpn,
        source_span: &Option<SourceSpan>,
    ) -> Result<RuntimeRpnTree, CompilerError> {
        let mut stack: Vec<RuntimeRpnTree> = Vec::with_capacity(rpn.items.len());

        for item in &rpn.items {
            match item {
                ExpressionRpnItem::Operator {
                    operator,
                    span: operator_span,
                } => match operator.required_values() {
                    1 => {
                        let Some(operand) = stack.pop() else {
                            return_hir_transformation_error!(
                                format!("RPN stack underflow for unary operator {:?}", operator),
                                self.hir_error_location(operator_span)
                            );
                        };

                        stack.push(RuntimeRpnTree::Unary {
                            op: operator.to_owned(),
                            operand: Box::new(operand),
                            span: *operator_span,
                        });
                    }
                    2 => {
                        let Some(right) = stack.pop() else {
                            return_hir_transformation_error!(
                                format!(
                                    "RPN stack underflow for operator {:?} (missing rhs)",
                                    operator
                                ),
                                self.hir_error_location(operator_span)
                            );
                        };
                        let Some(left) = stack.pop() else {
                            return_hir_transformation_error!(
                                format!(
                                    "RPN stack underflow for operator {:?} (missing lhs)",
                                    operator
                                ),
                                self.hir_error_location(operator_span)
                            );
                        };

                        stack.push(RuntimeRpnTree::Binary {
                            left: Box::new(left),
                            op: operator.to_owned(),
                            right: Box::new(right),
                            span: *operator_span,
                        });
                    }
                    _ => {
                        return_hir_transformation_error!(
                            format!("Unsupported operator arity for {:?}", operator),
                            self.hir_error_location(operator_span)
                        );
                    }
                },
                ExpressionRpnItem::Operand(expression) => {
                    stack.push(RuntimeRpnTree::Leaf(Box::new(expression.to_owned())));
                }
            }
        }

        if stack.len() != 1 {
            return_hir_transformation_error!(
                format!(
                    "Malformed runtime RPN expression: expected one value on stack, got {}",
                    stack.len()
                ),
                self.hir_error_location(source_span)
            );
        }

        let Some(tree) = stack.pop() else {
            return_hir_transformation_error!(
                "Malformed runtime RPN expression: validated stack unexpectedly empty",
                self.hir_error_location(source_span)
            );
        };

        Ok(tree)
    }
}
