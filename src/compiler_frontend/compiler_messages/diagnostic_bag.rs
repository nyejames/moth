//! Local diagnostic accumulator for owning-site capture.
//!
//! WHAT: `DiagnosticBag` stores diagnostics emitted by a stage before its owner captures them in
//! a `PreparedDiagnosticBatch`.
//! WHY: stages accumulate diagnostics without constructing a boundary vessel; the prepared
//! batch owns frozen diagnostic facts and the identity tables needed by the final boundary.

use super::compiler_errors::{CompilerError, InfrastructureErrors};
use super::diagnostics::draft::DiagnosticDraft;
use super::prepared_batch::PreparedDiagnosticBatch;
use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, DiagnosticSeverity};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::symbols::path_interner::{PathIdRemap, PathTable};
use crate::compiler_frontend::symbols::string_interning::{StringIdRemap, StringTable};
use std::sync::Arc;

#[derive(Debug, Default)]
pub(crate) struct DiagnosticBag {
    diagnostics: Vec<DiagnosticDraft>,
}

impl DiagnosticBag {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn from_diagnostics<D>(diagnostics: Vec<D>) -> Self
    where
        D: Into<DiagnosticDraft>,
    {
        Self {
            diagnostics: diagnostics.into_iter().map(Into::into).collect(),
        }
    }

    pub(crate) fn push(&mut self, diagnostic: impl Into<DiagnosticDraft>) {
        self.diagnostics.push(diagnostic.into());
    }

    pub(crate) fn append(&mut self, mut other: Self) {
        self.diagnostics.append(&mut other.diagnostics);
    }

    pub(crate) fn extend(&mut self, diagnostics: impl IntoIterator<Item = DiagnosticDraft>) {
        self.diagnostics.extend(diagnostics);
    }
    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.diagnostics.is_empty()
    }

    pub(crate) fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity() == DiagnosticSeverity::Error)
    }

    #[cfg(test)]
    pub(crate) fn as_slice(&self) -> &[DiagnosticDraft] {
        &self.diagnostics
    }

    pub(crate) fn iter_mut(&mut self) -> impl Iterator<Item = &mut DiagnosticDraft> {
        self.diagnostics.iter_mut()
    }

    pub(crate) fn remap_path_ids(&mut self, remap: &PathIdRemap) {
        for diagnostic in &mut self.diagnostics {
            diagnostic.remap_path_ids(remap);
        }
    }

    pub(crate) fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        for diagnostic in &mut self.diagnostics {
            diagnostic.remap_string_ids(remap);
        }
    }

    pub(crate) fn take_diagnostics(&mut self) -> Vec<DiagnosticDraft> {
        std::mem::take(&mut self.diagnostics)
    }

    pub(crate) fn into_diagnostics(self) -> Vec<DiagnosticDraft> {
        self.diagnostics
    }
}

impl<D> From<D> for DiagnosticBag
where
    D: Into<DiagnosticDraft>,
{
    fn from(diagnostic: D) -> Self {
        let mut bag = Self::new();
        bag.push(diagnostic);
        bag
    }
}

/// One pre-freeze failure lane for diagnosed and infrastructure outcomes.
///
/// WHAT: carries prepared diagnostic facts, owned infrastructure errors, or both when source
/// finalization races a semantic failure.
/// WHY: owning sites capture diagnostics before they leave the type/path context that produced
/// them; only the final boundary assembles source ownership and freezes reports.
#[derive(Debug)]
pub enum PreparedFailure {
    Diagnosed(PreparedDiagnosticBatch),
    Infrastructure(Box<InfrastructureErrors>),
    Mixed {
        batch: PreparedDiagnosticBatch,
        error: Box<InfrastructureErrors>,
    },
}

impl PreparedFailure {
    /// Capture a stage-local bag at its owner edge, retaining path and type facts only when
    /// the caller confirms the matching path table is available.
    pub(crate) fn capture_diagnosed(
        bag: DiagnosticBag,
        string_table: StringTable,
        environment: Option<&TypeEnvironment>,
        path_table_is_available: bool,
        path_table: Option<Arc<PathTable>>,
    ) -> Self {
        match PreparedDiagnosticBatch::capture_with_required_paths_and_types(
            bag,
            string_table,
            path_table_is_available.then_some(environment).flatten(),
            || path_table_is_available.then_some(path_table).flatten(),
        ) {
            Ok(batch) => Self::Diagnosed(batch),
            Err(error) => Self::from(error),
        }
    }

    /// Preserve prior warnings before a diagnosed failure, in their original production order.
    pub(crate) fn with_diagnostic_and_warnings<D>(
        diagnostic: CompilerDiagnostic,
        warning_drafts: Vec<D>,
        string_table: &StringTable,
        environment: Option<&TypeEnvironment>,
        path_table: impl FnOnce() -> Option<Arc<PathTable>>,
    ) -> Self
    where
        D: Into<DiagnosticDraft>,
    {
        let mut bag = DiagnosticBag::from_diagnostics(warning_drafts);
        bag.push(diagnostic);
        match PreparedDiagnosticBatch::capture_with_required_paths_and_types(
            bag,
            string_table.clone(),
            environment,
            path_table,
        ) {
            Ok(batch) => Self::Diagnosed(batch),
            Err(error) => Self::from(error),
        }
    }

    /// Preserve prior warnings beside an infrastructure failure, in their original order.
    #[cfg(test)]
    pub(crate) fn with_infrastructure_and_warnings<D>(
        error: CompilerError,
        warning_drafts: Vec<D>,
        string_table: &StringTable,
        environment: Option<&TypeEnvironment>,
        path_table: impl FnOnce() -> Option<Arc<PathTable>>,
    ) -> Self
    where
        D: Into<DiagnosticDraft>,
    {
        let warning_drafts = warning_drafts
            .into_iter()
            .map(Into::into)
            .collect::<Vec<_>>();
        if warning_drafts.is_empty() {
            return Self::from(error);
        }
        match PreparedDiagnosticBatch::capture_with_required_paths_and_types(
            DiagnosticBag::from_diagnostics(warning_drafts),
            string_table.clone(),
            environment,
            path_table,
        ) {
            Ok(batch) => Self::Mixed {
                batch,
                error: error.into(),
            },
            Err(capture_error) => {
                let mut errors = InfrastructureErrors::new(error);
                errors.push(capture_error);
                Self::Infrastructure(Box::new(errors))
            }
        }
    }

    pub(crate) fn prepend_diagnostics_with_required_paths<D>(
        &mut self,
        prior_diagnostics: impl IntoIterator<Item = D>,
        path_table: impl FnOnce() -> Option<Arc<PathTable>>,
    ) -> Result<(), CompilerError>
    where
        D: Into<DiagnosticDraft>,
    {
        match self {
            Self::Diagnosed(batch) | Self::Mixed { batch, .. } => {
                batch.prepend_diagnostics_with_required_paths(prior_diagnostics, path_table)
            }
            Self::Infrastructure(_) => Ok(()),
        }
    }
}

impl From<PreparedDiagnosticBatch> for PreparedFailure {
    fn from(batch: PreparedDiagnosticBatch) -> Self {
        Self::Diagnosed(batch)
    }
}

impl From<CompilerError> for PreparedFailure {
    fn from(error: CompilerError) -> Self {
        Self::Infrastructure(error.into())
    }
}
