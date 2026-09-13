//! Unary operator typing policy.
//!
//! WHAT: resolves the result type of unary `not` and unary minus (`-`) expressions.
//! WHY: AST expression evaluation must enforce that `not` is strictly boolean and that
//!      unary minus only applies to the builtin `Int` and `Float` types, the exact set
//!      HIR can lower as checked numeric negation.

use crate::compiler_frontend::ast::expressions::eval_expression::typing_error::ExpressionTypingError;
use crate::compiler_frontend::ast::expressions::expression::Operator;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::source::SourceSpan;

use super::diagnostics::diagnostic_operator_from_ast;

/// Resolve the result type of a unary operator application.
///
/// `not` requires a boolean operand and returns `Bool`. Unary minus requires a builtin
/// `Int` or `Float` operand and preserves it; the tokenizer/parser already distinguish
/// negative literals from runtime unary subtraction.
pub(super) fn resolve_unary_operator_type(
    op: &Operator,
    operand: TypeId,
    span: Option<SourceSpan>,
    type_environment: &TypeEnvironment,
) -> Result<TypeId, ExpressionTypingError> {
    match op {
        Operator::Not => {
            let bool_type_id = type_environment.builtins().bool;

            if operand == bool_type_id {
                Ok(bool_type_id)
            } else {
                Err(CompilerDiagnostic::unsupported_operator_types(
                    diagnostic_operator_from_ast(op),
                    operand,
                    None,
                    span,
                )
                .into())
            }
        }

        // Unary minus accepts exactly the operand set HIR can lower: `classify_checked_numeric_negation`
        // maps only builtin `Int` and `Float` to `NumericOp`, so any other operand would reach
        // HIR as a plain `UnaryOp` and fail an internal-invariant check.
        Operator::Negate => {
            let builtins = type_environment.builtins();

            if operand == builtins.int || operand == builtins.float {
                Ok(operand)
            } else {
                Err(CompilerDiagnostic::unsupported_operator_types(
                    diagnostic_operator_from_ast(op),
                    operand,
                    None,
                    span,
                )
                .into())
            }
        }

        // Defensive fallback: `Not` and `Negate` are the only operators that can appear in
        // unary position. Keeping the arm unreachable without a panic preserves totality.
        _ => Err(CompilerError::compiler_error(format!(
            "Unsupported unary operator in expression typing: {:?}",
            op
        ))
        .into()),
    }
}
