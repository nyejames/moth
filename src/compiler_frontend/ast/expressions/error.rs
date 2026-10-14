//! Error boundary for expression parsing helpers.
//!
//! WHAT: keeps expression-parser user diagnostics as `CompilerDiagnostic` values while still
//! carrying genuine infrastructure failures from deeper AST helpers.
//! WHY: expression parsing is consumed by several AST subsystems. A shared boundary lets each
//! caller decide whether typed source diagnostics or infrastructure failures fit its own stage
//! boundary without routing normal diagnostics through `CompilerError`.

use crate::compiler_frontend::ast::const_values::ConstValueStoreError;
use crate::compiler_frontend::ast::expressions::call_validation::CallValidationError;
use crate::compiler_frontend::ast::expressions::eval_expression::ExpressionTypingError;
use crate::compiler_frontend::ast::expressions::expression_rpn::ExpressionRpnItem;
use crate::compiler_frontend::ast::expressions::expression_store::ExpressionConstructionFailure;
use crate::compiler_frontend::ast::numeric_construction::DeferredExpressionSuffix;
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
use crate::compiler_frontend::headers::HeaderParseFailure;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::type_coercion::parse_context::ExpectedType;
use crate::compiler_frontend::value_mode::ValueMode;

/// One numeric-dependent expression position whose fragment completes at the body root.
///
/// WHAT: the complete ordered fragment of the position (still containing construction-only
///       numeric items), the exact receiving context that owned it, and whether the fragment is
///       the position's entire authored syntax.
/// WHY: numeric origins are selected from every permitted use in the body, so a use that cannot
///      be finished at parse time must retain its own RPN and resume in place. `complete_fragment`
///      is false when a nested parse swallowed structure (a receiver chain, a constructor field),
///      which body completion must reject rather than treat as a whole statement.
#[derive(Debug)]
pub(crate) struct NumericDeferral {
    pub(crate) items: Vec<ExpressionRpnItem>,
    pub(crate) expected: ExpectedType,
    pub(crate) value_mode: ValueMode,
    pub(crate) span: Option<SourceSpan>,
    pub(crate) suffix: DeferredExpressionSuffix,
    pub(crate) complete_fragment: bool,
}

#[derive(Debug)]
pub(crate) enum ExpressionParseError {
    Diagnostic(CompilerDiagnostic),
    Infrastructure(Box<CompilerError>),
    /// A numeric-dependent expression fragment only body completion can finish.
    NumericDeferral(Box<NumericDeferral>),
}

impl From<CompilerDiagnostic> for ExpressionParseError {
    fn from(diagnostic: CompilerDiagnostic) -> Self {
        ExpressionParseError::Diagnostic(diagnostic)
    }
}

impl From<ExpressionConstructionFailure> for ExpressionParseError {
    fn from(failure: ExpressionConstructionFailure) -> Self {
        match failure {
            ExpressionConstructionFailure::Diagnostic(diagnostic) => Self::Diagnostic(diagnostic),
            ExpressionConstructionFailure::Invariant(error) => {
                Self::Infrastructure(Box::new(error))
            }
        }
    }
}

impl From<ConstValueStoreError> for ExpressionParseError {
    fn from(failure: ConstValueStoreError) -> Self {
        match failure {
            ConstValueStoreError::Diagnostic(diagnostic) => Self::Diagnostic(diagnostic),
            ConstValueStoreError::Infrastructure(error) => Self::Infrastructure(error),
        }
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

impl ExpressionParseError {
    /// Take a complete numeric deferral, or convert it into a precise internal failure.
    ///
    /// WHAT: returns the retained fragment for a consumer that can splice it verbatim, and
    ///       reports an unsupported position when the fragment is not the position's complete
    ///       syntax. Every other outcome passes through unchanged.
    /// WHY: a nested parse that swallowed enclosing structure must never look like a complete
    ///      statement fragment; body completion must reject it rather than silently drop the
    ///      swallowed syntax.
    pub(crate) fn take_numeric_deferral(
        self,
        position: &str,
    ) -> Result<NumericDeferral, ExpressionParseError> {
        match self {
            ExpressionParseError::NumericDeferral(deferral) if deferral.complete_fragment => {
                Ok(*deferral)
            }
            ExpressionParseError::NumericDeferral(_) => Err(ExpressionParseError::Infrastructure(
                Box::new(CompilerError::compiler_error(format!(
                    "numeric-dependent expression at {position} cannot be retained: the \
                         enclosing construct does not support body-root numeric completion"
                ))),
            )),
            other => Err(other),
        }
    }

    /// Split one failure at a boundary that owns no numeric retention or completion.
    ///
    /// WHAT: an authored diagnostic stays a diagnostic and an infrastructure failure stays an
    ///       infrastructure failure; a leaked numeric deferral becomes a precise infrastructure
    ///       failure naming the boundary.
    /// WHY: only a body root (or a lane that retains the fragment for it) may observe a numeric
    ///       deferral, so any other boundary has a broken handoff. It must report that invariant
    ///       instead of downgrading the failure to a source diagnostic or dropping retained
    ///       syntax.
    pub(crate) fn at_boundary_without_numeric_retention(
        self,
        boundary: &str,
    ) -> Result<CompilerDiagnostic, CompilerError> {
        match self {
            ExpressionParseError::Diagnostic(diagnostic) => Ok(diagnostic),
            ExpressionParseError::Infrastructure(error) => Err(*error),
            ExpressionParseError::NumericDeferral(_) => {
                Err(CompilerError::compiler_error(format!(
                    "a numeric-dependent expression fragment reached {boundary}, which cannot retain \
                 or complete it before the body root"
                )))
            }
        }
    }

    /// Reject an unresolved numeric fragment at a boundary that cannot retain it.
    ///
    /// WHAT: the same conversion as [`ExpressionParseError::at_boundary_without_numeric_retention`],
    ///       returned in the parse-error lane for direct rejection sites.
    /// WHY: statement and declaration boundaries propagate `ExpressionParseError`; they need the
    ///      precise invariant failure without re-deriving the two-lane split.
    pub(crate) fn reject_at_boundary_without_numeric_retention(
        self,
        boundary: &str,
    ) -> ExpressionParseError {
        match self.at_boundary_without_numeric_retention(boundary) {
            Ok(diagnostic) => ExpressionParseError::Diagnostic(diagnostic),
            Err(error) => ExpressionParseError::Infrastructure(Box::new(error)),
        }
    }
}
