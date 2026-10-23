//! Local template error boundary.
//!
//! WHAT: keeps formatter and template-owned source diagnostics typed while infrastructure failures
//! remain on the compiler-error lane.
//! WHY: template construction and folding sit between AST source diagnostics and project-aware
//! formatting/folding infrastructure. This boundary keeps that distinction explicit locally.

use crate::compiler_frontend::ast::const_eval::ConstantFoldError;
use crate::compiler_frontend::ast::const_values::store::ConstValueStoreError;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression_store::ExpressionConstructionFailure;
use crate::compiler_frontend::compiler_errors::{CompilerError, InfrastructureErrors};
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;

#[derive(Debug)]
pub(crate) enum TemplateError {
    Diagnostic(DiagnosticDraft),
    EmptyArguments(DiagnosticDraft),
    NonConstantReferenceInConstant(DiagnosticDraft),
    Infrastructure(Box<InfrastructureErrors>),
}

impl TemplateError {
    /// Rewrite a source diagnostic at a directive-specific boundary without collapsing an
    /// infrastructure failure into the user-facing lane.
    pub(crate) fn map_diagnostic(
        self,
        map: impl FnOnce(DiagnosticDraft) -> DiagnosticDraft,
    ) -> Self {
        match self {
            Self::Diagnostic(diagnostic) => Self::Diagnostic(map(diagnostic)),
            Self::EmptyArguments(diagnostic) => Self::EmptyArguments(map(diagnostic)),
            Self::NonConstantReferenceInConstant(diagnostic) => {
                Self::NonConstantReferenceInConstant(map(diagnostic))
            }
            Self::Infrastructure(error) => Self::Infrastructure(error),
        }
    }
}

impl From<DiagnosticDraft> for TemplateError {
    fn from(diagnostic: DiagnosticDraft) -> Self {
        TemplateError::Diagnostic(diagnostic)
    }
}

impl From<CompilerError> for TemplateError {
    fn from(error: CompilerError) -> Self {
        TemplateError::Infrastructure(error.into())
    }
}

impl From<Box<InfrastructureErrors>> for TemplateError {
    fn from(errors: Box<InfrastructureErrors>) -> Self {
        Self::Infrastructure(errors)
    }
}

/// Template control-flow headers reuse ordinary expression and loop-header parsing. Preserve the
/// original lane when that shared parser reports an internal retained-data lifecycle failure.
impl From<ExpressionParseError> for TemplateError {
    fn from(error: ExpressionParseError) -> Self {
        match error {
            ExpressionParseError::Diagnostic(diagnostic) => TemplateError::Diagnostic(diagnostic),
            ExpressionParseError::TemplateDirectiveEmptyArguments(diagnostic) => {
                TemplateError::EmptyArguments(diagnostic)
            }
            ExpressionParseError::NonConstantReferenceInConstant(diagnostic) => {
                TemplateError::NonConstantReferenceInConstant(diagnostic)
            }
            ExpressionParseError::Infrastructure(error) => TemplateError::Infrastructure(error),
            // A numeric-dependent fragment is retained only by the boundary that owns the whole
            // authored position: template head positions retain through the head expression
            // funnel, and control-flow suffix headers retain through the template body parser's
            // nested-construct producers. A deferral escaping to this conversion therefore means
            // the position was never handed to an owning boundary, so it must surface as an
            // internal failure instead of becoming a fabricated value.
            ExpressionParseError::NumericDeferral(_) => TemplateError::Infrastructure(
                CompilerError::compiler_error(
                    "numeric-dependent expression reached a template boundary that does not \
                     retain it; template head positions and control-flow suffix headers must \
                     consume deferrals through their owning boundary",
                )
                .into(),
            ),
        }
    }
}

impl From<ExpressionConstructionFailure> for TemplateError {
    fn from(error: ExpressionConstructionFailure) -> Self {
        match error {
            ExpressionConstructionFailure::Diagnostic(diagnostic) => Self::Diagnostic(diagnostic),
            ExpressionConstructionFailure::Invariant(error) => error.into(),
        }
    }
}

impl From<ConstValueStoreError> for TemplateError {
    fn from(error: ConstValueStoreError) -> Self {
        match error {
            ConstValueStoreError::Diagnostic(diagnostic) => Self::Diagnostic(diagnostic),
            ConstValueStoreError::Infrastructure(error) => Self::Infrastructure(error),
        }
    }
}

impl From<ConstantFoldError> for TemplateError {
    fn from(error: ConstantFoldError) -> Self {
        match error {
            ConstantFoldError::Diagnostic(diagnostic)
            | ConstantFoldError::NumericFailure(diagnostic) => Self::Diagnostic(diagnostic),
            ConstantFoldError::Infrastructure(error) => Self::Infrastructure(error),
        }
    }
}
