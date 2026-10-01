//! Unary operator typing policy.
//!
//! WHAT: resolves result types for unary logical not and numeric negation.
//! WHY: AST expression evaluation must enforce operand validity and select the semantic negation
//!      domain before HIR lowering.

use crate::compiler_frontend::ast::expressions::eval_expression::typing_error::ExpressionTypingError;
use crate::compiler_frontend::ast::expressions::expression::Operator;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::datatypes::numeric_operators::negation_domain;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::source::SourceSpan;

use super::diagnostics::diagnostic_operator_from_ast;

/// Resolve the result type of a unary operator application.
///
/// `not` requires a boolean operand and returns `Bool`. Unary minus follows the shared numeric
/// negation policy, whose domain can widen signed narrow integers and fixed binary floats.
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

        Operator::Negate => {
            let domain =
                NumericScalar::from_type_id(operand, type_environment).and_then(negation_domain);

            if let Some(domain) = domain {
                Ok(domain.type_id(type_environment))
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
