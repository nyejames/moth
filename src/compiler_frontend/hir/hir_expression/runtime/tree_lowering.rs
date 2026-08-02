//! Lower runtime RPN trees into HIR expressions.
//!
//! WHAT: walks a `RuntimeRpnTree` and emits `HirExpression` values, including calls
//!       to operators, external functions, and fallible carriers.
//! WHY: this is the bridge from AST-owned runtime expression shape to HIR-owned
//!      value kind and effect representation.

use crate::compiler_frontend::ast::expressions::expression::Operator;
use crate::compiler_frontend::ast::expressions::expression_rpn::ExpressionRpn;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::ids::TypeId as FrontendTypeId;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::hir::expressions::{HirExpression, HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::hir_builder::HirBuilder;
use crate::compiler_frontend::hir::numeric::HirNumericOperands;
use crate::compiler_frontend::hir::operators::HirUnaryOp;
use crate::compiler_frontend::hir::statements::HirStatement;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;

use super::super::LoweredExpression;
use super::RuntimeRpnTree;

impl<'a> HirBuilder<'a> {
    // WHAT: evaluates AST runtime expressions stored in expression-owned RPN order into HIR values.
    // WHY: this keeps parser precedence decisions intact while enabling dedicated CFG lowering for
    //      short-circuit `and`/`or` so RHS side effects stay branch-gated.
    pub(crate) fn lower_runtime_rpn_expression(
        &mut self,
        rpn: &ExpressionRpn,
        location: &SourceLocation,
        expr_type_id: FrontendTypeId,
    ) -> Result<LoweredExpression, CompilerError> {
        let tree = self.build_runtime_rpn_tree(rpn, location)?;
        let mut lowered = self.lower_runtime_tree_node(&tree, location)?;
        let expected_ty = self.lower_type_id(expr_type_id, location)?;
        lowered.value.ty = expected_ty;
        Ok(lowered)
    }

    // WHAT: lowers a runtime RPN tree and emits its prelude into the active block,
    //       returning only the resulting HIR value.
    // WHY: short-circuit CFG construction needs a value for conditions and jump arguments,
    //      but cannot return pending prelude to a parent expression because the work must
    //      be sequenced inside the branch block that owns it.
    pub(super) fn lower_runtime_tree_value_to_current_block(
        &mut self,
        node: &RuntimeRpnTree,
        location: &SourceLocation,
    ) -> Result<HirExpression, CompilerError> {
        let lowered = self.lower_runtime_tree_node(node, location)?;
        for prelude in lowered.prelude {
            self.emit_statement_to_current_block(prelude, location)?;
        }
        Ok(lowered.value)
    }

    pub(super) fn lower_runtime_tree_node(
        &mut self,
        node: &RuntimeRpnTree,
        _location: &SourceLocation,
    ) -> Result<LoweredExpression, CompilerError> {
        match node {
            RuntimeRpnTree::Leaf(expression) => {
                if self.expression_needs_current_block_lowering(expression) {
                    let value = self.lower_expression_value_to_current_block(expression)?;
                    Ok(LoweredExpression {
                        prelude: vec![],
                        value,
                    })
                } else {
                    self.lower_expression(expression)
                }
            }

            RuntimeRpnTree::Unary {
                op,
                operand,
                location,
            } => {
                let mut prelude = Vec::new();
                let lowered_operand =
                    self.lower_runtime_tree_child_for_parent(&mut prelude, operand, location)?;
                let region = self.current_region_or_error(location)?;
                let hir_op = self.lower_unary_op(op, location)?;
                let result_ty = match hir_op {
                    HirUnaryOp::Not => builtin_type_ids::BOOL,
                    HirUnaryOp::Neg => lowered_operand.ty,
                };

                // Numeric negation is a checked effect and must go through NumericOp.
                if *op == Operator::Negate
                    && let Some((numeric_op, numeric_result_ty)) =
                        self.classify_checked_numeric_negation(&lowered_operand)
                {
                    for prelude_statement in prelude.drain(..) {
                        self.emit_statement_to_current_block(prelude_statement, location)?;
                    }
                    let value = self.emit_checked_numeric_value(
                        numeric_op,
                        HirNumericOperands::Unary {
                            operand: lowered_operand,
                        },
                        numeric_result_ty,
                        location,
                    )?;
                    return Ok(LoweredExpression {
                        prelude: vec![],
                        value,
                    });
                }

                Ok(LoweredExpression {
                    prelude,
                    value: self.make_expression(
                        location,
                        HirExpressionKind::UnaryOp {
                            op: hir_op,
                            operand: Box::new(lowered_operand),
                        },
                        result_ty,
                        ValueKind::RValue,
                        region,
                    ),
                })
            }
            RuntimeRpnTree::Binary {
                left,
                op,
                right,
                location,
            } => {
                if matches!(op, Operator::And | Operator::Or) {
                    return self.lower_short_circuit_binary_expression(left, op, right, location);
                }

                let mut prelude = Vec::new();
                let lowered_left =
                    self.lower_runtime_tree_child_for_parent(&mut prelude, left, location)?;
                let lowered_right =
                    self.lower_runtime_tree_child_for_parent(&mut prelude, right, location)?;
                let region = self.current_region_or_error(location)?;

                if matches!(op, Operator::Range) {
                    let range_ty = builtin_type_ids::RANGE;
                    return Ok(LoweredExpression {
                        prelude,
                        value: self.make_expression(
                            location,
                            HirExpressionKind::Range {
                                start: Box::new(lowered_left),
                                end: Box::new(lowered_right),
                            },
                            range_ty,
                            ValueKind::RValue,
                            region,
                        ),
                    });
                }

                // Numeric arithmetic is lowered as a checked NumericOp statement. Comparisons
                // and booleans retain plain BinOp form, ranges use the dedicated Range node,
                // and compiler-owned template appends use HirBinOp::StringAppend.
                if let Some((numeric_op, numeric_result_ty)) =
                    self.classify_checked_numeric_binop(op, &lowered_left, &lowered_right)
                {
                    for prelude_statement in prelude.drain(..) {
                        self.emit_statement_to_current_block(prelude_statement, location)?;
                    }
                    let (left, right) = self.lower_checked_numeric_binary_operands(
                        numeric_op,
                        lowered_left,
                        lowered_right,
                        location,
                    )?;
                    let value = self.emit_checked_numeric_value(
                        numeric_op,
                        HirNumericOperands::Binary { left, right },
                        numeric_result_ty,
                        location,
                    )?;
                    return Ok(LoweredExpression {
                        prelude: vec![],
                        value,
                    });
                }

                let hir_op = self.lower_bin_op(op, location)?;
                let result_ty =
                    self.infer_binop_result_type(lowered_left.ty, lowered_right.ty, hir_op);

                Ok(LoweredExpression {
                    prelude,
                    value: self.make_expression(
                        location,
                        HirExpressionKind::BinOp {
                            op: hir_op,
                            left: Box::new(lowered_left),
                            right: Box::new(lowered_right),
                        },
                        result_ty,
                        ValueKind::RValue,
                        region,
                    ),
                })
            }
        }
    }

    fn lower_runtime_tree_child_for_parent(
        &mut self,
        pending_prelude: &mut Vec<HirStatement>,
        child: &RuntimeRpnTree,
        location: &SourceLocation,
    ) -> Result<HirExpression, CompilerError> {
        if self.runtime_tree_needs_current_block_lowering(child) {
            for prelude in pending_prelude.drain(..) {
                self.emit_statement_to_current_block(prelude, location)?;
            }

            return self.lower_runtime_tree_value_to_current_block(child, location);
        }

        let lowered = self.lower_runtime_tree_node(child, location)?;
        pending_prelude.extend(lowered.prelude);
        Ok(lowered.value)
    }

    fn runtime_tree_needs_current_block_lowering(&self, node: &RuntimeRpnTree) -> bool {
        match node {
            RuntimeRpnTree::Leaf(expression) => {
                self.expression_needs_current_block_lowering(expression)
            }

            RuntimeRpnTree::Unary { op, operand, .. } => {
                matches!(op, Operator::Negate)
                    || self.runtime_tree_needs_current_block_lowering(operand)
            }

            RuntimeRpnTree::Binary {
                left, op, right, ..
            } => {
                matches!(op, Operator::And | Operator::Or)
                    || Self::operator_emits_checked_numeric_statement(op)
                    || self.runtime_tree_needs_current_block_lowering(left)
                    || self.runtime_tree_needs_current_block_lowering(right)
            }
        }
    }

    /// Whether lowering this operator can emit a checked numeric statement.
    ///
    /// WHAT: conservatively treats arithmetic-shaped operators as current-block effects.
    /// WHY: `NumericOp` emission may append statements and, in recoverable contexts, split CFG.
    ///      Parent lowering must flush pending left-side preludes before visiting such a child so
    ///      source evaluation order remains left-to-right. Boolean comparisons remain ordinary
    ///      binary operations; internal template construction uses the dedicated `StringAppend`
    ///      operator and follows the same current-block boundary.
    fn operator_emits_checked_numeric_statement(op: &Operator) -> bool {
        matches!(
            op,
            Operator::Add
                | Operator::Subtract
                | Operator::Multiply
                | Operator::Divide
                | Operator::IntDivide
                | Operator::Modulus
                | Operator::Exponent
        )
    }
}
