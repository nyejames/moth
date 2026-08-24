//! Expression result-type resolution for AST evaluation.
//!
//! WHAT: mirrors final RPN execution shape with a `TypeId`-only stack.
//! WHY: AST must enforce operator typing before folding/lowering so later stages never infer
//! type policy from runtime-oriented structures. The stack carries semantic IDs only: operator
//! policy decides on `TypeId` equality, and diagnostics resolve their own spelling from the
//! `TypeId` at the point they are built, so a successful typing pass materialises no `DataType`.

use super::operator_policy::{resolve_binary_operator_type, resolve_unary_operator_type};
use super::typing_error::ExpressionTypingError;
use crate::compiler_frontend::ast::expressions::expression::Operator;
use crate::compiler_frontend::ast::expressions::expression_rpn::ExpressionRpnItem;
use crate::compiler_frontend::compiler_errors::{CompilerError, SourceLocation};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidExpressionReason, OperatorOperandPosition,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::instrumentation::{AstCounter, add_ast_counter};
use crate::compiler_frontend::symbols::string_interning::StringTable;

pub(super) fn resolve_expression_result_type(
    output_queue: &[ExpressionRpnItem],
    expression_location: &SourceLocation,
    string_table: &mut StringTable,
    type_environment: &TypeEnvironment,
) -> Result<TypeId, ExpressionTypingError> {
    // Mirror the final RPN evaluation shape with a type-only stack so operator diagnostics fire
    // before constant folding mutates any nodes.
    add_ast_counter(AstCounter::ExpressionTypedStackItems, output_queue.len());

    let mut stack: Vec<TypeId> = Vec::with_capacity(output_queue.len());

    // ------------------------
    //  Walk RPN output queue
    // ------------------------

    for item in output_queue {
        match item {
            // Operand expressions push their pre-resolved types directly.
            ExpressionRpnItem::Operand(expression) => {
                stack.push(expression.type_id);
            }

            // Operators consume operand types from the stack and push the result type.
            ExpressionRpnItem::Operator { operator, location } => {
                match operator.required_values() {
                    1 => {
                        let Some(operand) = stack.pop() else {
                            return Err(missing_operand_error(
                                operator,
                                OperatorOperandPosition::Unary,
                                location,
                                string_table,
                            ));
                        };
                        stack.push(resolve_unary_operator_type(
                            operator,
                            operand,
                            location,
                            type_environment,
                        )?);
                    }

                    2 => {
                        let Some(rhs) = stack.pop() else {
                            return Err(missing_operand_error(
                                operator,
                                OperatorOperandPosition::BinaryRight,
                                location,
                                string_table,
                            ));
                        };
                        let Some(lhs) = stack.pop() else {
                            return Err(missing_operand_error(
                                operator,
                                OperatorOperandPosition::BinaryLeft,
                                location,
                                string_table,
                            ));
                        };
                        stack.push(resolve_binary_operator_type(
                            lhs,
                            rhs,
                            operator,
                            location,
                            type_environment,
                        )?);
                    }

                    _ => {
                        return Err(CompilerError::compiler_error(format!(
                            "Unsupported operator arity during expression typing: {:?}",
                            operator
                        ))
                        .into());
                    }
                }
            }
        }
    }

    // ------------------------
    //  Validate final stack shape
    // ------------------------

    if stack.len() != 1 {
        return Err(CompilerDiagnostic::invalid_expression(
            InvalidExpressionReason::UnresolvedStackShape,
            expression_location.clone(),
        )
        .into());
    }

    // ------------------------
    //  Extract resolved result
    // ------------------------

    // stack.len() == 1 guarantees pop() returns Some; the None arm guards a compiler bug.
    match stack.pop() {
        Some(resolved_type) => Ok(resolved_type),
        None => Err(CompilerError::compiler_error(
            "Expression typing stack unexpectedly empty after shape validation.",
        )
        .into()),
    }
}

/// Build a missing-operand diagnostic for the given operator and stack position.
fn missing_operand_error(
    operator: &Operator,
    position: OperatorOperandPosition,
    location: &SourceLocation,
    string_table: &mut StringTable,
) -> ExpressionTypingError {
    CompilerDiagnostic::missing_operator_operand(
        string_table.get_or_intern(operator.to_str().to_owned()),
        position,
        location.clone(),
    )
    .into()
}
