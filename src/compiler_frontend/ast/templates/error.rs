//! Local template error boundary.
//!
//! WHAT: keeps formatter and template-owned source diagnostics typed while template helpers still
//! expose a mix of `CompilerDiagnostic` and older `CompilerError` entrypoints.
//! WHY: template construction and folding sit between AST source diagnostics and project-aware
//! formatting/folding infrastructure. This boundary makes that distinction explicit locally.

use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::templates::template_slots::TemplateSlotError;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
use crate::compiler_frontend::paths::compile_time_paths::CompileTimePathResolutionError;

#[derive(Debug)]
pub(crate) enum TemplateError {
    Diagnostic(Box<CompilerDiagnostic>),
    Infrastructure(Box<CompilerError>),
}

impl TemplateError {
    /// Rewrite a source diagnostic at a directive-specific boundary without collapsing an
    /// infrastructure failure into the user-facing lane.
    pub(crate) fn map_diagnostic(
        self,
        map: impl FnOnce(Box<CompilerDiagnostic>) -> Box<CompilerDiagnostic>,
    ) -> Self {
        match self {
            TemplateError::Diagnostic(diagnostic) => TemplateError::Diagnostic(map(diagnostic)),
            TemplateError::Infrastructure(error) => TemplateError::Infrastructure(error),
        }
    }
}

impl From<CompilerDiagnostic> for TemplateError {
    fn from(diagnostic: CompilerDiagnostic) -> Self {
        TemplateError::Diagnostic(Box::new(diagnostic))
    }
}

impl From<Box<CompilerDiagnostic>> for TemplateError {
    fn from(diagnostic: Box<CompilerDiagnostic>) -> Self {
        TemplateError::Diagnostic(diagnostic)
    }
}

impl From<CompilerError> for TemplateError {
    fn from(error: CompilerError) -> Self {
        TemplateError::Infrastructure(Box::new(error))
    }
}

impl From<CompileTimePathResolutionError> for TemplateError {
    fn from(error: CompileTimePathResolutionError) -> Self {
        match error {
            CompileTimePathResolutionError::Diagnostic(diagnostic) => {
                TemplateError::Diagnostic(diagnostic)
            }
            CompileTimePathResolutionError::Infrastructure(error) => {
                TemplateError::Infrastructure(Box::new(error))
            }
        }
    }
}

/// Template control-flow headers reuse ordinary expression and loop-header parsing. Preserve the
/// original lane when that shared parser reports an internal retained-data lifecycle failure.
impl From<ExpressionParseError> for TemplateError {
    fn from(error: ExpressionParseError) -> Self {
        match error {
            ExpressionParseError::Diagnostic(diagnostic) => TemplateError::Diagnostic(diagnostic),
            ExpressionParseError::Infrastructure(error) => TemplateError::Infrastructure(error),
        }
    }
}

impl From<TemplateSlotError> for TemplateError {
    fn from(error: TemplateSlotError) -> Self {
        match error {
            TemplateSlotError::Diagnostic(diagnostic) => TemplateError::Diagnostic(diagnostic),
            TemplateSlotError::Infrastructure(error) => TemplateError::Infrastructure(error),
        }
    }
}
