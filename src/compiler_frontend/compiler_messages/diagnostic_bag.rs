//! Local diagnostic accumulator and premerge batch owner.
//!
//! WHAT: `DiagnosticBag` stores diagnostics emitted by a stage before a build or render
//! boundary packages them into `CompilerMessages`. `PremergeDiagnosticBatch` additionally owns the
//! file-local `StringTable` and any local render type-context ranges so a diagnosed file result can
//! cross the preparation boundary as one move-only owner without constructing `CompilerMessages`
//! early.
//! WHY: file stages must not build the final boundary vessel; later module aggregation owns
//! the single merge into `CompilerMessages`. Both owners are move-only so a diagnostic can
//! never be observed through two owners at once.

use super::compiler_errors::{CompilerError, CompilerMessages, RenderTypeContext};
use super::module_diagnostics::ModuleDiagnostics;
use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, DiagnosticSeverity};
use crate::compiler_frontend::source::{FrozenIdentityHandle, SourceDatabase};
use crate::compiler_frontend::symbols::string_interning::{StringIdRemap, StringTable};
use std::sync::Arc;

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

    pub(crate) fn into_diagnostics(self) -> Vec<CompilerDiagnostic> {
        self.diagnostics
    }

    /// Remap every interned string owned by the bagged diagnostics into the merged table.
    #[allow(dead_code)]
    pub(crate) fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        for diagnostic in &mut self.diagnostics {
            diagnostic.remap_string_ids(remap);
        }
    }
}

impl From<CompilerDiagnostic> for DiagnosticBag {
    fn from(diagnostic: CompilerDiagnostic) -> Self {
        Self::from_diagnostics(vec![diagnostic])
    }
}

/// Move-only premerge owner for one diagnosed file result.
///
/// WHAT: owns the file's diagnostics as a `DiagnosticBag` plus the file-local `StringTable`
/// that issued their interned ids, and any local render type-context ranges the producer
/// already attached for later module aggregation. File-stage producers carry no type
/// contexts yet, so the range list starts empty and only shifts through prepend/append.
/// WHY: the diagnosed file result must cross the preparation boundary as a single owner
/// without constructing the final `CompilerMessages` vessel. Source snapshots stay with
/// the final source owner and attach only at the render boundary, so this batch never
/// carries source contexts.
#[derive(Debug)]
pub(crate) struct PremergeDiagnosticBatch {
    bag: DiagnosticBag,
    string_table: StringTable,
    render_type_contexts: Vec<RenderTypeContext>,
}

/// Shared premerge failure lane preserving diagnosed vs infrastructure outcomes.
///
/// WHAT: carries a move-only diagnosed [`PremergeDiagnosticBatch`], a typed infrastructure
/// [`CompilerError`], or both when a source-finalization failure races a semantic failure.
/// The mixed form exists only for that double-failure tail: the diagnosed batch stays
/// authoritative for user output while the finish failure stays observable on the outer
/// infrastructure lane instead of being dropped or fabricated into a user diagnostic.
/// WHY: premerge producers must return one lane that keeps authored-source rejection
/// separate from malformed retained state; the final merge boundary owns the single
/// conversion into `CompilerMessages`.
#[derive(Debug)]
pub(crate) enum PremergeFailure {
    Diagnosed(PremergeDiagnosticBatch),
    Infrastructure(CompilerError),
    Mixed {
        batch: PremergeDiagnosticBatch,
        error: CompilerError,
    },
}

impl From<PremergeDiagnosticBatch> for PremergeFailure {
    fn from(batch: PremergeDiagnosticBatch) -> Self {
        Self::Diagnosed(batch)
    }
}

impl From<CompilerError> for PremergeFailure {
    fn from(error: CompilerError) -> Self {
        Self::Infrastructure(error)
    }
}

