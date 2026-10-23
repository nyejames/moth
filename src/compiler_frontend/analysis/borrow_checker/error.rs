//! Borrow-checker boundary error.
//!
//! WHAT: separates user-facing borrow diagnostic drafts from internal borrow-checker
//! infrastructure failures.
//! WHY: borrow-rule failures stay on the typed diagnostic lane, while invalid HIR metadata or
//! missing compiler side-table facts stay on the internal `CompilerError` path.

use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;

#[derive(Debug)]
pub(crate) enum BorrowCheckError {
    Diagnostic(DiagnosticDraft),
    Infrastructure(Box<CompilerError>),
}

impl BorrowCheckError {
    #[cfg(test)]
    pub(crate) fn into_draft_or_infrastructure(self) -> Result<DiagnosticDraft, CompilerError> {
        match self {
            BorrowCheckError::Diagnostic(diagnostic) => Ok(diagnostic),
            BorrowCheckError::Infrastructure(error) => Err(*error),
        }
    }

    #[cfg(test)]
    pub(crate) fn infrastructure(&self) -> Option<&CompilerError> {
        match self {
            BorrowCheckError::Diagnostic(_) => None,
            BorrowCheckError::Infrastructure(error) => Some(error.as_ref()),
        }
    }
}

impl From<CompilerError> for BorrowCheckError {
    fn from(error: CompilerError) -> Self {
        BorrowCheckError::Infrastructure(Box::new(error))
    }
}

impl From<DiagnosticDraft> for BorrowCheckError {
    fn from(diagnostic: DiagnosticDraft) -> Self {
        BorrowCheckError::Diagnostic(diagnostic)
    }
}
