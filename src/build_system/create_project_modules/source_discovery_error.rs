//! Error boundary for Stage 0 source discovery.
//!
//! Source discovery can fail in two different ways:
//! - source-level diagnostics from tokenizing/dependency-clause parsing,
//! - filesystem or tooling failures before a stable source representation exists.
//!
//! Keeping those paths distinct prevents Stage 0 from downgrading typed diagnostics into a lossy
//! syntax bucket while still allowing real file errors to stay on `InfrastructureFailure`.
//!
//! Finalized failures additionally preserve the finished [`SourceDatabase`] beside the typed
//! [`PreparedFailure`]. The final boundary appends that failure with its source owner directly to the
//! accumulator and freezes the report once.

use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::families::token_lexical;
use crate::compiler_frontend::compiler_messages::prepared_batch::PreparedDiagnosticBatch;
use crate::compiler_frontend::compiler_messages::{DiagnosticBag, PreparedFailure};
use crate::compiler_frontend::headers::parse_file_headers::{
    FileFrontendPrepareError, FileFrontendPrepareFailure,
};
use crate::compiler_frontend::paths::dependency_resolution::DependencyPathResolutionError;
use crate::compiler_frontend::source::{SourceCapacityError, SourceDatabase, SourceDatabaseError};
use crate::compiler_frontend::symbols::path_interner::{PathInternerFork, PathTable};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use std::sync::Arc;
/// Finalized Stage 0 failure preserving the finished source owner.
///
/// WHAT: carries the typed premerge failure beside the finished source database that was
///       finalized before the failure escaped.
/// WHY: discovery finalization moves every retained snapshot and original span builder into
///      the final owner before any fallible identity transformation, so a terminal failure
///      must retain that owner until it appends the prepared failure to the source-aware
///      accumulator and freezes the report once.
#[derive(Debug)]
pub(crate) struct FinalizedDiscoveryFailure {
    pub(crate) failure: PreparedFailure,
    pub(crate) source_database: SourceDatabase,
}

impl FinalizedDiscoveryFailure {
    pub(crate) fn new(failure: PreparedFailure, source_database: SourceDatabase) -> Self {
        Self {
            failure,
            source_database,
        }
    }

    pub(crate) fn into_parts(self) -> (PreparedFailure, SourceDatabase) {
        (self.failure, self.source_database)
    }
}

/// Stage 0 source discovery failure, classified by how its boundary must report it.
///
/// WHAT: a typed diagnostic draft, a premerge failure captured by its producer, a finalized
///       discovery owner, or an infrastructure failure.
/// WHY: each variant has one reporting path, so a typed diagnostic is never downgraded into
///      infrastructure.
#[derive(Debug)]
pub(crate) enum SourceDiscoveryError {
    Diagnostic(DiagnosticDraft),
    /// Diagnostics captured by their producer, including external import providers.
    Premerge(PreparedFailure),
    Finalized(Box<FinalizedDiscoveryFailure>),
    Infrastructure(InfrastructureFailure),
}

impl SourceDiscoveryError {
    /// Finalized constructor for failures that already own a finished source database.
    pub(crate) fn finalized(failure: PreparedFailure, source_database: SourceDatabase) -> Self {
        SourceDiscoveryError::Finalized(Box::new(FinalizedDiscoveryFailure::new(
            failure,
            source_database,
        )))
    }

    /// Convert the discovery failure into the premerge lane without freezing a report.
    ///
    /// WHAT: moves a diagnostic plus the caller's local table into a prepared batch, or
    ///       propagates a prepared failure or infrastructure error typed. The table moves via
    ///       `mem::take`; no clone carries diagnostics. Required path identities retain their
    ///       issuing path snapshot with the prepared batch.
    /// WHY: inventory preparation aborts on the first discovery failure and discards its
    ///      discovery owner, so the batch becomes the sole table owner. The finalized boundary
    ///      later pairs that failure with its source row and freezes one report.
    /// A finalized failure already owns a finished source database this lane cannot carry:
    /// [`PreparedFailure`] has no source owner, so converting here would drop the finished owner.
    /// Finalized failures must be matched explicitly and split with
    /// [`FinalizedDiscoveryFailure::into_parts`] (see `collect_reachable_input_files`);
    /// reaching this converter with one is a caller bug and fails loudly in all builds
    /// instead of dropping the finished owner.
    pub(crate) fn into_failure(
        self,
        string_table: &mut StringTable,
        path_table: Arc<PathTable>,
    ) -> PreparedFailure {
        match self {
            SourceDiscoveryError::Diagnostic(diagnostic) => {
                let table = string_table.take_for_handoff();
                match PreparedDiagnosticBatch::capture_with_required_paths(
                    DiagnosticBag::from(diagnostic),
                    table,
                    None,
                    || Some(path_table),
                ) {
                    Ok(batch) => PreparedFailure::Diagnosed(batch),
                    Err(error) => PreparedFailure::Infrastructure(error.into()),
                }
            }
            SourceDiscoveryError::Premerge(failure) => failure,
            SourceDiscoveryError::Finalized(_) => {
                crate::compiler_bug!(
                    CompilerStage::ProjectBuildAssembly,
                    "finalized discovery failures own a finished SourceDatabase; match Finalized \
                     explicitly instead of converting through into_failure"
                )
            }
            SourceDiscoveryError::Infrastructure(error) => {
                PreparedFailure::Infrastructure(error.into())
            }
        }
    }

