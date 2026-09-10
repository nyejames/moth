//! Error boundary for Stage 0 source discovery.
//!
//! Source discovery can fail in two different ways:
//! - source-level diagnostics from tokenizing/dependency-clause parsing,
//! - filesystem or tooling failures before a stable source representation exists.
//!
//! Keeping those paths distinct prevents Stage 0 from downgrading typed diagnostics into a lossy
//! syntax bucket while still allowing real file errors to stay on `CompilerError`.
//!
//! Finalized failures additionally preserve the finished [`SourceDatabase`] beside the typed
//! [`PremergeFailure`] so the single-file final boundary can convert once with source context
//! attached. Production discovery never constructs [`CompilerMessages`]; that vessel is built
//! exactly once by the final render-test tails and the parent single-file boundary.

use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, PremergeDiagnosticBatch, PremergeFailure,
};
use crate::compiler_frontend::paths::dependency_resolution::DependencyPathResolutionError;
use crate::compiler_frontend::source::SourceDatabase;
use crate::compiler_frontend::symbols::string_interning::StringTable;
/// Finalized Stage 0 failure preserving the finished source owner.
///
/// WHAT: carries the typed premerge failure beside the finished source database that was
///       finalized before the failure escaped.
/// WHY: discovery finalization moves every retained snapshot and original span builder into
///      the final owner before any fallible identity transformation, so a terminal failure
///      must not drop that owner. The parent single-file/test tail converts the inner
///      [`PremergeFailure`] once and attaches this database as the render source context.
#[derive(Debug)]
pub(crate) struct FinalizedDiscoveryFailure {
    pub(crate) failure: PremergeFailure,
    pub(crate) source_database: SourceDatabase,
}

impl FinalizedDiscoveryFailure {
    pub(crate) fn new(failure: PremergeFailure, source_database: SourceDatabase) -> Self {
        Self {
            failure,
            source_database,
        }
    }

    pub(crate) fn into_parts(self) -> (PremergeFailure, SourceDatabase) {
        (self.failure, self.source_database)
    }
}

/// Stage 0 source discovery failure.
pub(crate) enum SourceDiscoveryError {
    Diagnostic(CompilerDiagnostic),
    Premerge(PremergeFailure),
    Finalized(Box<FinalizedDiscoveryFailure>),
    Infrastructure(CompilerError),
}

impl SourceDiscoveryError {
    /// Finalized constructor for failures that already own a finished source database.
    pub(crate) fn finalized(failure: PremergeFailure, source_database: SourceDatabase) -> Self {
        SourceDiscoveryError::Finalized(Box::new(FinalizedDiscoveryFailure::new(
            failure,
            source_database,
        )))
    }

    /// Convert the discovery failure into the premerge lane without an intermediate vessel.
    ///
    /// WHAT: moves a diagnostic plus the caller's local table into a diagnosed batch, or
    ///       propagates an infrastructure error typed. The table moves via `mem::take`; no
    ///       clone carries diagnostics.
    /// WHY: inventory preparation aborts on the first discovery failure and discards its
    ///      discovery owner, so the batch becomes the sole table owner. The final boundary owns
    ///      the single vessel conversion.
    ///
    /// A finalized failure already owns a finished source database this lane cannot carry:
    /// [`PremergeFailure`] has no source owner, so converting here would drop the finished
    /// database. Finalized failures must be matched explicitly and split with
    /// [`FinalizedDiscoveryFailure::into_parts`] (see `collect_reachable_input_files`);
    /// reaching this converter with one is a caller bug and fails loudly in all builds
    /// instead of dropping the finished owner.
    pub(crate) fn into_failure(self, string_table: &mut StringTable) -> PremergeFailure {
        match self {
            SourceDiscoveryError::Diagnostic(diagnostic) => {
                let table = std::mem::take(string_table);
                PremergeFailure::Diagnosed(PremergeDiagnosticBatch::from_diagnostic(
                    diagnostic, table,
                ))
            }
            SourceDiscoveryError::Premerge(failure) => failure,
            SourceDiscoveryError::Finalized(_) => {
                unreachable!(
                    "finalized discovery failures own a finished SourceDatabase; match Finalized \
                     explicitly instead of converting through into_failure"
                )
            }
            SourceDiscoveryError::Infrastructure(error) => PremergeFailure::Infrastructure(error),
        }
    }
}

impl From<CompilerDiagnostic> for SourceDiscoveryError {
    fn from(diagnostic: CompilerDiagnostic) -> Self {
        SourceDiscoveryError::Diagnostic(diagnostic)
    }
}

impl From<CompilerError> for SourceDiscoveryError {
    fn from(error: CompilerError) -> Self {
        SourceDiscoveryError::Infrastructure(error)
    }
}

impl From<CompilerMessages> for SourceDiscoveryError {
    fn from(messages: CompilerMessages) -> Self {
        SourceDiscoveryError::Premerge(PremergeFailure::from(messages))
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
