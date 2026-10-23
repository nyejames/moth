//! Local diagnostic accumulator for owning-site capture.
//!
//! WHAT: `DiagnosticBag` stores diagnostics emitted by a stage before its owner captures them in
//! a `PreparedDiagnosticBatch`.
//! WHY: stages can accumulate diagnostics without constructing `CompilerMessages`; the prepared
//! batch owns frozen diagnostic facts and the identity tables needed by the final boundary.

use super::compiler_errors::{CompilerError, InfrastructureErrors};
use super::prepared_batch::PreparedDiagnosticBatch;
use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, DiagnosticSeverity};
use crate::compiler_frontend::symbols::path_interner::PathIdRemap;
use crate::compiler_frontend::symbols::string_interning::StringIdRemap;

#[derive(Debug, Default, PartialEq)]
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
pub enum PremergeFailure {
    Diagnosed(PreparedDiagnosticBatch),
    Infrastructure(Box<InfrastructureErrors>),
    Mixed {
        batch: PreparedDiagnosticBatch,
        error: Box<InfrastructureErrors>,
    },
}

impl From<PreparedDiagnosticBatch> for PremergeFailure {
    fn from(batch: PreparedDiagnosticBatch) -> Self {
        Self::Diagnosed(batch)
    }
}

impl From<CompilerError> for PremergeFailure {
    fn from(error: CompilerError) -> Self {
        Self::Infrastructure(error.into())
    }
}
