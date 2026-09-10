//! Error boundary for expression parsing helpers.
//!
//! WHAT: keeps expression-parser user diagnostics as `CompilerDiagnostic` values while still
//! carrying genuine infrastructure failures from deeper AST helpers.
//! WHY: expression parsing is consumed by several AST subsystems. A shared boundary lets each
//! caller decide whether typed source diagnostics or infrastructure failures fit its own stage
//! boundary without routing normal diagnostics through `CompilerError`.

use crate::compiler_frontend::ast::expressions::call_validation::CallValidationError;
use crate::compiler_frontend::ast::expressions::eval_expression::ExpressionTypingError;
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
use crate::compiler_frontend::headers::HeaderParseFailure;

#[derive(Debug)]
pub(crate) enum ExpressionParseError {
    Diagnostic(CompilerDiagnostic),
    Infrastructure(Box<CompilerError>),
}

impl ExpressionParseError {
    /// Returns the contained user-facing diagnostic, or `None` if this is an infrastructure
    /// failure.
    #[cfg(test)]
    pub(super) fn diagnostic(&self) -> Option<&CompilerDiagnostic> {
        match self {
            ExpressionParseError::Diagnostic(diagnostic) => Some(diagnostic),
            ExpressionParseError::Infrastructure(_) => None,
        }
    }
}

impl From<CompilerDiagnostic> for ExpressionParseError {
    fn from(diagnostic: CompilerDiagnostic) -> Self {
        ExpressionParseError::Diagnostic(diagnostic)
    }
}

impl From<HeaderParseFailure> for ExpressionParseError {
    fn from(error: HeaderParseFailure) -> Self {
        match error {
            HeaderParseFailure::Diagnostic(diagnostic) => {
                ExpressionParseError::Diagnostic(diagnostic)
            }
            HeaderParseFailure::Infrastructure(error) => {
                ExpressionParseError::Infrastructure(Box::new(error))
            }
        }
    }
}

impl From<CompilerError> for ExpressionParseError {
    fn from(error: CompilerError) -> Self {
        ExpressionParseError::Infrastructure(Box::new(error))
    }
}

impl From<CallValidationError> for ExpressionParseError {
    fn from(error: CallValidationError) -> Self {
        match error {
            CallValidationError::Diagnostic(diagnostic) => {
                ExpressionParseError::Diagnostic(diagnostic)
            }
            CallValidationError::Infrastructure(error) => {
                ExpressionParseError::Infrastructure(error)
            }
        }
    }
}

impl From<ExpressionTypingError> for ExpressionParseError {
    fn from(error: ExpressionTypingError) -> Self {
        match error {
            ExpressionTypingError::Diagnostic(diagnostic) => {
                ExpressionParseError::Diagnostic(diagnostic)
            }
            ExpressionTypingError::Infrastructure(error) => {
                ExpressionParseError::Infrastructure(error)
            }
        }
    }
}

impl From<TemplateError> for ExpressionParseError {
    fn from(error: TemplateError) -> Self {
        match error {
            TemplateError::Diagnostic(diagnostic) => ExpressionParseError::Diagnostic(diagnostic),
            TemplateError::Infrastructure(error) => ExpressionParseError::Infrastructure(error),
        }
    }
}
