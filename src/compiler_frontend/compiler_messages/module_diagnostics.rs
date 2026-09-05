//! Diagnosed-module owner for the retained-module semantic boundary.
//!
//! WHAT: owns one diagnosed module's user-facing diagnostics, the module-local string table and
//!       render contexts produced alongside them.
//! WHY: the semantic module boundary separates a diagnosed source failure (user diagnostics the
//!      renderer surfaces) from an infrastructure `CompilerError` that aborts the build. This
//!      owner is the single place that classifies a deeper stage's mixed `CompilerMessages` into
//!      one of those two result classes, and it is structurally unable to carry an infrastructure
//!      diagnostic: the constructor routes any `DiagnosticPayload::InfrastructureError` back out
//!      as a typed `CompilerError` instead of storing it as a normal diagnosed result.

use super::compiler_errors::{
    CompilerError, CompilerMessages, RenderSourceContext, RenderTypeContext,
};
use super::{CompilerDiagnostic, DiagnosticPayload, DiagnosticSeverity};
use crate::compiler_frontend::symbols::string_interning::StringTable;

/// One diagnosed module's user-facing diagnostic set at the retained-module semantic boundary.
///
/// WHAT: carries the ordered user-facing diagnostics, the module-local `StringTable` and render
///       contexts produced for one module, plus enough render identity to surface the diagnostics
///       after the local compilation call has finished.
/// WHY: a diagnosed module must expose no `Module` and no infrastructure diagnostic. The
///      constructor rejects any `DiagnosticPayload::InfrastructureError` and routes it back as a
///      `CompilerError`, so a successful `ModuleDiagnostics` only ever carries user-facing
///      diagnostics the renderer is allowed to surface.
#[derive(Debug)]
pub(crate) struct ModuleDiagnostics {
    diagnostics: Vec<CompilerDiagnostic>,
    string_table: StringTable,
    render_source_contexts: Vec<RenderSourceContext>,
    render_type_contexts: Vec<RenderTypeContext>,
}

