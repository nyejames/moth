//! Operator typing policy for AST expression evaluation.
//!
//! WHAT: resolves unary/binary operator result types for natural expressions.
//! WHY: AST is the policy owner for operator typing; contextual coercion happens at explicit
//! declaration/return boundaries after parsing.

mod arithmetic;
mod comparison;
mod diagnostics;
mod logical;
mod shared;
mod unary;

use crate::compiler_frontend::ast::expressions::eval_expression::typing_error::ExpressionTypingError;
use crate::compiler_frontend::ast::expressions::expression::Operator;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;

pub(super) fn resolve_unary_operator_type(
    op: &Operator,
    operand: TypeId,
    span: Option<SourceSpan>,
    type_environment: &TypeEnvironment,
) -> Result<TypeId, ExpressionTypingError> {
    unary::resolve_unary_operator_type(op, operand, span, type_environment)
}

pub(super) fn resolve_binary_operator_type(
    lhs: TypeId,
    rhs: TypeId,
    op: &Operator,
    span: Option<SourceSpan>,
    type_environment: &TypeEnvironment,
    path_fork: &PathInternerFork,
) -> Result<TypeId, ExpressionTypingError> {
    shared::reject_fallible_operands(lhs, rhs, op, span, type_environment)?;

    if logical::is_logical_operator(op) {
        return logical::resolve_logical_operator_type(lhs, rhs, op, span, type_environment);
    }

    if comparison::is_comparison_operator(op) {
        return comparison::resolve_comparison_operator_type(
            lhs,
            rhs,
            op,
            span,
            type_environment,
            path_fork,
        );
    }

    arithmetic::resolve_arithmetic_operator_type(lhs, rhs, op, span, type_environment)
}
