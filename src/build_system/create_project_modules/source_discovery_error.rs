//! Error boundary for Stage 0 source discovery.
//!
//! Source discovery can fail in two different ways:
//! - source-level diagnostics from tokenizing/dependency-clause parsing,
//! - filesystem or tooling failures before a stable source representation exists.
//!
//! Keeping those paths distinct prevents Stage 0 from downgrading typed diagnostics into a lossy
//! syntax bucket while still allowing real file errors to stay on `CompilerError`.

use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages};
use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
use crate::compiler_frontend::paths::dependency_resolution::DependencyPathResolutionError;
use crate::compiler_frontend::symbols::string_interning::StringTable;

/// Stage 0 source discovery failure.
pub(crate) enum SourceDiscoveryError {
    Diagnostic(Box<CompilerDiagnostic>),
    Messages(CompilerMessages),
    Infrastructure(CompilerError),
}

impl SourceDiscoveryError {
    /// Convert the discovery failure into the project boundary message container.
    pub(crate) fn into_messages(self, string_table: &StringTable) -> CompilerMessages {
        match self {
            SourceDiscoveryError::Diagnostic(diagnostic) => {
                CompilerMessages::from_diagnostics(vec![*diagnostic], string_table.clone())
            }
            SourceDiscoveryError::Messages(messages) => messages,
            SourceDiscoveryError::Infrastructure(error) => {
                CompilerMessages::from_error_ref(error, string_table)
            }
        }
    }
}

impl From<CompilerDiagnostic> for SourceDiscoveryError {
    fn from(diagnostic: CompilerDiagnostic) -> Self {
        SourceDiscoveryError::Diagnostic(Box::new(diagnostic))
    }
}

impl From<CompilerError> for SourceDiscoveryError {
    fn from(error: CompilerError) -> Self {
        SourceDiscoveryError::Infrastructure(error)
    }
}

impl From<CompilerMessages> for SourceDiscoveryError {
    fn from(messages: CompilerMessages) -> Self {
        SourceDiscoveryError::Messages(messages)
    }
}

impl From<DependencyPathResolutionError> for SourceDiscoveryError {
    fn from(error: DependencyPathResolutionError) -> Self {
        match error {
            DependencyPathResolutionError::Diagnostic(diagnostic) => {
                SourceDiscoveryError::Diagnostic(diagnostic)
            }
            DependencyPathResolutionError::Infrastructure(error) => {
                SourceDiscoveryError::Infrastructure(error)
            }
        }
    }
}