impl ModuleDiagnostics {
    /// Classify a deeper stage's boundary `CompilerMessages` into a diagnosed module or a typed
    /// infrastructure `CompilerError`.
    ///
    /// WHAT: returns `Ok(ModuleDiagnostics)` when the message set carries no infrastructure
    ///       payload and at least one user-facing `Error` diagnostic; warnings and notes may
    ///       accompany that error and stay in production order for rendering. Returns
    ///       `Err(CompilerError)` when the set carries an infrastructure payload: a single
    ///       infrastructure diagnostic is recovered losslessly from its structured payload, and
    ///       any non-error (Warning/Note) companions it travelled with — the legitimate
    ///       `from_error_with_warnings` shape emitted by AST/HIR stages — are discarded because
    ///       the typed `Err` lane aborts the owning compilation and does not surface warnings.
    ///       An empty, warning/note-only, mixed user-error-plus-infrastructure, or
    ///       multi-infrastructure sequence is reported as a compiler invariant failure.
    /// WHY: deeper stage APIs still return mixed `CompilerMessages`, and AST/HIR stages
    ///      legitimately prepend warnings to a single infrastructure failure. The semantic module
    ///      boundary normalizes them once, here, using only the structured payload and
    ///      `DiagnosticSeverity` — never rendered strings, stable diagnostic codes or `ErrorType`
    ///      guesses — and never silently drops a user error that travelled beside an
    ///      infrastructure failure.
    ///
    /// Render-identity preservation: when an infrastructure diagnostic is recovered into a typed
    /// `CompilerError`, the consumed message set's `StringTable` is attached to that error. The
    /// error's `SourceLocation` carries interned path IDs issued by that module-local table, so
    /// the attached context lets `CompilerMessages::from_error` merge and remap the location
    /// exactly once instead of resolving it against a mismatched or empty table.
    pub(crate) fn from_messages(messages: CompilerMessages) -> Result<Self, CompilerError> {
        let diagnostics = messages.diagnostics;
        let string_table = messages.string_table;
        let render_source_contexts = messages.render_source_contexts;
        let render_type_contexts = messages.render_type_contexts;

        // One explicit classification pass over the diagnostic stream. The boundary reads only
        // the structured payload and severity: an infrastructure payload marks an internal
        // failure, while a non-infrastructure `Error` severity marks a user-facing source error.
        // Recording the single infrastructure index here lets the recovery path move that one
        // diagnostic out of the stream without cloning, instead of scanning a second time.
        let mut infrastructure_index: Option<usize> = None;
        let mut infrastructure_count = 0usize;
        let mut has_user_error = false;
        for (index, diagnostic) in diagnostics.iter().enumerate() {
            if is_infrastructure_payload(diagnostic) {
                infrastructure_count += 1;
                infrastructure_index = Some(index);
            } else if diagnostic.severity == DiagnosticSeverity::Error {
                has_user_error = true;
            }
        }

        match infrastructure_count {
            0 => {
                // No infrastructure payload. A diagnosed module requires at least one user-facing
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
                    render_source_contexts,
                    render_type_contexts,
                })
            }
            1 => {
                // Exactly one infrastructure diagnostic. The legitimate `from_error_with_warnings`
                // shape from AST/HIR stages emits non-error (Warning/Note) companion diagnostics
                // before the single infrastructure failure, so a one-infrastructure sequence may
                // carry companions. Recover the originating `CompilerError` losslessly from its
                // structured payload and location, and discard the companions: the typed `Err`
                // lane aborts the owning compilation and does not surface warnings. A
                // non-infrastructure `Error` diagnostic beside the infrastructure failure is
                // malformed — the current stage contracts never blend a user error with an
                // infrastructure failure — and is reported as an invariant instead of silently
                // dropping the user error. Attach the consumed message set's `StringTable` as the
                // error's render-identity context so the location's interned path IDs remain
                // resolvable after this module-local table is dropped. The later
                // `CompilerMessages::from_error` boundary merges that context into its own table
                // and remaps the location exactly once.
                if has_user_error {
                    return Err(CompilerError::compiler_error(format!(
                        "module semantic stage returned a malformed diagnostic sequence mixing a \
                         user-facing error with an infrastructure failure among {total} total",
                        total = diagnostics.len(),
                    )));
                }

                let Some(infra_index) = infrastructure_index else {
                    // Unreachable: `infrastructure_count == 1` recorded an index above.
                    return Err(CompilerError::compiler_error(
                        "module semantic boundary expected one infrastructure diagnostic but found none",
                    ));
                };
                let Some(diagnostic) = diagnostics.into_iter().nth(infra_index) else {
                    // Unreachable: the recorded index is in range.
                    return Err(CompilerError::compiler_error(
                        "module semantic boundary recorded an out-of-range infrastructure index",
                    ));
                };
                let DiagnosticPayload::InfrastructureError {
                    msg,
                    error_type,
                    metadata,
                } = diagnostic.payload
                else {
                    // Unreachable: the recorded index was counted as infrastructure.
                    return Err(CompilerError::compiler_error(
                        "module semantic boundary classified a non-infrastructure payload as infrastructure",
                    ));
                };

                Err(
                    CompilerError::new(msg, diagnostic.primary_location, error_type)
                        .with_metadata(metadata)
                        .with_render_context(string_table),
                )
            }
            _ => {
                // More than one infrastructure diagnostic is malformed: a stage emits at most one
                // infrastructure failure, so the boundary never has to choose between them.
                Err(CompilerError::compiler_error(format!(
                    "module semantic stage returned a malformed diagnostic sequence with \
                     {infrastructure_count} infrastructure diagnostics among {total} total",
                    total = diagnostics.len(),
                )))
            }
        }
    }

    /// Reconstruct the build/render-boundary `CompilerMessages` from this diagnosed module.
    ///
    /// WHAT: moves the owned diagnostics, string table and render type contexts back into the
    ///       boundary container existing outer callers still consume.
    /// WHY: this is the lossless inverse of `from_messages`. Build and render boundaries keep
    ///       using `CompilerMessages` while the semantic boundary exchanges the typed
    ///       `ModuleDiagnostics` owner.
    pub(crate) fn into_messages(self) -> CompilerMessages {
        CompilerMessages {
            diagnostics: self.diagnostics,
            string_table: self.string_table,
            render_source_contexts: self.render_source_contexts,
            render_type_contexts: self.render_type_contexts,
        }
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

/// Return whether a diagnostic carries an infrastructure payload.
///
/// WHAT: matches the structured `DiagnosticPayload::InfrastructureError` variant only.
/// WHY: this is the single structured signal that a deeper stage routed a `CompilerError` into
///      the mixed `CompilerMessages` boundary, so the classifier never inspects rendered strings,
///      stable codes or `ErrorType` values.
fn is_infrastructure_payload(diagnostic: &CompilerDiagnostic) -> bool {
    matches!(
        diagnostic.payload,
        DiagnosticPayload::InfrastructureError { .. }
    )
}