impl PremergeFailure {
    /// Prepend earlier-stage diagnostics to a diagnosed premerge batch.
    ///
    /// Infrastructure failures remain typed and untouched. A mixed failure retains its outer
    /// infrastructure error while shifting the diagnosed batch's render ranges.
    pub(crate) fn prepend_diagnostics(
        &mut self,
        prior_diagnostics: impl IntoIterator<Item = CompilerDiagnostic>,
    ) {
        match self {
            Self::Diagnosed(batch) => batch.prepend_diagnostics(prior_diagnostics),
            Self::Infrastructure(_) => {}
            Self::Mixed { batch, .. } => batch.prepend_diagnostics(prior_diagnostics),
        }
    }
    /// Preserve existing mixed-domain owners while filling ownership for an unowned batch.
    pub(crate) fn set_frozen_identity_handle_if_missing(
        &mut self,
        frozen_identity_handle: FrozenIdentityHandle,
    ) {
        match self {
            Self::Diagnosed(batch) => {
                batch.set_frozen_identity_handle_if_missing(frozen_identity_handle);
            }
            Self::Infrastructure(_) => {}
            Self::Mixed { batch, .. } => {
                batch.set_frozen_identity_handle_if_missing(frozen_identity_handle);
            }
        }
    }

    /// Convert the premerge lane into the final boundary vessel exactly once.
    ///
    /// WHAT: moves a diagnosed batch into `CompilerMessages`, carries an infrastructure
    /// `CompilerError` on the outer lane, or carries both for a mixed double-failure,
    /// without cloning diagnostics.
    /// WHY: only true final build/package/check/benchmark boundaries own this conversion;
    /// intermediate stages return the failure lane itself so `StringId` remap and source
    /// attachment each happen exactly once at the merge boundary.
    pub(crate) fn into_messages(self, string_table: &StringTable) -> CompilerMessages {
        match self {
            Self::Diagnosed(batch) => batch.into_messages(),
            Self::Infrastructure(error) => CompilerMessages::from_error_ref(error, string_table),
            Self::Mixed { batch, error } => {
                let mut messages = batch.into_messages();
                messages.set_infrastructure_error(error);
                messages
            }
        }
    }

    /// Convert the lane into the final vessel and attach the finalized source database.
    ///
    /// WHAT: builds the message set as [`Self::into_messages`] then attaches `source_database`
    /// as the render source context for every diagnostic.
    /// WHY: discovery/preparation failures carry no source contexts; the final owner that just
    /// finished the source tables attaches them once here instead of at every intermediate site.
    pub(crate) fn into_messages_with_source(
        self,
        string_table: &StringTable,
        source_database: Arc<SourceDatabase>,
    ) -> CompilerMessages {
        let mut messages = self.into_messages(string_table);
        messages.set_source_database(source_database);
        messages
    }
}

/// Transitional classifier for AST/HIR APIs that still return `CompilerMessages`.
///
/// WHAT: routes a legacy boundary vessel through the single
/// [`ModuleDiagnostics`](super::module_diagnostics::ModuleDiagnostics) classifier: user-facing
/// failures become a move-only diagnosed batch, while infrastructure and malformed sequences
/// become the typed `CompilerError` lane.
/// WHY: direct migration of those public AST/HIR methods is outside this stage-handoff slice.
/// New orchestration wrappers classify immediately and carry `PremergeFailure` upward without
/// rebuilding a second vessel. Warning companions of infrastructure failures are discarded by
/// the classifier because the typed lane aborts the owning compilation.
impl From<super::compiler_errors::CompilerMessages> for PremergeFailure {
    fn from(messages: super::compiler_errors::CompilerMessages) -> Self {
        match ModuleDiagnostics::from_messages(messages) {
            Ok(diagnostics) => match diagnostics.into_batch() {
                Ok(batch) => Self::Diagnosed(batch),
                Err(error) => Self::Infrastructure(error),
            },
            Err(error) => Self::Infrastructure(error),
        }
    }
}
impl PremergeDiagnosticBatch {
    /// Rebuild a batch from already-merged diagnostics, table and type contexts.
    ///
    /// WHAT: moves each lane back into the single premerge owner without cloning.
    /// WHY: canonical merging moves a diagnosed module into the batch lane for its single
    /// remap, then classifies back; this constructor keeps that round-trip move-only.
    pub(crate) fn from_parts(
        diagnostics: Vec<CompilerDiagnostic>,
        string_table: StringTable,
        render_type_contexts: Vec<RenderTypeContext>,
    ) -> Self {
        Self {
            bag: DiagnosticBag::from_diagnostics(diagnostics),
            string_table,
            render_type_contexts,
        }
    }

