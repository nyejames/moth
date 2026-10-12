//! Contextual coercion for explicit type boundaries.
//!
//! WHAT: applies the one implicit conversion that Moth allows at an explicit
//! boundary: wrapping a value into its optional target (`T -> T?`).
//! WHY: this module coerces already-typed values after natural expression typing
//! has completed, inserting explicit AST coercion nodes at the boundary. Numeric
//! conversions are explicit-only and never inserted here. Context for unresolved
//! literal arithmetic belongs to the indexed RPN result-typing owner in
//! `ast::expressions::eval_expression::result_type`.

use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, TypeMismatchContext};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;

use crate::compiler_frontend::type_coercion::compatibility::is_type_compatible;

/// Result for the contextual coercion boundary.
///
/// Coercion diagnostics remain plain `CompilerDiagnostic` values until the
/// owning accumulation boundary.
type ContextualCoercionResult<T> = Result<T, CompilerDiagnostic>;

/// Validates and applies the contextual coercion policy for an explicit typed boundary.
///
/// WHAT: accepts exact matches and the ordinary contextual `T -> T?` option wrap.
/// WHY: declarations, returns, produced values, and explicit collection elements should share one
/// frontend-owned policy rather than duplicating the same compatibility checks. Numeric
/// conversions are explicit-only, so no integer-to-float receiving promotion exists here.
pub(crate) fn coerce_expression_to_explicit_type_boundary(
    expression: Expression,
    expected_type_id: TypeId,
    type_environment: &TypeEnvironment,
    mismatch_context: TypeMismatchContext,
) -> ContextualCoercionResult<Expression> {
    if expression.type_id == expected_type_id {
        return Ok(expression);
    }

    if is_type_compatible(expected_type_id, expression.type_id, type_environment) {
        return Ok(coerce_expression_to_declared_type(
            expression,
            expected_type_id,
            type_environment,
        ));
    }

    Err(CompilerDiagnostic::type_mismatch(
        expected_type_id,
        expression.type_id,
        mismatch_context,
        expression.span,
    ))
}

/// Applies contextual coercion to `expr` if the target type requires it.
///
/// WHAT: post-parse coercion step for explicit type boundaries.
/// WHY: `create_expression` resolves the natural type of an expression. When a
/// boundary says `T?`, this function rewrites the expression to carry the
/// explicit target type so HIR/backend lowering can materialize the wrapping
/// instead of silently mistyping the inner value.
///
/// Returns the original expression unchanged when no coercion is needed.
pub(crate) fn coerce_expression_to_declared_type(
    expr: Expression,
    declared_type_id: TypeId,
    type_environment: &TypeEnvironment,
) -> Expression {
    // Every exact numeric identity requires an explicit `cast` to convert, so contextual
    // coercion never selects a numeric conversion for a typed value.
    if should_wrap_in_option(expr.type_id, declared_type_id, type_environment) {
        return Expression::coerced(expr, declared_type_id);
    }

    expr
}

fn should_wrap_in_option(
    actual_type_id: TypeId,
    declared_type_id: TypeId,
    type_environment: &TypeEnvironment,
) -> bool {
    if actual_type_id == declared_type_id {
        return false;
    }

    type_environment.option_inner_type(declared_type_id) == Some(actual_type_id)
}

#[cfg(test)]
#[path = "tests/contextual_tests.rs"]
mod contextual_tests;
