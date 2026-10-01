//! Arithmetic and non-comparison binary operator typing policy.
//!
//! WHAT: resolves arithmetic result types and the existing Int-only range operator.
//! WHY: the canonical numeric policy owns promotion so AST typing, folding, and lowering use the
//!      same domain; ranges remain a separate language operation.

use super::diagnostics::invalid_operator_types;
use crate::compiler_frontend::ast::expressions::eval_expression::typing_error::ExpressionTypingError;
use crate::compiler_frontend::ast::expressions::expression::Operator;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::datatypes::numeric_operators::binary_operation_domain;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::source::SourceSpan;

pub(super) fn resolve_arithmetic_operator_type(
    lhs: TypeId,
    rhs: TypeId,
    op: &Operator,
    span: Option<SourceSpan>,
    type_environment: &TypeEnvironment,
) -> Result<TypeId, ExpressionTypingError> {
    let builtins = type_environment.builtins();

    if matches!(op, Operator::Range) {
        return if lhs == builtins.int && rhs == builtins.int {
            Ok(builtins.range)
        } else {
            invalid_operator_types(lhs, rhs, op, span)
        };
    }

    let Some(numeric_operator) = op.numeric_operator() else {
        return invalid_operator_types(lhs, rhs, op, span);
    };
    let Some(left) = NumericScalar::from_type_id(lhs, type_environment) else {
        return invalid_operator_types(lhs, rhs, op, span);
    };
    let Some(right) = NumericScalar::from_type_id(rhs, type_environment) else {
        return invalid_operator_types(lhs, rhs, op, span);
    };
    let Some(domain) = binary_operation_domain(numeric_operator, left, right) else {
        return invalid_operator_types(lhs, rhs, op, span);
    };

    Ok(domain.type_id(type_environment))
}
