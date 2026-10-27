//! Local diagnostic accumulator for owning-site capture.
//!
//! WHAT: `DiagnosticBag` stores diagnostics emitted by a stage before its owner captures them in
//! a `PreparedDiagnosticBatch`.
//! WHY: stages accumulate diagnostic drafts; the prepared batch owns frozen diagnostic facts and the
//! identity tables needed by the final boundary.

use super::compiler_errors::{InfrastructureErrors, InfrastructureFailure};
use super::diagnostics::draft::DiagnosticDraft;
use super::prepared_batch::PreparedDiagnosticBatch;
use crate::compiler_frontend::compiler_messages::DiagnosticSeverity;
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

    pub(crate) fn from_diagnostics(diagnostics: Vec<DiagnosticDraft>) -> Self {
        Self { diagnostics }
    }

    pub(crate) fn push(&mut self, diagnostic: DiagnosticDraft) {
        self.diagnostics.push(diagnostic);
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
    /// Successful warnings stay staged, but a failed diagnostic batch keeps only its errors.
    pub(crate) fn retain_errors_only_if_failed(&mut self) {
        if self.has_errors() {
            self.diagnostics
                .retain(|diagnostic| diagnostic.severity() == DiagnosticSeverity::Error);
        }
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

impl From<DiagnosticDraft> for DiagnosticBag {
    fn from(diagnostic: DiagnosticDraft) -> Self {
        let mut bag = Self::new();
        bag.push(diagnostic);
        bag
    }
}

/// Prepared user diagnostics or aggregate infrastructure causes.
///
/// This stage-local type does not combine the two lanes. At the owning boundary infrastructure
/// takes precedence if both are encountered.
#[derive(Debug)]
pub enum PreparedFailure {
    Diagnosed(PreparedDiagnosticBatch),
    Infrastructure(Box<InfrastructureErrors>),
}

impl PreparedFailure {
    /// Capture a stage-local bag at its owner edge, using the supplied type environment and
    /// retaining a path snapshot only for captured path IDs.
    pub(crate) fn capture_diagnosed(
        bag: DiagnosticBag,
        string_table: StringTable,
        environment: Option<&TypeEnvironment>,
        path_table: Option<Arc<PathTable>>,
    ) -> Self {
        match PreparedDiagnosticBatch::capture_with_required_paths(
            bag,
            string_table,
            environment,
            || path_table,
        ) {
            Ok(batch) => Self::Diagnosed(batch),
            Err(error) => Self::from(error),
        }
    }

    /// Admit an infrastructure cause into this owner's failure. Infrastructure dominates a
    /// diagnosed failure and appends to earlier infrastructure causes.
    pub(crate) fn with_infrastructure_cause(self, error: InfrastructureFailure) -> Self {
        self.with_infrastructure_errors(InfrastructureErrors::new(error))
    }

    /// Admit a later failure of the same owner, keeping the first diagnosed failure.
    ///
    /// A later diagnostic is dropped here, before capture, so it never takes the owner's string
    /// or path table. A later infrastructure cause still merges and dominates.
    pub(crate) fn with_later(self, later: Self) -> Self {
        match later {
            Self::Infrastructure(errors) => self.with_infrastructure_errors(*errors),
            Self::Diagnosed(_) => self,
        }
    }

    /// Admit infrastructure causes into this owner's failure with the dominance of
    /// [`Self::with_infrastructure_cause`].
    pub(crate) fn with_infrastructure_errors(self, errors: InfrastructureErrors) -> Self {
        match self {
            Self::Infrastructure(mut kept) => {
                kept.append(errors);
                Self::Infrastructure(kept)
            }
            Self::Diagnosed(_) => Self::Infrastructure(Box::new(errors)),
        }
    }
}

impl From<PreparedDiagnosticBatch> for PreparedFailure {
    fn from(batch: PreparedDiagnosticBatch) -> Self {
        Self::Diagnosed(batch)
    }
}

impl From<InfrastructureFailure> for PreparedFailure {
    fn from(error: InfrastructureFailure) -> Self {
        Self::Infrastructure(error.into())
    }
}
impl From<Box<InfrastructureErrors>> for PreparedFailure {
    fn from(errors: Box<InfrastructureErrors>) -> Self {
        Self::Infrastructure(errors)
    }
}
impl From<DiagnosticDraft> for PreparedFailure {
    fn from(diagnostic: DiagnosticDraft) -> Self {
        Self::capture_diagnosed(
            DiagnosticBag::from(diagnostic),
            StringTable::new(),
            None,
            None,
        )
    }
}
