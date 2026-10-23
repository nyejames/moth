//! Local diagnostic accumulator for owning-site capture.
//!
//! WHAT: `DiagnosticBag` stores diagnostics emitted by a stage before its owner captures them in
//! a `PreparedDiagnosticBatch`.
//! WHY: stages accumulate diagnostics without constructing a boundary vessel; the prepared
//! batch owns frozen diagnostic facts and the identity tables needed by the final boundary.

use super::compiler_errors::{CompilerError, InfrastructureErrors};
use super::prepared_batch::PreparedDiagnosticBatch;
use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, DiagnosticSeverity};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::symbols::path_interner::{PathIdRemap, PathTable};
use crate::compiler_frontend::symbols::string_interning::{StringIdRemap, StringTable};
use std::sync::Arc;

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct DiagnosticBag {
    diagnostics: Vec<CompilerDiagnostic>,
}

impl DiagnosticBag {
    pub(crate) fn new() -> Self {
        Self {
            diagnostics: Vec::new(),
        }
    }

    pub(crate) fn from_diagnostics(diagnostics: Vec<CompilerDiagnostic>) -> Self {
        Self { diagnostics }
    }

    pub(crate) fn push(&mut self, diagnostic: CompilerDiagnostic) {
        self.diagnostics.push(diagnostic);
    }

    pub(crate) fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error)
    }

    #[cfg(test)]
    pub(crate) fn has_warnings(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == DiagnosticSeverity::Warning)
    }

    #[cfg(test)]
    pub(crate) fn errors(&self) -> impl Iterator<Item = &CompilerDiagnostic> {
        self.diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error)
    }

    #[cfg(test)]
    pub(crate) fn warnings(&self) -> impl Iterator<Item = &CompilerDiagnostic> {
        self.diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.severity == DiagnosticSeverity::Warning)
    }

    #[cfg(test)]
    pub(crate) fn diagnostics(&self) -> &[CompilerDiagnostic] {
        &self.diagnostics
    }

    #[cfg(test)]
    pub(crate) fn as_slice(&self) -> &[CompilerDiagnostic] {
        &self.diagnostics
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

    pub(crate) fn take_diagnostics(&mut self) -> Vec<CompilerDiagnostic> {
        std::mem::take(&mut self.diagnostics)
    }

    pub(crate) fn into_diagnostics(self) -> Vec<CompilerDiagnostic> {
        self.diagnostics
    }
}

impl From<CompilerDiagnostic> for DiagnosticBag {
    fn from(diagnostic: CompilerDiagnostic) -> Self {
        Self::from_diagnostics(vec![diagnostic])
    }
}

/// Shared prepared failure lane preserving diagnosed vs infrastructure outcomes.
///
/// WHAT: carries prepared diagnosed facts, an owned infrastructure aggregate, or both when source
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
    pub(crate) fn with_diagnostic_and_warnings(
        diagnostic: CompilerDiagnostic,
        mut warning_diagnostics: Vec<CompilerDiagnostic>,
        string_table: &StringTable,
        environment: Option<&TypeEnvironment>,
        path_table: impl FnOnce() -> Option<Arc<PathTable>>,
    ) -> Self {
        warning_diagnostics.push(diagnostic);
        match PreparedDiagnosticBatch::capture_with_required_paths_and_types(
            DiagnosticBag::from_diagnostics(warning_diagnostics),
            string_table.clone(),
            environment,
            path_table,
        ) {
            Ok(batch) => Self::Diagnosed(batch),
            Err(error) => Self::from(error),
        }
    }

    #[cfg(test)]
    /// Preserve prior warnings beside an infrastructure failure, in their original order.
    pub(crate) fn with_infrastructure_and_warnings(
        error: CompilerError,
        warning_diagnostics: Vec<CompilerDiagnostic>,
        string_table: &StringTable,
        environment: Option<&TypeEnvironment>,
        path_table: impl FnOnce() -> Option<Arc<PathTable>>,
    ) -> Self {
        if warning_diagnostics.is_empty() {
            return Self::from(error);
        }
        match PreparedDiagnosticBatch::capture_with_required_paths_and_types(
            DiagnosticBag::from_diagnostics(warning_diagnostics),
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

    pub(crate) fn prepend_diagnostics_with_required_paths(
        &mut self,
        prior_diagnostics: impl IntoIterator<Item = CompilerDiagnostic>,
        path_table: impl FnOnce() -> Option<Arc<PathTable>>,
    ) -> Result<(), CompilerError> {
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
