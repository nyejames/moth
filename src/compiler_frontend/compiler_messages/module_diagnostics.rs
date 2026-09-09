//! Diagnosed-module owner for the retained-module semantic boundary.
//!
//! WHAT: owns one diagnosed module's user-facing diagnostics, its module-local string table and
//!       render contexts produced alongside them.
//! WHY: the semantic module boundary separates a diagnosed source failure (user diagnostics the
//!      renderer surfaces) from an infrastructure `CompilerError` that aborts the build. This
//!      owner is the single place that classifies a deeper stage's boundary `CompilerMessages`
//!      into one of those two result classes, and it is structurally unable to carry an
//!      infrastructure failure: the constructor routes the outer `CompilerError` lane back out
//!      as a typed `CompilerError` instead of storing it as a normal diagnosed result.

use super::compiler_errors::{
    CompilerError, CompilerMessages, RenderFrozenContext, RenderSourceContext, RenderTypeContext,
};
use super::{CompilerDiagnostic, DiagnosticSeverity, PremergeDiagnosticBatch};
use crate::compiler_frontend::symbols::string_interning::StringTable;

/// One diagnosed module's user-facing diagnostic set at the retained-module semantic boundary.
///
/// WHAT: carries the ordered user-facing diagnostics, the module-local `StringTable` and render
///       contexts produced for one module, plus enough render identity to surface the diagnostics
///       after the local compilation call has finished.
/// WHY: a diagnosed module must expose no `Module` and no infrastructure failure. The
///      constructor rejects any outer `CompilerError` lane and routes it back as a typed
///      `CompilerError`, so a successful `ModuleDiagnostics` only ever carries user-facing
///      diagnostics the renderer is allowed to surface.
#[derive(Debug)]
pub(crate) struct ModuleDiagnostics {
    diagnostics: Vec<CompilerDiagnostic>,
    string_table: StringTable,
    render_frozen_contexts: Vec<RenderFrozenContext>,
    render_source_contexts: Vec<RenderSourceContext>,
    render_type_contexts: Vec<RenderTypeContext>,
}

impl ModuleDiagnostics {
    /// Classify a deeper stage's boundary `CompilerMessages` into a diagnosed module or a typed
    /// infrastructure `CompilerError`.
    ///
    /// WHAT: returns `Ok(ModuleDiagnostics)` when the message set carries no outer infrastructure
    ///       failure and at least one user-facing `Error` diagnostic; warnings and notes may
    ///       accompany that error and stay in production order for rendering. Returns
    ///       `Err(CompilerError)` when the set carries the outer infrastructure lane: the typed
    ///       error is recovered as-is, and any non-error (Warning/Note) companions it travelled
    ///       with — the legitimate `from_error_with_warnings` shape emitted by AST/HIR stages —
    ///       are discarded because the typed `Err` lane aborts the owning compilation and does
    ///       not surface warnings. An empty, warning/note-only, or mixed user-error-plus-outer
    ///       sequence is reported as a compiler invariant failure.
    /// WHY: deeper stage APIs still return the legacy `CompilerMessages` vessel, and AST/HIR
    ///      stages legitimately prepend warnings to a single infrastructure failure. The semantic
    ///      module boundary normalizes them once, here, using only the outer lane and
    ///      `DiagnosticSeverity` — never rendered strings, stable diagnostic codes or `ErrorType`
    ///      guesses — and never silently drops a user error that travelled beside an
    ///      infrastructure failure.
    ///
    /// The infrastructure error carries its own typed source span or host path. The consumed
    /// message set's `StringTable` remains owned by the diagnosed lane and is not attached to or
    /// remapped through the error.
    pub(crate) fn from_messages(messages: CompilerMessages) -> Result<Self, CompilerError> {
        let CompilerMessages {
            diagnostics,
            infrastructure_error,
            string_table,
            render_frozen_contexts,
            render_source_contexts,
            render_type_contexts,
        } = messages;

        // One explicit classification pass over the diagnostic stream. The boundary reads only
        // the outer lane and severity: a stored outer failure marks an internal failure, while
        // an `Error` severity marks a user-facing source error.
        let mut has_user_error = false;
        for diagnostic in &diagnostics {
            if diagnostic.severity == DiagnosticSeverity::Error {
                has_user_error = true;
            }
        }

        match infrastructure_error {
            None => {
                // No outer failure. A diagnosed module requires at least one user-facing
                // `Error` diagnostic; warnings and notes may accompany that error and stay in
                // production order for rendering. An empty or warning/note-only failure carries no
                // user error to surface, so it is a compiler invariant failure rather than a silent
                // empty diagnosed module.
                if !has_user_error {
                    if diagnostics.is_empty() {
                        return Err(CompilerError::compiler_error(
                            "module semantic stage returned a failure with no diagnostics",
                        ));
                    }
                    return Err(CompilerError::compiler_error(
                        "module semantic stage returned a failure with no user-facing error diagnostic",
                    ));
                }

                Ok(ModuleDiagnostics {
                    diagnostics,
                    string_table,
                    render_frozen_contexts,
                    render_source_contexts,
                    render_type_contexts,
                })
            }
            Some(error) => {
                // An outer infrastructure failure. The legitimate `from_error_with_warnings`
                // shape from AST/HIR stages emits non-error (Warning/Note) companion diagnostics
                // before the single failure, so an outer failure may carry companions. Recover
                // the originating `CompilerError` as-is and discard the companions: the typed
                // `Err` lane aborts the owning compilation and does not surface warnings. A
                // user-facing `Error` diagnostic beside the outer failure is malformed — the
                // current stage contracts never blend a user error with an infrastructure
                // failure — and is reported as an invariant instead of silently dropping the
                // user error.
                if has_user_error {
                    return Err(CompilerError::compiler_error(format!(
                        "module semantic stage returned a malformed diagnostic sequence mixing a \
                         user-facing error with an infrastructure failure among {total} total",
                        total = diagnostics.len(),
                    )));
                }

                Err(error)
            }
        }
    }

