//! Local template error boundary.
//!
//! WHAT: keeps formatter and template-owned source diagnostics typed while infrastructure failures
//! remain on the compiler-error lane.
//! WHY: template construction and folding sit between AST source diagnostics and project-aware
//! formatting/folding infrastructure. This boundary keeps that distinction explicit locally.

use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;

#[derive(Debug)]
pub(crate) enum TemplateError {
    Diagnostic(CompilerDiagnostic),
    Infrastructure(Box<CompilerError>),
}

impl TemplateError {
    /// Rewrite a source diagnostic at a directive-specific boundary without collapsing an
    /// infrastructure failure into the user-facing lane.
    pub(crate) fn map_diagnostic(
        self,
        map: impl FnOnce(CompilerDiagnostic) -> CompilerDiagnostic,
    ) -> Self {
        match self {
            TemplateError::Diagnostic(diagnostic) => TemplateError::Diagnostic(map(diagnostic)),
            TemplateError::Infrastructure(error) => TemplateError::Infrastructure(error),
        }
    }
}

impl From<CompilerDiagnostic> for TemplateError {
    fn from(diagnostic: CompilerDiagnostic) -> Self {
        TemplateError::Diagnostic(diagnostic)
    }
}

impl From<CompilerError> for TemplateError {
    fn from(error: CompilerError) -> Self {
        TemplateError::Infrastructure(Box::new(error))
    }
}

/// Template control-flow headers reuse ordinary expression and loop-header parsing. Preserve the
/// original lane when that shared parser reports an internal retained-data lifecycle failure.
impl From<ExpressionParseError> for TemplateError {
    fn from(error: ExpressionParseError) -> Self {
        match error {
            ExpressionParseError::Diagnostic(diagnostic) => TemplateError::Diagnostic(diagnostic),
            ExpressionParseError::Infrastructure(error) => TemplateError::Infrastructure(error),
            // A numeric-dependent fragment is retained only by the boundary that owns the whole
            // authored position: template head positions retain through the head expression
            // funnel, and control-flow suffix headers retain through the template body parser's
            // nested-construct producers. A deferral escaping to this conversion therefore means
            // the position was never handed to an owning boundary, so it must surface as an
            // internal failure instead of becoming a fabricated value.
            ExpressionParseError::NumericDeferral(_) => {
                TemplateError::Infrastructure(Box::new(CompilerError::compiler_error(
                    "numeric-dependent expression reached a template boundary that does not \
                     retain it; template head positions and control-flow suffix headers must \
                     consume deferrals through their owning boundary",
                )))
            }
        }
    }
}
