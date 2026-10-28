//! Contextual coercion for explicit type boundaries.
//!
//! WHAT: applies the one implicit conversion that Moth allows at an explicit
//! boundary: wrapping a value into its optional target (`T -> T?`).
//! WHY: this module coerces already-typed values after natural expression typing
//! has completed, inserting explicit AST coercion nodes at the boundary. Numeric
//! conversions are explicit-only and never inserted here. Context for unresolved
//! literal arithmetic belongs to the indexed RPN result-typing owner in
//! `ast::expressions::eval_expression::result_type`.

use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExpressionConstructionFailure, OwnedExpr,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::types_borrows_targets;
use crate::compiler_frontend::compiler_messages::{TypeMismatchContext, ValueReceiverContext};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;

use crate::compiler_frontend::type_coercion::compatibility::is_type_compatible;

/// Result for the contextual coercion boundary.
///
/// Type mismatches are drafts carried through `ExpressionConstructionFailure`
/// until the owning accumulation boundary.
type ContextualCoercionResult<T> = Result<T, ExpressionConstructionFailure>;

/// Validates and applies the contextual coercion policy for an explicit typed boundary.
///
/// WHAT: accepts exact matches and the ordinary contextual `T -> T?` option wrap.
/// WHY: declarations, returns, produced values, and explicit collection elements should share one
/// frontend-owned policy rather than duplicating the same compatibility checks. Numeric
/// conversions are explicit-only, so no integer-to-float receiving promotion exists here.
pub(crate) fn coerce_expression_to_explicit_type_boundary(
    expression: OwnedExpr,
    expected_type_id: TypeId,
    type_environment: &TypeEnvironment,
    mismatch_context: TypeMismatchContext,
    expression_builder: &mut AstExpressionBuilder,
) -> ContextualCoercionResult<OwnedExpr> {
    let view = expression_builder.view_owned(expression)?;
    let actual_type_id = view.require_single(value_receiver_context(mismatch_context))?;
    let span = view.span();
    if actual_type_id == expected_type_id {
        return Ok(expression);
    }

    if is_type_compatible(expected_type_id, actual_type_id, type_environment) {
        return coerce_expression_to_declared_type(
            expression,
            expected_type_id,
            type_environment,
            expression_builder,
        );
    }

    Err(types_borrows_targets::type_mismatch(
        expected_type_id,
        actual_type_id,
        mismatch_context,
        span,
    )
    .into())
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
/// Callers must already have validated that `expr` produces exactly one success slot.
pub(crate) fn coerce_expression_to_declared_type(
    expression: OwnedExpr,
    declared_type_id: TypeId,
    type_environment: &TypeEnvironment,
    expression_builder: &mut AstExpressionBuilder,
) -> ContextualCoercionResult<OwnedExpr> {
    // Numeric conversions remain explicit. Only an option receiver adds a semantic wrapper.
    let actual_type_id = expression_builder.view_owned(expression)?.value_type_id()?;
    if should_wrap_in_option(actual_type_id, declared_type_id, type_environment) {
        return expression_builder.coerced(expression, declared_type_id);
    }

    Ok(expression)
}

fn value_receiver_context(context: TypeMismatchContext) -> ValueReceiverContext {
    match context {
        TypeMismatchContext::Assignment => ValueReceiverContext::Assignment,
        TypeMismatchContext::Declaration => ValueReceiverContext::Declaration,
        TypeMismatchContext::ReturnValue | TypeMismatchContext::ErrorReturn => {
            ValueReceiverContext::Return
        }
        TypeMismatchContext::FunctionArgument | TypeMismatchContext::ReceiverArgument => {
            ValueReceiverContext::FunctionArgument
        }
        TypeMismatchContext::AssertionArgument => ValueReceiverContext::AssertionArgument,
        TypeMismatchContext::ConstructorArgument | TypeMismatchContext::StructFieldDefault => {
            ValueReceiverContext::ConstructorArgument
        }
        TypeMismatchContext::Operator => ValueReceiverContext::OperatorOperand,
        TypeMismatchContext::Condition => ValueReceiverContext::Condition,
        TypeMismatchContext::CollectionElement => ValueReceiverContext::CollectionElement,
        TypeMismatchContext::TemplateInterpolation => ValueReceiverContext::TemplateInterpolation,
        TypeMismatchContext::MatchScrutinee => ValueReceiverContext::MatchScrutinee,
        TypeMismatchContext::MatchPattern
        | TypeMismatchContext::Pattern
        | TypeMismatchContext::General => ValueReceiverContext::General,
    }
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