    ///
    /// WHAT: moves the owned diagnostics, string table and render type contexts back into the
    ///       boundary container existing outer callers still consume.
    /// WHY: this is the lossless inverse of `from_messages`. Build and render boundaries keep
    ///       using `CompilerMessages` while the semantic boundary exchanges the typed
    ///       `ModuleDiagnostics` owner.
    pub(crate) fn into_messages(self) -> CompilerMessages {
        CompilerMessages {
            diagnostics: self.diagnostics,
            infrastructure_error: None,
            string_table: self.string_table,
            render_frozen_contexts: self.render_frozen_contexts,
            render_source_contexts: self.render_source_contexts,
            render_type_contexts: self.render_type_contexts,
        }
    }
    /// Classify a premerge batch into a diagnosed module or a typed invariant failure.
    ///
    /// WHAT: moves the batch's diagnostics, string table and type contexts into the diagnosed
    /// owner after verifying the batch carries at least one user-facing `Error`. The premerge
    /// lane is typed: infrastructure failures travel as `PremergeFailure::Infrastructure` and
    /// never enter a batch, so no payload inspection is needed here. Batches arriving through
    /// the premerge lane never carry source contexts, so the result starts with none; the
    /// final boundary attaches them later.
    /// WHY: canonical aggregation merges premerge batches exactly once and only then needs the
    /// diagnosed owner the render boundary consumes. Classifying here keeps the single
    /// user-error/infrastructure separation beside the existing `from_messages` owner instead
    /// of reintroducing a mixed message vessel for the merge.
    pub(crate) fn from_batch(batch: PremergeDiagnosticBatch) -> Result<Self, CompilerError> {
        let (bag, string_table, render_type_contexts) = batch.into_parts();
        let diagnostics = bag.into_diagnostics();
        let mut has_user_error = false;
        for diagnostic in &diagnostics {
            if diagnostic.severity == DiagnosticSeverity::Error {
                has_user_error = true;
            }
        }
        if !has_user_error {
            if diagnostics.is_empty() {
                return Err(CompilerError::compiler_error(
                    "module semantic stage returned a premerge batch with no diagnostics",
                ));
            }
            return Err(CompilerError::compiler_error(
                "module semantic stage returned a premerge batch with no user-facing error diagnostic",
            ));
        }
        Ok(Self {
            diagnostics,
            string_table,
            render_frozen_contexts: Vec::new(),
            render_source_contexts: Vec::new(),
            render_type_contexts,
        })
    }

    /// Move this diagnosed module back into the premerge batch lane for canonical merging.
    ///
    /// WHAT: moves diagnostics, string table and type contexts into a batch, rejecting a
    /// module that already carries source contexts with a typed invariant failure.
    /// WHY: canonical wave merging remaps each module-local delta exactly once through the
    /// batch owner, then classifies back via `from_batch` without round-tripping through the
    /// final `CompilerMessages` vessel. Batches intentionally carry no source contexts — the
    /// final boundary attaches them after merging — so an attached association here is a
    /// caller error. Rejecting it in all builds keeps a release build from silently
    /// misassociating diagnostics with a missing snapshot; premerge batches stay
    /// source-context-free by construction.
    pub(crate) fn into_batch(self) -> Result<PremergeDiagnosticBatch, CompilerError> {
        if !self.render_source_contexts.is_empty() {
            return Err(CompilerError::compiler_error(format!(
                "diagnosed module reached canonical merging with {} attached source contexts \
                 beside {} diagnostics; source attachment must happen after merging",
                self.render_source_contexts.len(),
                self.diagnostics.len(),
            )));
        }
        if !self.render_frozen_contexts.is_empty() {
            return Err(CompilerError::compiler_error(format!(
                "diagnosed module reached canonical merging with {} attached frozen identity \
                 contexts beside {} diagnostics; frozen attachment must happen after merging",
                self.render_frozen_contexts.len(),
                self.diagnostics.len(),
            )));
        }
        let Self {
            diagnostics,
            string_table,
            render_type_contexts,
            ..
        } = self;
        Ok(PremergeDiagnosticBatch::from_parts(
            diagnostics,
            string_table,
            render_type_contexts,
        ))
    }

    /// Borrow the ordered user-facing diagnostics. Used by focused boundary tests.
    #[cfg(test)]
    pub(crate) fn diagnostics(&self) -> &[CompilerDiagnostic] {
        &self.diagnostics
    }

    /// Borrow the module-local string table. Used by focused boundary tests.
    #[cfg(test)]
    pub(crate) fn string_table(&self) -> &StringTable {
        &self.string_table
    }

    /// Borrow the render type contexts. Used by focused boundary tests.
    #[cfg(test)]
    pub(crate) fn render_type_contexts(&self) -> &[RenderTypeContext] {
        &self.render_type_contexts
    }
}
