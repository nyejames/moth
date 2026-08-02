//! Shared operator-policy helpers for binary expression typing.
//!
//! WHAT: small predicates and guards used by arithmetic, comparison, and logical
//!      operator policy modules.
//! WHY: operator categories share narrow rules (mixed numeric detection and fallible-carrier
//!      rejection) that are easier to review in one place.

use super::super::result_type::ExpressionResultType;
use crate::compiler_frontend::ast::expressions::eval_expression::typing_error::ExpressionTypingError;
use crate::compiler_frontend::ast::expressions::expression::Operator;
use crate::compiler_frontend::compiler_errors::SourceLocation;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidFallibleOperandReason, UnsupportedOperatorCategory,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;

/// Rejects binary operators applied to unwrapped fallible `Error!` carriers.
///
/// WHAT: guards every binary operator path so that fallible carriers cannot silently
///      participate in arithmetic, comparison, or logical operations.
/// WHY: unwrapped fallible operators are deferred to later pipeline stages; AST typing
///      must emit a clear diagnostic here instead of allowing an invalid type through.
pub(super) fn reject_fallible_operands(
    lhs: &ExpressionResultType,
    rhs: &ExpressionResultType,
    op: &Operator,
    location: &SourceLocation,
    type_environment: &TypeEnvironment,
) -> Result<(), ExpressionTypingError> {
    if type_environment.is_fallible_carrier(lhs.type_id)
        || type_environment.is_fallible_carrier(rhs.type_id)
    {
        let operand_type_id = if type_environment.is_fallible_carrier(lhs.type_id) {
            lhs.type_id
        } else {
            rhs.type_id
        };

        let category = match op {
            // Arithmetic operators.
            Operator::Add
            | Operator::Subtract
            | Operator::Multiply
            | Operator::Divide
            | Operator::IntDivide
            | Operator::Modulus
            | Operator::Exponent => UnsupportedOperatorCategory::Arithmetic,

            // Comparison operators.
            Operator::Equality
            | Operator::NotEqual
            | Operator::GreaterThan
            | Operator::GreaterThanOrEqual
            | Operator::LessThan
            | Operator::LessThanOrEqual => UnsupportedOperatorCategory::Comparison,

            // Logical operators.
            Operator::And | Operator::Or => UnsupportedOperatorCategory::Logical,

            // Any operator not covered above.
            _ => UnsupportedOperatorCategory::Other,
        };

        return Err(CompilerDiagnostic::invalid_fallible_operand(
            InvalidFallibleOperandReason::FallibleValueNotHandled,
            category,
            operand_type_id,
            location.clone(),
        )
        .into());
    }

    Ok(())
}

/// Returns `true` when one operand is `Int` and the other is `Float`.
///
/// WHAT: detects the narrow mixed-numeric pair that implicit promotion supports.
/// WHY: mixed `Int`/`Float` promotion is intentionally restricted so broader
///      "compatible" types cannot quietly weaken arithmetic or comparison rules.
pub(super) fn is_mixed_int_float(
    lhs: &ExpressionResultType,
    rhs: &ExpressionResultType,
    type_environment: &TypeEnvironment,
) -> bool {
    let builtins = type_environment.builtins();

    (lhs.type_id == builtins.int && rhs.type_id == builtins.float)
        || (lhs.type_id == builtins.float && rhs.type_id == builtins.int)
}
