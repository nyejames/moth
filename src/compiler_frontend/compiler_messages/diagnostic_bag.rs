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
use crate::compiler_frontend::source::SourceDatabase;
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

    /// Append diagnostics produced after the current contents, preserving order.
    pub(crate) fn extend(&mut self, diagnostics: impl IntoIterator<Item = CompilerDiagnostic>) {
        self.diagnostics.extend(diagnostics);
    }

    /// Move every diagnostic out of `other` onto the end of this bag, preserving order.
    ///
    /// WHAT: consumes `other` so the same diagnostic is never owned twice.
    /// WHY: file and module aggregation merge per-file bags into one premerge owner.
    pub(crate) fn append_bag(&mut self, other: Self) {
        self.diagnostics.extend(other.into_diagnostics());
    }

    /// Prepend diagnostics produced before the current contents, preserving order.
    ///
    /// WHAT: `prior` diagnostics stay before the existing ones.
    /// WHY: per-file warnings are emitted before the terminal diagnostic and must keep
    /// that production order through aggregation.
    pub(crate) fn prepend_diagnostics(
        &mut self,
        prior_diagnostics: impl IntoIterator<Item = CompilerDiagnostic>,
    ) {
        let mut prior_diagnostics = prior_diagnostics.into_iter().collect::<Vec<_>>();
        if prior_diagnostics.is_empty() {
            return;
        }

        prior_diagnostics.append(&mut self.diagnostics);
        self.diagnostics = prior_diagnostics;
    }

    /// Move every diagnostic out of `other` ahead of the current contents, preserving order.
    pub(crate) fn prepend_bag(&mut self, other: Self) {
        self.prepend_diagnostics(other.into_diagnostics());
    }

    pub(crate) fn len(&self) -> usize {
        self.diagnostics.len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.diagnostics.is_empty()
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = &CompilerDiagnostic> {
        self.diagnostics.iter()
    }

    pub(crate) fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error)
    }

    pub(crate) fn has_warnings(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == DiagnosticSeverity::Warning)
    }

    pub(crate) fn errors(&self) -> impl Iterator<Item = &CompilerDiagnostic> {
        self.diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error)
    }

    pub(crate) fn warnings(&self) -> impl Iterator<Item = &CompilerDiagnostic> {
        self.diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.severity == DiagnosticSeverity::Warning)
    }

    pub(crate) fn diagnostics(&self) -> &[CompilerDiagnostic] {
        &self.diagnostics
    }

    pub(crate) fn into_diagnostics(self) -> Vec<CompilerDiagnostic> {
        self.diagnostics
    }

    /// Remap every interned string owned by the bagged diagnostics into the merged table.
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
/// WHAT: carries either a move-only diagnosed [`PremergeDiagnosticBatch`] or a typed
/// infrastructure [`CompilerError`] without constructing `CompilerMessages`.
/// WHY: premerge producers must return one lane that keeps authored-source rejection
/// separate from malformed retained state; the final merge boundary owns the single
/// conversion into `CompilerMessages`.
#[derive(Debug)]
pub(crate) enum PremergeFailure {
    Diagnosed(PremergeDiagnosticBatch),
    Infrastructure(CompilerError),
}

impl PremergeFailure {
    pub(crate) fn diagnosed(batch: PremergeDiagnosticBatch) -> Self {
        Self::Diagnosed(batch)
    }

    pub(crate) fn infrastructure(error: CompilerError) -> Self {
        Self::Infrastructure(error)
    }

    pub(crate) fn is_diagnosed(&self) -> bool {
        matches!(self, Self::Diagnosed(_))
    }

    pub(crate) fn as_batch(&self) -> Option<&PremergeDiagnosticBatch> {
        match self {
            Self::Diagnosed(batch) => Some(batch),
            Self::Infrastructure(_) => None,
        }
    }

    pub(crate) fn as_infrastructure(&self) -> Option<&CompilerError> {
        match self {
            Self::Diagnosed(_) => None,
            Self::Infrastructure(error) => Some(error),
        }
    }

