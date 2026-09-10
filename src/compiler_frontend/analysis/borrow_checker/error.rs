//! Borrow-checker boundary error.
//!
//! WHAT: separates user-facing borrow diagnostics from internal borrow-checker infrastructure
//! failures.
//! WHY: borrow-rule failures should remain typed `CompilerDiagnostic` values, while invalid HIR
//! metadata or missing compiler side-table facts should stay on the internal `CompilerError` path.

use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;

#[derive(Debug, Clone)]
pub(crate) enum BorrowCheckError {
    Diagnostic(CompilerDiagnostic),
    Infrastructure(Box<CompilerError>),
}

impl BorrowCheckError {
    #[cfg(test)]
    pub(crate) fn into_diagnostic_or_infrastructure(
        self,
    ) -> Result<CompilerDiagnostic, CompilerError> {
        match self {
            BorrowCheckError::Diagnostic(diagnostic) => Ok(diagnostic),
            BorrowCheckError::Infrastructure(error) => Err(*error),
        }
    }

    #[cfg(test)]
    pub(crate) fn diagnostic(&self) -> Option<&CompilerDiagnostic> {
        match self {
            BorrowCheckError::Diagnostic(diagnostic) => Some(diagnostic),
            BorrowCheckError::Infrastructure(_) => None,
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

impl From<CompilerDiagnostic> for BorrowCheckError {
    fn from(diagnostic: CompilerDiagnostic) -> Self {
        BorrowCheckError::Diagnostic(diagnostic)
    }
}