    pub(crate) fn new(bag: DiagnosticBag, string_table: StringTable) -> Self {
        Self {
            bag,
            string_table,
            render_type_contexts: Vec::new(),
        }
    }

    pub(crate) fn from_bag(bag: DiagnosticBag, string_table: StringTable) -> Self {
        Self::new(bag, string_table)
    }

    pub(crate) fn from_diagnostic(
        diagnostic: CompilerDiagnostic,
        string_table: StringTable,
    ) -> Self {
        Self::new(DiagnosticBag::from(diagnostic), string_table)
    }

    pub(crate) fn from_diagnostics(
        diagnostics: Vec<CompilerDiagnostic>,
        string_table: StringTable,
    ) -> Self {
        Self::new(DiagnosticBag::from_diagnostics(diagnostics), string_table)
    }
    /// Attach the generated-source owner only to spans that do not already have one.
    ///
    /// Existing primary and label owners are authoritative: this bridge supplies ownership for
    /// legacy AST/HIR diagnostics without replacing mixed-domain provenance.
    pub(crate) fn set_frozen_identity_handle_if_missing(
        &mut self,
        frozen_identity_handle: FrozenIdentityHandle,
    ) {
        for diagnostic in &mut self.bag.diagnostics {
            diagnostic.attach_frozen_identity_handle_if_missing(frozen_identity_handle.clone());
        }
    }

    pub(crate) fn has_errors(&self) -> bool {
        self.bag.has_errors()
    }
    /// Prepend diagnostics emitted before this batch while preserving retained render ranges.
    ///
    /// WHAT: moves the incoming diagnostics in front of this batch and shifts each retained type
    /// context by their count.
    /// WHY: preparation warnings are collected before a later diagnosed batch; moving them here
    /// keeps authored order and avoids cloning diagnostics or their interned IDs.
    pub(crate) fn prepend_diagnostics(
        &mut self,
        prior_diagnostics: impl IntoIterator<Item = CompilerDiagnostic>,
    ) {
        let mut prior_diagnostics = prior_diagnostics.into_iter().collect::<Vec<_>>();
        let shift = prior_diagnostics.len();

        if shift == 0 {
            return;
        }

        prior_diagnostics.append(&mut self.bag.diagnostics);
        self.bag.diagnostics = prior_diagnostics;

        for type_context in &mut self.render_type_contexts {
            type_context.diagnostic_range.start += shift;
            type_context.diagnostic_range.end += shift;
        }
    }

    /// Remap every interned string owned by this batch into the merged global table.
    ///
    /// WHAT: remaps the bagged diagnostics and every retained type environment.
    /// WHY: per-file preparation uses local tables; merging them into the module table
    /// requires shifting every `StringId` so later stages resolve through one table.
    #[allow(dead_code)]
    pub(crate) fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        self.bag.remap_string_ids(remap);
        for context in &mut self.render_type_contexts {
            context.type_environment.remap_string_ids(remap);
        }
    }

    /// Consume the batch into its owned parts for the final merge boundary.
    pub(crate) fn into_parts(self) -> (DiagnosticBag, StringTable, Vec<RenderTypeContext>) {
        (self.bag, self.string_table, self.render_type_contexts)
    }

    /// Consume the batch into the final boundary vessel, preserving type-context ranges.
    ///
    /// WHAT: moves the owned diagnostics, string table and render type contexts into
    /// `CompilerMessages` with no source contexts attached.
    /// WHY: this is the single vessel construction for a premerge batch; the final owner
    /// attaches its finalized source database afterwards so each diagnostic keeps exactly one
    /// source association.
    pub(crate) fn into_messages(self) -> CompilerMessages {
        let (bag, string_table, render_type_contexts) = self.into_parts();
        CompilerMessages {
            diagnostics: bag.into_diagnostics(),
            infrastructure_error: None,
            string_table: Box::new(string_table),
            render_frozen_contexts: Vec::new(),
            render_source_contexts: Vec::new(),
            render_type_contexts,
        }
    }
}
