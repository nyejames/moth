//! Error boundary for AST expression type resolution.
//!
//! WHAT: keeps user-facing expression typing diagnostics as compact drafts while still allowing
//! the AST boundary to report genuine compiler infrastructure failures.
//! WHY: operator policy is a semantic diagnostic owner. It should not wrap normal source errors in
//! `InfrastructureFailure`; only the surrounding AST boundary needs the infrastructure split.

use crate::compiler_frontend::ast::const_eval::ConstantFoldError;
use crate::compiler_frontend::ast::expressions::expression_store::ExpressionConstructionFailure;
use crate::compiler_frontend::compiler_errors::{InfrastructureErrors, InfrastructureFailure};
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;

/// Either a user-facing diagnostic draft or an internal infrastructure failure.
///
/// WHAT: distinguishes normal source-level typing errors from genuine compiler bugs or broken
/// invariants that should never reach the user as diagnostics.
/// WHY: the AST expression evaluator routes both kinds through one `Result` boundary so operator
/// policy and constant folding can stay diagnostic-first without losing infrastructure failures.
pub(crate) enum ExpressionTypingError {
    Diagnostic(DiagnosticDraft),
    Infrastructure(Box<InfrastructureErrors>),
}

impl From<DiagnosticDraft> for ExpressionTypingError {
    fn from(diagnostic: DiagnosticDraft) -> Self {
        Self::Diagnostic(diagnostic)
    }
}

impl From<InfrastructureFailure> for ExpressionTypingError {
    fn from(error: InfrastructureFailure) -> Self {
        ExpressionTypingError::Infrastructure(error.into())
    }
}

impl From<Box<InfrastructureErrors>> for ExpressionTypingError {
    fn from(errors: Box<InfrastructureErrors>) -> Self {
        Self::Infrastructure(errors)
    }
}

impl From<ConstantFoldError> for ExpressionTypingError {
    fn from(error: ConstantFoldError) -> Self {
        match error {
            ConstantFoldError::Diagnostic(diagnostic)
            | ConstantFoldError::NumericFailure(diagnostic) => Self::Diagnostic(diagnostic),
            ConstantFoldError::Infrastructure(error) => Self::Infrastructure(error),
        }
    }
}

impl From<ExpressionConstructionFailure> for ExpressionTypingError {
    fn from(error: ExpressionConstructionFailure) -> Self {
        match error {
            ExpressionConstructionFailure::Diagnostic(diagnostic) => Self::Diagnostic(diagnostic),
            ExpressionConstructionFailure::Invariant(error) => error.into(),
        }
    }
}
