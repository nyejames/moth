//! Shared operator-policy diagnostics.
//!
//! WHAT: constructs `CompilerDiagnostic` values for unsupported operator-type combinations so
//!      comparison, arithmetic, and unary policy files do not duplicate diagnostic creation.
//! WHY: exact operator and operand-type reporting is a single responsibility; keeping it here
//!      ensures consistent error messages and stable diagnostic codes across all operator policies.

use crate::compiler_frontend::ast::expressions::eval_expression::typing_error::ExpressionTypingError;
use crate::compiler_frontend::ast::expressions::expression::Operator;
use crate::compiler_frontend::compiler_errors::SourceLocation;
use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, DiagnosticOperator};
use crate::compiler_frontend::datatypes::ids::TypeId;

/// Maps an AST `Operator` to the diagnostic-owned exact operator at the emission boundary.
///
/// WHAT: converts internal AST operator storage into the diagnostic-layer spelling so the
///      diagnostic payload never depends on the AST enum.
/// WHY: the diagnostic type stays AST-independent while the mapping lives at the one boundary
///      where AST and diagnostic facts meet.
pub(super) fn diagnostic_operator_from_ast(op: &Operator) -> DiagnosticOperator {
    match op {
        Operator::Add => DiagnosticOperator::Add,
        Operator::Subtract => DiagnosticOperator::Subtract,
        Operator::Multiply => DiagnosticOperator::Multiply,
        Operator::Divide => DiagnosticOperator::Divide,
        Operator::IntDivide => DiagnosticOperator::IntDivide,
        Operator::Modulus => DiagnosticOperator::Modulus,
        Operator::Exponent => DiagnosticOperator::Exponent,
        Operator::And => DiagnosticOperator::And,
        Operator::Or => DiagnosticOperator::Or,
        Operator::GreaterThan => DiagnosticOperator::GreaterThan,
        Operator::GreaterThanOrEqual => DiagnosticOperator::GreaterThanOrEqual,
        Operator::LessThan => DiagnosticOperator::LessThan,
        Operator::LessThanOrEqual => DiagnosticOperator::LessThanOrEqual,
        Operator::Equality => DiagnosticOperator::Equality,
        Operator::NotEqual => DiagnosticOperator::NotEqual,
        Operator::Not => DiagnosticOperator::Not,
        // Unary minus shares the `-` spelling with subtraction. It never reaches an
        // unsupported-operator diagnostic because unary policy preserves the operand type.
        Operator::Negate => DiagnosticOperator::Subtract,
        Operator::Range => DiagnosticOperator::Range,
    }
}

pub(super) fn invalid_comparison_types(
    lhs: TypeId,
    rhs: TypeId,
    op: &Operator,
    location: &SourceLocation,
) -> Result<TypeId, ExpressionTypingError> {
    Err(CompilerDiagnostic::unsupported_operator_types(
        diagnostic_operator_from_ast(op),
        lhs,
        Some(rhs),
        location.clone(),
    )
    .into())
}

pub(super) fn invalid_operator_types(
    lhs: TypeId,
    rhs: TypeId,
    op: &Operator,
    location: &SourceLocation,
) -> Result<TypeId, ExpressionTypingError> {
    Err(CompilerDiagnostic::unsupported_operator_types(
        diagnostic_operator_from_ast(op),
        lhs,
        Some(rhs),
        location.clone(),
    )
    .into())
}
