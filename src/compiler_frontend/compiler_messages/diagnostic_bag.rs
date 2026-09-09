//! Local diagnostic accumulator and premerge batch owner.
//!
//! WHAT: `DiagnosticBag` stores diagnostics emitted by a stage before a build or render
//! boundary packages them into `CompilerMessages`. `PremergeDiagnosticBatch` (also named
//! `LegacyDiagnosticBatch`) additionally owns the file-local `StringTable` and any local
//! render type-context ranges so a diagnosed file result can cross the preparation boundary
//! as one move-only owner without constructing `CompilerMessages` early.
//! WHY: file stages must not build the final boundary vessel; later module aggregation owns
//! the single merge into `CompilerMessages`. Both owners are move-only so a diagnostic can
//! never be observed through two owners at once.

use super::compiler_errors::RenderTypeContext;
use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, DiagnosticSeverity};
use crate::compiler_frontend::symbols::string_interning::{StringIdRemap, StringTable};

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

/// Historic name for [`PremergeDiagnosticBatch`], kept for the file-stage seam.
pub(crate) type LegacyDiagnosticBatch = PremergeDiagnosticBatch;

impl PremergeDiagnosticBatch {
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

    fn bag_take(&mut self) -> DiagnosticBag {
        std::mem::take(&mut self.bag)
    }

    fn render_type_contexts_take(&mut self) -> Vec<RenderTypeContext> {
        std::mem::take(&mut self.render_type_contexts)
    }
}
