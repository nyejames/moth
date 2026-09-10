//! Error boundary for AST expression type resolution.
//!
//! WHAT: keeps user-facing expression typing diagnostics as `CompilerDiagnostic` values while
//! still allowing this AST slice to report genuine compiler infrastructure failures.
//! WHY: operator policy is a semantic diagnostic owner. It should not wrap normal source errors in
//! `CompilerError`; only the surrounding AST boundary still needs that old return shape.

use crate::compiler_frontend::ast::const_eval::ConstantFoldError;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;

/// Either a user-facing diagnostic or an internal infrastructure failure.
///
/// WHAT: distinguishes normal source-level typing errors from genuine compiler bugs or
///       broken invariants that should never reach the user as diagnostics.
/// WHY: the AST expression evaluator routes both kinds through one `Result` boundary so
///      operator policy and constant folding can stay diagnostic-first without losing the
///      ability to report internal failures.
pub(crate) enum ExpressionTypingError {
    Diagnostic(CompilerDiagnostic),
    Infrastructure(Box<CompilerError>),
}

impl From<CompilerDiagnostic> for ExpressionTypingError {
    fn from(diagnostic: CompilerDiagnostic) -> Self {
        ExpressionTypingError::Diagnostic(diagnostic)
    }
}

impl From<CompilerError> for ExpressionTypingError {
    fn from(error: CompilerError) -> Self {
        ExpressionTypingError::Infrastructure(Box::new(error))
    }
}

impl From<ConstantFoldError> for ExpressionTypingError {
    fn from(error: ConstantFoldError) -> Self {
        match error {
            ConstantFoldError::Diagnostic(diagnostic) => {
                ExpressionTypingError::Diagnostic(diagnostic)
            }
            ConstantFoldError::Infrastructure(error) => {
                ExpressionTypingError::Infrastructure(error)
            }
        }
    }
}
