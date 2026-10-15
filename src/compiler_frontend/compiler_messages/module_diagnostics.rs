//! Diagnosed-module owner for the retained-module semantic boundary.
//!
//! WHAT: owns one diagnosed module's user-facing diagnostics, its module-local string table and
//!       render contexts produced alongside them.
//! WHY: the semantic module boundary classifies `CompilerMessages` into diagnosed source
//! failures or an owned infrastructure aggregate. A diagnosed module cannot retain infrastructure
//! causes, so the constructor moves every cause back out through its typed error lane.

use super::compiler_errors::{
    CompilerError, CompilerMessages, InfrastructureErrors, RenderFrozenContext, RenderPathContext,
    RenderSourceContext, RenderTypeContext,
};
use super::{CompilerDiagnostic, DiagnosticSeverity, PremergeDiagnosticBatch};
use crate::compiler_frontend::symbols::string_interning::StringTable;

/// One diagnosed module's user-facing diagnostic set at the retained-module semantic boundary.
///
/// WHAT: carries the ordered user-facing diagnostics, the module-local `StringTable` and render
///       contexts produced for one module, plus enough render identity to surface the diagnostics
///       after the local compilation call has finished.
/// WHY: a diagnosed module exposes no `Module` and no infrastructure failure. Its constructor
/// routes every outer infrastructure cause back through `InfrastructureErrors`.
#[derive(Debug)]
pub(crate) struct ModuleDiagnostics {
    diagnostics: Vec<CompilerDiagnostic>,
    string_table: StringTable,
    render_frozen_contexts: Vec<RenderFrozenContext>,
    render_source_contexts: Vec<RenderSourceContext>,
    render_type_contexts: Vec<RenderTypeContext>,
    render_path_contexts: Vec<RenderPathContext>,
}

impl ModuleDiagnostics {
    /// Classify boundary messages into a diagnosed module or the whole infrastructure aggregate.
    ///
    /// User errors retain their warning/note companions in production order. Infrastructure
    /// failures move out unchanged and discard warning/note companions because the typed error
    /// lane aborts the compilation. Mixing user errors with infrastructure causes is malformed:
    /// append an invariant failure without replacing any original cause. Empty and warning-only
    /// failures also report invariants rather than constructing empty diagnosed modules.
    ///
    /// Classification reads only the outer lane and diagnostic severity, never rendered strings,
    /// diagnostic codes or infrastructure error kinds.
    pub(crate) fn from_messages(
        messages: CompilerMessages,
    ) -> Result<Self, Box<InfrastructureErrors>> {
        let CompilerMessages {
            diagnostics,
            infrastructure_errors,
            string_table,
            render_frozen_contexts,
            render_source_contexts,
            render_type_contexts,
            render_path_contexts,
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

        match infrastructure_errors {
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
                        )
                        .into());
                    }
                    return Err(CompilerError::compiler_error(
                        "module semantic stage returned a failure with no user-facing error diagnostic",
                    ).into());
                }

                Ok(ModuleDiagnostics {
                    diagnostics,
                    string_table: *string_table,
                    render_frozen_contexts,
                    render_source_contexts,
                    render_type_contexts,
                    render_path_contexts: render_path_contexts
                        .map_or_else(Vec::new, |contexts| contexts.into_vec()),
                })
            }
            Some(mut errors) => {
                // Warning/note companions are discarded at this aborting boundary. A user error
                // cannot be surfaced through this lane, so retain its malformed-sequence
                // invariant after every original infrastructure cause.
                if has_user_error {
                    errors.push(CompilerError::compiler_error(format!(
                        "module semantic stage returned a malformed diagnostic sequence mixing a \
                         user-facing error with an infrastructure failure among {total} total",
                        total = diagnostics.len(),
                    )));
                }

                Err(errors)
            }
        }
    }

    ///
    /// WHAT: moves the owned diagnostics, string table and render type contexts back into the
    ///       boundary container existing outer callers still consume.
    /// WHY: this is the lossless inverse of `from_messages`. Build and render boundaries keep
    pub(crate) fn into_messages(self) -> CompilerMessages {
        CompilerMessages {
            diagnostics: self.diagnostics,
            infrastructure_errors: None,
            string_table: Box::new(self.string_table),
            render_frozen_contexts: self.render_frozen_contexts,
            render_source_contexts: self.render_source_contexts,
            render_type_contexts: self.render_type_contexts,
            render_path_contexts: (!self.render_path_contexts.is_empty())
                .then(|| self.render_path_contexts.into_boxed_slice()),
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
    pub(crate) fn from_batch(batch: PremergeDiagnosticBatch) -> Result<Self, CompilerError> {
        let (bag, string_table, render_type_contexts, render_path_contexts) = batch.into_parts();
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
            render_path_contexts,
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
            render_path_contexts,
            render_type_contexts,
            ..
        } = self;
        Ok(PremergeDiagnosticBatch::from_parts(
            diagnostics,
            string_table,
            render_type_contexts,
            render_path_contexts,
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
