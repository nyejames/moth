//! Prepared diagnostics that cross the premerge boundary without a legacy vessel.
//!
//! WHAT: owns family drafts with captured type-display snapshots, the batch-local string table,
//! explicit path rows, and frozen-identity handles collected during Pass A.
//! WHY: a premerge batch joins the run accumulator without constructing `CompilerMessages` or
//! retaining its live type environment.

use super::compiler_errors::{
    CompilerError, CompilerMessages, InfrastructureErrors, RenderPathContext, RenderTypeContext,
};
use super::diagnostic_bag::{DiagnosticBag, PremergeDiagnosticBatch, PremergeFailure};
use super::diagnostics::legacy_adapter::{PreparedDiagnostic, prepare_diagnostic};
use crate::compiler_frontend::compiler_messages::DiagnosticSeverity;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::source::FrozenIdentityHandle;
use crate::compiler_frontend::symbols::path_interner::PathTable;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use std::sync::Arc;

/// Batch-local string, path, and frozen-identity ownership.
pub(super) struct PreparedBatchIdentity {
    pub(super) string_table: StringTable,
    pub(super) path_rows: Vec<RenderPathContext>,
    pub(super) required_handles: Vec<FrozenIdentityHandle>,
}

/// Move-only premerge diagnostics ready to append directly to a boundary run.
///
/// The boxed identity side store keeps this carrier at most 48 bytes. Each draft owns its captured
/// type snapshot so append remaps the pair in place without a second conversion.
pub(crate) struct PreparedDiagnosticBatch {
    pub(super) prepared: Vec<PreparedDiagnostic>,
    pub(super) identity: Box<PreparedBatchIdentity>,
}

#[derive(Debug)]
pub(crate) enum AstDiagnosticFailure {
    Prepared(PreparedDiagnosticBatch),
    Infrastructure(Box<InfrastructureErrors>),
}

impl AstDiagnosticFailure {
    pub(crate) fn capture_messages(
        messages: CompilerMessages,
        environment: Option<&TypeEnvironment>,
        path_table_is_available: bool,
        path_table: Option<Arc<PathTable>>,
    ) -> Self {
        let CompilerMessages {
            diagnostics,
            infrastructure_errors,
            string_table,
            ..
        } = messages;
        if let Some(errors) = infrastructure_errors {
            return Self::Infrastructure(errors);
        }

        Self::Prepared(PreparedDiagnosticBatch::capture_from_bag(
            DiagnosticBag::from_diagnostics(diagnostics),
            *string_table,
            environment,
            path_table_is_available,
            path_table,
        ))
    }

    pub(crate) fn prepend_diagnostics(
        &mut self,
        prior_diagnostics: impl IntoIterator<
            Item = crate::compiler_frontend::compiler_messages::CompilerDiagnostic,
        >,
    ) {
        if let Self::Prepared(batch) = self {
            batch.prepend_diagnostics(prior_diagnostics);
        }
    }
}

impl From<CompilerError> for AstDiagnosticFailure {
    fn from(error: CompilerError) -> Self {
        Self::Infrastructure(error.into())
    }
}

/// Typed failures from the semantic pipeline before its owning boundary finalizes sources.
#[derive(Debug)]
pub(crate) enum SemanticStageFailure {
    Premerge(PremergeFailure),
    Ast(AstDiagnosticFailure),
}

impl From<PremergeFailure> for SemanticStageFailure {
    fn from(failure: PremergeFailure) -> Self {
        Self::Premerge(failure)
    }
}

impl From<CompilerError> for SemanticStageFailure {
    fn from(error: CompilerError) -> Self {
        Self::Premerge(PremergeFailure::from(error))
    }
}

impl From<AstDiagnosticFailure> for SemanticStageFailure {
    fn from(failure: AstDiagnosticFailure) -> Self {
        Self::Ast(failure)
    }
}

impl std::fmt::Debug for PreparedDiagnosticBatch {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PreparedDiagnosticBatch")
            .field("diagnostic_count", &self.prepared.len())
            .finish_non_exhaustive()
    }
}

const _: () = assert!(std::mem::size_of::<PreparedDiagnosticBatch>() <= 48);

