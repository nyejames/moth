//! Arithmetic and non-comparison binary operator typing policy.
//!
//! WHAT: resolves result types for arithmetic operators (+, -, *, /, //, %, **) on scalar operands.
//! WHY: arithmetic rules must stay explicit so implicit broad compatibility cannot quietly
//!      weaken type safety; mixed numeric promotion is intentionally narrow.

use super::diagnostics::invalid_operator_types;
use super::shared::is_mixed_int_float;
use crate::compiler_frontend::ast::expressions::eval_expression::typing_error::ExpressionTypingError;
use crate::compiler_frontend::ast::expressions::expression::Operator;
use crate::compiler_frontend::compiler_errors::SourceLocation;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;

pub(super) fn resolve_arithmetic_operator_type(
    lhs: TypeId,
    rhs: TypeId,
    op: &Operator,
    location: &SourceLocation,
    type_environment: &TypeEnvironment,
) -> Result<TypeId, ExpressionTypingError> {
    let builtins = type_environment.builtins();

    if lhs == rhs {
        // Same-type operator handling stays explicit so broad "compatible" types cannot quietly
        // weaken arithmetic rules.
        if lhs == builtins.int {
            return match op {
                Operator::Add
                | Operator::Subtract
                | Operator::Multiply
                | Operator::Modulus
                | Operator::Exponent
                | Operator::IntDivide => Ok(builtins.int),

                // Standard division always produces Float, even when both operands are Int.
                Operator::Divide => Ok(builtins.float),

                // Range construction is only valid between two Int operands.
                Operator::Range => Ok(builtins.range),

                _ => invalid_operator_types(lhs, rhs, op, location),
            };
        }

        if lhs == builtins.float {
            return match op {
                Operator::Add
                | Operator::Subtract
                | Operator::Multiply
                | Operator::Divide
                | Operator::Modulus
                | Operator::Exponent => Ok(builtins.float),

                _ => invalid_operator_types(lhs, rhs, op, location),
            };
        }
    }

    if is_mixed_int_float(lhs, rhs, type_environment) {
        // Mixed numeric promotion is intentionally narrow: only Int/Float pairs mix implicitly,
        // and only for numeric arithmetic/comparisons.
        return match op {
            Operator::Add
            | Operator::Subtract
            | Operator::Multiply
            | Operator::Divide
            | Operator::Modulus
            | Operator::Exponent => Ok(builtins.float),

            _ => invalid_operator_types(lhs, rhs, op, location),
        };
    }

    invalid_operator_types(lhs, rhs, op, location)
}