    /// Admit this failure into its source owner's slot.
    ///
    /// The owner's first failure is kept. A later diagnostic is dropped before capture, so it
    /// never takes the owner's string or path table; a later infrastructure cause merges and
    /// dominates. Without an owner the failure is captured once, as in [`Self::into_failure`].
    pub(crate) fn admit_into_owner(
        self,
        owner: Option<PreparedFailure>,
        string_table: &mut StringTable,
        path_fork: &PathInternerFork,
    ) -> PreparedFailure {
        let Some(owner) = owner else {
            return self.into_failure(string_table, Arc::new(path_fork.shared_table()));
        };
        match self {
            SourceDiscoveryError::Diagnostic(_) => owner,
            SourceDiscoveryError::Premerge(later) => owner.with_later(later),
            SourceDiscoveryError::Infrastructure(error) => owner.with_infrastructure_cause(error),
            finalized @ SourceDiscoveryError::Finalized(_) => {
                finalized.into_failure(string_table, Arc::new(path_fork.shared_table()))
            }
        }
    }
}
impl From<DiagnosticDraft> for SourceDiscoveryError {
    fn from(diagnostic: DiagnosticDraft) -> Self {
        SourceDiscoveryError::Diagnostic(diagnostic)
    }
}

impl From<InfrastructureFailure> for SourceDiscoveryError {
    fn from(error: InfrastructureFailure) -> Self {
        SourceDiscoveryError::Infrastructure(error)
    }
}

/// WHAT: splits one source-database error into a path-less capacity diagnostic or the typed
///       infrastructure failure, for both conversions below.
/// WHY: an authored snapshot over the size limit must name its user file. Only
///      `retained_source_error` knows that logical path, so every authored retention site converts
///      through it; reaching the snapshot arm here is a compiler bug.
fn classify_source_database_error(
    error: SourceDatabaseError,
) -> Result<DiagnosticDraft, InfrastructureFailure> {
    match error {
        SourceDatabaseError::Capacity(SourceCapacityError::SourceSnapshotTooLarge) => {
            crate::compiler_bug!(
                CompilerStage::SourceLoading,
                "an oversized authored source must be named by retained_source_error"
            )
        }
        SourceDatabaseError::Capacity(capacity) => Ok(token_lexical::source_span_capacity(
            0,
            u32::MAX,
            capacity.resource(),
            None,
        )),
        SourceDatabaseError::Infrastructure(error) => Err(error),
    }
}

impl From<SourceDatabaseError> for SourceDiscoveryError {
    fn from(error: SourceDatabaseError) -> Self {
        match classify_source_database_error(error) {
            Ok(diagnostic) => SourceDiscoveryError::Diagnostic(diagnostic),
            Err(error) => SourceDiscoveryError::Infrastructure(error),
        }
    }
}

impl From<SourceDatabaseError> for PreparedFailure {
    fn from(error: SourceDatabaseError) -> Self {
        match classify_source_database_error(error) {
            Ok(diagnostic) => PreparedFailure::from(diagnostic),
            Err(error) => PreparedFailure::from(error),
        }
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

/// Maps one discovery failure onto the file-preparation failure lane.
///
/// WHAT: a typed diagnostic draft becomes a `Diagnosed` failure with no warnings; infrastructure
///       passes through unchanged.
/// WHY: the template bundle and the module worker both prepare files through this lane, so an
///      authored read failure reaches the same diagnostic from either caller. Premerge and
///      finalized failures never reach a file-preparation boundary.
pub(crate) fn file_prepare_failure_from_discovery(
    error: SourceDiscoveryError,
) -> FileFrontendPrepareFailure {
    match error {
        SourceDiscoveryError::Diagnostic(diagnostic) => {
            FileFrontendPrepareFailure::Diagnosed(FileFrontendPrepareError {
                warnings: DiagnosticBag::new(),
                diagnostic,
            })
        }
        SourceDiscoveryError::Infrastructure(error) => {
            FileFrontendPrepareFailure::Infrastructure(error)
        }
        SourceDiscoveryError::Premerge(_) | SourceDiscoveryError::Finalized(_) => {
            crate::compiler_bug!(
                CompilerStage::ProjectBuildAssembly,
                "source load cannot produce a premerge or finalized discovery failure"
            )
        }
    }
}