impl PreparedDiagnosticBatch {
    /// Capture a premerge batch against its own retained contexts using the legacy adapter's Pass A.
    pub(crate) fn from_premerge(batch: PremergeDiagnosticBatch) -> Self {
        let (bag, string_table, type_rows, path_rows) = batch.into_parts();
        let diagnostics = bag.into_diagnostics();
        let mut required_handles = Vec::new();
        let mut prepared = Vec::with_capacity(diagnostics.len());

        for (index, diagnostic) in diagnostics.iter().enumerate() {
            let explicit_path = path_rows
                .iter()
                .position(|row| row.diagnostic_range.contains(&index));
            let environment = explicit_path
                .and_then(|_| {
                    type_rows
                        .iter()
                        .find(|row| row.diagnostic_range.contains(&index))
                })
                .map(|row: &RenderTypeContext| &row.type_environment);
            let mut item = prepare_diagnostic(diagnostic, environment, &mut required_handles);
            item.set_explicit_path(explicit_path);
            prepared.push(item);
        }

        Self {
            prepared,
            identity: Box::new(PreparedBatchIdentity {
                string_table,
                path_rows,
                required_handles,
            }),
        }
    }
    /// Capture an owning site's bag against the environment it supplies.
    ///
    /// Unlike `from_premerge`, this site already knows whether its path table was available while
    /// the environment was live. Capture every diagnostic against that borrowed environment when
    /// it was available; do not infer availability from a retained path row.
    pub(crate) fn capture_from_bag(
        bag: DiagnosticBag,
        string_table: StringTable,
        environment: Option<&TypeEnvironment>,
        path_table_is_available: bool,
        path_table: Option<Arc<PathTable>>,
    ) -> Self {
        let diagnostics = bag.into_diagnostics();
        let environment = if path_table_is_available {
            environment
        } else {
            None
        };
        let has_path_row = path_table.is_some() && !diagnostics.is_empty();
        let mut required_handles = Vec::new();
        let mut prepared = Vec::with_capacity(diagnostics.len());
        for diagnostic in &diagnostics {
            let mut item = prepare_diagnostic(diagnostic, environment, &mut required_handles);
            item.set_explicit_path(has_path_row.then_some(0));
            prepared.push(item);
        }
        let path_rows = match path_table {
            Some(path_table) if has_path_row => vec![RenderPathContext {
                diagnostic_range: 0..diagnostics.len(),
                path_table,
            }],
            _ => Vec::new(),
        };

        Self {
            prepared,
            identity: Box::new(PreparedBatchIdentity {
                string_table,
                path_rows,
                required_handles,
            }),
        }
    }
    pub(crate) fn prepend_diagnostics(
        &mut self,
        prior_diagnostics: impl IntoIterator<
            Item = crate::compiler_frontend::compiler_messages::CompilerDiagnostic,
        >,
    ) {
        let prior_diagnostics = prior_diagnostics.into_iter().collect::<Vec<_>>();
        let shift = prior_diagnostics.len();
        if shift == 0 {
            return;
        }

        let leading_path = self
            .identity
            .path_rows
            .iter()
            .position(|row| row.diagnostic_range.start == 0);
        let mut prepared = Vec::with_capacity(self.prepared.len() + shift);
        for diagnostic in &prior_diagnostics {
            let mut item =
                prepare_diagnostic(diagnostic, None, &mut self.identity.required_handles);
            item.set_explicit_path(leading_path);
            prepared.push(item);
        }
        prepared.append(&mut self.prepared);
        self.prepared = prepared;

        for (index, row) in self.identity.path_rows.iter_mut().enumerate() {
            if Some(index) == leading_path {
                row.diagnostic_range.start = 0;
                row.diagnostic_range.end += shift;
            } else {
                row.diagnostic_range.start += shift;
                row.diagnostic_range.end += shift;
            }
        }
    }

    pub(crate) fn classify_premerge(self) -> Result<Self, CompilerError> {
        if !self.has_errors() {
            if self.prepared.is_empty() {
                return Err(CompilerError::compiler_error(
                    "module semantic stage returned a premerge batch with no diagnostics",
                ));
            }
            return Err(CompilerError::compiler_error(
                "module semantic stage returned a premerge batch with no user-facing error diagnostic",
            ));
        }
        Ok(self)
    }

    pub(crate) fn has_errors(&self) -> bool {
        self.prepared
            .iter()
            .any(|diagnostic| diagnostic.severity() == DiagnosticSeverity::Error)
    }

    pub(crate) fn diagnostic_count(&self) -> usize {
        self.prepared.len()
    }

    #[cfg(feature = "data_layout_memory_probe")]
    pub(crate) fn label_count(&self) -> usize {
        self.prepared
            .iter()
            .map(PreparedDiagnostic::label_count)
            .sum()
    }

    pub(crate) fn requires_source_identity(&self) -> bool {
        self.prepared
            .iter()
            .any(PreparedDiagnostic::requires_source_identity)
    }
}

/// Classify a legacy semantic failure while keeping its contexts in the pre-Pass-A lane.
///
/// The diagnosed premerge carrier still owns the type environment until its last path-table
/// pairing is known. Capturing here would run Pass A before that owning-site boundary.
pub(crate) fn classify_module_messages(
    messages: CompilerMessages,
) -> Result<PremergeDiagnosticBatch, Box<InfrastructureErrors>> {
    let CompilerMessages {
        diagnostics,
        infrastructure_errors,
        string_table,
        render_source_contexts,
        render_type_contexts,
        render_path_contexts,
        ..
    } = messages;

    let has_user_error = diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error);

    match infrastructure_errors {
        None => {
            if !has_user_error {
                if diagnostics.is_empty() {
                    return Err(CompilerError::compiler_error(
                        "module semantic stage returned a failure with no diagnostics",
                    )
                    .into());
                }
                return Err(CompilerError::compiler_error(
                    "module semantic stage returned a failure with no user-facing error diagnostic",
                )
                .into());
            }

            if !render_source_contexts.is_empty() {
                return Err(CompilerError::compiler_error(format!(
                    "diagnosed module reached canonical merging with {} attached source contexts \
                     beside {} diagnostics; source attachment must happen after merging",
                    render_source_contexts.len(),
                    diagnostics.len(),
                ))
                .into());
            }

            Ok(PremergeDiagnosticBatch::from_parts(
                diagnostics,
                *string_table,
                render_type_contexts,
                render_path_contexts.map_or_else(Vec::new, |contexts| contexts.into_vec()),
            ))
        }
        Some(mut errors) => {
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