    pub(crate) fn into_result(self) -> Result<PremergeDiagnosticBatch, CompilerError> {
        match self {
            Self::Diagnosed(batch) => Ok(batch),
            Self::Infrastructure(error) => Err(error),
        }
    }
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
    /// Convert the premerge lane into the final boundary vessel exactly once.
    ///
    /// WHAT: moves a diagnosed batch into `CompilerMessages` or wraps an infrastructure
    /// `CompilerError` without cloning diagnostics.
    /// WHY: only true final build/package/check/benchmark boundaries own this conversion;
    /// intermediate stages return the failure lane itself so `StringId` remap and source
    /// attachment each happen exactly once at the merge boundary.
    pub(crate) fn into_messages(self, string_table: &StringTable) -> CompilerMessages {
        match self {
            Self::Diagnosed(batch) => batch.into_messages(),
            Self::Infrastructure(error) => CompilerMessages::from_error_ref(error, string_table),
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

/// Classify a deeper stage's mixed boundary vessel into the premerge lane.
///
/// WHAT: routes a `CompilerMessages` failure through the single
/// [`ModuleDiagnostics`](super::module_diagnostics::ModuleDiagnostics) classifier: user-facing
/// failures become a move-only diagnosed batch, infrastructure payloads (and malformed
/// sequences) become the typed `CompilerError` lane.
/// WHY: binding/order/AST/HIR stages still return the mixed vessel; the semantic service
/// normalizes each one here so `?` carries the classified lane upward without rebuilding a
/// second vessel per stage. Warning companions of infrastructure failures are discarded by
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

    pub(crate) fn empty(string_table: StringTable) -> Self {
        Self::new(DiagnosticBag::new(), string_table)
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

    pub(crate) fn push(&mut self, diagnostic: CompilerDiagnostic) {
        self.bag.push(diagnostic);
    }

    /// Append later diagnostics, preserving order. Type-context ranges are unaffected.
    pub(crate) fn extend(&mut self, diagnostics: impl IntoIterator<Item = CompilerDiagnostic>) {
        self.bag.extend(diagnostics);
    }

    /// Move every diagnostic and type-context range out of `other` onto the end of this
    /// batch, preserving order.
    ///
    /// WHAT: consumes `other`; its ranges shift by the current diagnostic count so they
    /// keep pointing at their own diagnostics.
    /// WHY: module aggregation merges premerge batches without copying diagnostics.
    /// The caller must have remapped `other` into this batch's string-table domain first;
    /// `other`'s table is discarded.
    pub(crate) fn append_batch(&mut self, mut other: Self) {
        let shift = self.bag.len();
        self.bag.append_bag(other.bag_take());
        for mut context in other.render_type_contexts_take() {
            context.diagnostic_range.start += shift;
            context.diagnostic_range.end += shift;
            self.render_type_contexts.push(context);
        }
    }

    /// Prepend earlier diagnostics, preserving order, and shift existing type-context
    /// ranges forward by the prepended length.
    pub(crate) fn prepend_diagnostics(
        &mut self,
        prior_diagnostics: impl IntoIterator<Item = CompilerDiagnostic>,
    ) {
        let prior_diagnostics = prior_diagnostics.into_iter().collect::<Vec<_>>();
        let shift = prior_diagnostics.len();
        if shift == 0 {
            return;
        }

        self.bag.prepend_diagnostics(prior_diagnostics);
        for context in &mut self.render_type_contexts {
            context.diagnostic_range.start += shift;
            context.diagnostic_range.end += shift;
        }
    }

    /// Move every diagnostic out of `other` ahead of the current contents. See
    /// [`Self::append_batch`] for the string-table domain contract; `other`'s table is
    /// discarded and its ranges shift onto the prepended positions.
    pub(crate) fn prepend_batch(&mut self, mut other: Self) {
        let shift = other.bag.len();

        for context in &mut self.render_type_contexts {
            context.diagnostic_range.start += shift;
            context.diagnostic_range.end += shift;
        }
        let mut contexts = other.render_type_contexts_take();
        contexts.append(&mut self.render_type_contexts);
        self.render_type_contexts = contexts;
        self.bag.prepend_bag(other.bag_take());
    }

    /// Attach one local render type-context range produced alongside these diagnostics.
    pub(crate) fn push_type_context(&mut self, context: RenderTypeContext) {
        self.render_type_contexts.push(context);
    }

    pub(crate) fn len(&self) -> usize {
        self.bag.len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.bag.is_empty()
    }

    pub(crate) fn has_errors(&self) -> bool {
        self.bag.has_errors()
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = &CompilerDiagnostic> {
        self.bag.iter()
    }

    pub(crate) fn diagnostics(&self) -> &[CompilerDiagnostic] {
        self.bag.diagnostics()
    }

    pub(crate) fn bag(&self) -> &DiagnosticBag {
        &self.bag
    }

    pub(crate) fn string_table(&self) -> &StringTable {
        &self.string_table
    }

    pub(crate) fn string_table_mut(&mut self) -> &mut StringTable {
        &mut self.string_table
    }

    pub(crate) fn render_type_contexts(&self) -> &[RenderTypeContext] {
        &self.render_type_contexts
    }

    /// Remap every interned string owned by this batch into the merged global table.
    ///
    /// WHAT: remaps the bagged diagnostics and every retained type environment.
    /// WHY: per-file preparation uses local tables; merging them into the module table
    /// requires shifting every `StringId` so later stages resolve through one table.
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

    /// Merge this batch's local string-table delta into the global table and remap once.
    ///
    /// WHAT: merges the owned local table delta starting at `base_len` into `global`, then
    /// remaps every bagged diagnostic and retained type environment through the returned
    /// remap. Identity merges skip the remap walk.
    /// WHY: canonical module aggregation merges each module-local delta exactly once before
    /// the diagnostics enter the boundary table; doing both steps here keeps that single-remap
    /// contract beside the batch owner instead of scattered across callers.
    pub(crate) fn merge_delta_into_global(&mut self, global: &mut StringTable, base_len: usize) {
        let remap = global.merge_delta_from(&self.string_table, base_len);
        if !remap.is_identity() {
            self.remap_string_ids(&remap);
        }
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
            string_table,
            render_source_contexts: Vec::new(),
            render_type_contexts,
        }
    }

    /// Consume the batch into the final vessel with its source database attached.
    pub(crate) fn into_messages_with_source(
        self,
        source_database: Arc<SourceDatabase>,
    ) -> CompilerMessages {
        let mut messages = self.into_messages();
        messages.set_source_database(source_database);
        messages
    }

    fn bag_take(&mut self) -> DiagnosticBag {
        std::mem::take(&mut self.bag)
    }

    fn render_type_contexts_take(&mut self) -> Vec<RenderTypeContext> {
        std::mem::take(&mut self.render_type_contexts)
    }
}
