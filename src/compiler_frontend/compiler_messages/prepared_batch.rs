//! Prepared diagnostics that cross the premerge boundary without a legacy vessel.
//!
//! WHAT: owns family drafts with captured type-display snapshots, the batch-local string table,
//! explicit path rows, and frozen-identity handles collected during Pass A.
//! WHY: a premerge batch joins the run accumulator without constructing `CompilerMessages` or
//! retaining its live type environment.
use super::compiler_errors::{
    CompilerError, CompilerMessages, InfrastructureErrors, RenderPathContext,
};
use super::diagnostic_bag::{DiagnosticBag, PremergeFailure};
use super::diagnostics::legacy_adapter::{PreparedDiagnostic, prepare_diagnostic};
use crate::compiler_frontend::compiler_messages::DiagnosticSeverity;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::source::{FrozenIdentityHandle, SourceId};
use crate::compiler_frontend::symbols::path_interner::PathTable;
use crate::compiler_frontend::symbols::string_interning::{StringIdRemap, StringTable};
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
pub struct PreparedDiagnosticBatch {
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

        match PreparedDiagnosticBatch::capture_with_required_paths_and_types(
            DiagnosticBag::from_diagnostics(diagnostics),
            *string_table,
            environment,
            || path_table_is_available.then_some(path_table).flatten(),
        ) {
            Ok(batch) => Self::Prepared(batch),
            Err(error) => Self::from(error),
        }
    }

    pub(crate) fn prepend_diagnostics_with_required_paths(
        &mut self,
        prior_diagnostics: impl IntoIterator<
            Item = crate::compiler_frontend::compiler_messages::CompilerDiagnostic,
        >,
        path_table: impl FnOnce() -> Option<Arc<PathTable>>,
    ) -> Result<(), CompilerError> {
        if let Self::Prepared(batch) = self {
            batch.prepend_diagnostics_with_required_paths(prior_diagnostics, path_table)?;
        }
        Ok(())
    }
}

impl From<CompilerError> for AstDiagnosticFailure {
    fn from(error: CompilerError) -> Self {
        Self::Infrastructure(error.into())
    }
}

impl From<AstDiagnosticFailure> for PremergeFailure {
    fn from(failure: AstDiagnosticFailure) -> Self {
        match failure {
            AstDiagnosticFailure::Prepared(batch) => Self::Diagnosed(batch),
            AstDiagnosticFailure::Infrastructure(errors) => Self::Infrastructure(errors),
        }
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
    /// Capture an owning site's bag against the environment it supplies.
    ///
    /// Type displays are captured when the caller marks the environment available. Path rows
    /// are retained independently, only when a snapshot is supplied.
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
    /// Capture diagnostics while retaining a path snapshot only when a payload renders paths.
    ///
    /// Path snapshots are paired with the batch-local string table before capture. The type
    /// environment is consulted only when that pairing succeeds and the snapshot is retained.
    pub(crate) fn capture_with_required_paths(
        bag: DiagnosticBag,
        string_table: StringTable,
        environment: Option<&TypeEnvironment>,
        path_table: impl FnOnce() -> Option<Arc<PathTable>>,
    ) -> Result<Self, CompilerError> {
        Self::capture_with_required_paths_impl(bag, string_table, environment, path_table, false)
    }

    /// Capture path payloads and preserve supplied type facts for path-free diagnostics.
    ///
    /// When an environment is supplied, the retained path context also resolves PathIds embedded
    /// in captured type spellings. The type-free helper above does not retain that context.
    pub(crate) fn capture_with_required_paths_and_types(
        bag: DiagnosticBag,
        string_table: StringTable,
        environment: Option<&TypeEnvironment>,
        path_table: impl FnOnce() -> Option<Arc<PathTable>>,
    ) -> Result<Self, CompilerError> {
        Self::capture_with_required_paths_impl(bag, string_table, environment, path_table, true)
    }

    fn capture_with_required_paths_impl(
        bag: DiagnosticBag,
        string_table: StringTable,
        environment: Option<&TypeEnvironment>,
        path_table: impl FnOnce() -> Option<Arc<PathTable>>,
        capture_types_without_paths: bool,
    ) -> Result<Self, CompilerError> {
        let diagnostics = bag.into_diagnostics();
        let mut required_paths = Vec::new();
        for diagnostic in &diagnostics {
            diagnostic
                .payload
                .required_path_payload_paths(&mut required_paths);
        }

        let retain_type_context =
            capture_types_without_paths && environment.is_some() && !diagnostics.is_empty();
        let path_table = if required_paths.is_empty() && !retain_type_context {
            None
        } else {
            path_table()
        };
        if let Some(path_table) = &path_table
            && let Some(index) = super::compiler_errors::unpaired_path_table_component_index(
                path_table,
                &string_table,
                &required_paths,
            )
        {
            return Err(CompilerError::compiler_error(format!(
                "diagnostic path table is not pairable with this string table: path node \
                 {index} is unaddressable or carries a component StringId the string table \
                 cannot resolve"
            )));
        }
        let type_environment_is_available = path_table.is_some();
        let batch = Self::capture_from_bag(
            DiagnosticBag::from_diagnostics(diagnostics),
            string_table,
            environment,
            type_environment_is_available,
            path_table,
        );
        if let Some(path_table) = batch
            .identity
            .path_rows
            .first()
            .map(|row| row.path_table.as_ref())
        {
            let mut type_paths = Vec::new();
            for diagnostic in &batch.prepared {
                diagnostic.append_type_paths(&mut type_paths);
            }
            if let Some(index) = super::compiler_errors::unpaired_path_table_component_index(
                path_table,
                &batch.identity.string_table,
                &type_paths,
            ) {
                return Err(CompilerError::compiler_error(format!(
                    "diagnostic path table is not pairable with this string table: path node \
                     {index} is unaddressable or carries a component StringId the string table \
                     cannot resolve"
                )));
            }
        }
        Ok(batch)
    }
    pub(crate) fn rebind_source_ids(
        &mut self,
        rebind: &mut impl FnMut(SourceId) -> Result<SourceId, CompilerError>,
    ) -> Result<(), CompilerError> {
        for diagnostic in &mut self.prepared {
            diagnostic.rebind_source_ids(rebind)?;
        }
        Ok(())
    }

    /// Rebase every prepared string ID onto the run root table and release this batch's local
    /// identity table. After this call, all string IDs belong to the run root table; append the
    /// batch only to an accumulator initialized with that same table.
    pub(crate) fn rebase_strings(&mut self, remap: &StringIdRemap) {
        if !remap.is_identity() {
            for diagnostic in &mut self.prepared {
                diagnostic.remap_string_ids(remap);
            }
            for row in &mut self.identity.path_rows {
                Arc::make_mut(&mut row.path_table).remap_string_ids(remap);
            }
        }
        self.identity.string_table = StringTable::empty_unallocated();
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

        let mut prepared = Vec::with_capacity(self.prepared.len() + shift);
        for diagnostic in &prior_diagnostics {
            let mut item =
                prepare_diagnostic(diagnostic, None, &mut self.identity.required_handles);
            item.set_explicit_path(None);
            prepared.push(item);
        }
        prepared.append(&mut self.prepared);
        self.prepared = prepared;

        for row in &mut self.identity.path_rows {
            row.diagnostic_range.start += shift;
            row.diagnostic_range.end += shift;
        }
    }

    pub(crate) fn prepend_diagnostics_with_required_paths(
        &mut self,
        prior_diagnostics: impl IntoIterator<
            Item = crate::compiler_frontend::compiler_messages::CompilerDiagnostic,
        >,
        path_table: impl FnOnce() -> Option<Arc<PathTable>>,
    ) -> Result<(), CompilerError> {
        let prior_diagnostics = prior_diagnostics.into_iter().collect::<Vec<_>>();
        let shift = prior_diagnostics.len();
        if shift == 0 {
            return Ok(());
        }

        let mut prior_batch = Self::capture_with_required_paths(
            DiagnosticBag::from_diagnostics(prior_diagnostics),
            std::mem::take(&mut self.identity.string_table),
            None,
            path_table,
        )?;
        let prior_path_rows = std::mem::take(&mut prior_batch.identity.path_rows);
        let prior_path_index = self.identity.path_rows.len();
        if !prior_path_rows.is_empty() {
            for diagnostic in &mut prior_batch.prepared {
                diagnostic.set_explicit_path(Some(prior_path_index));
            }
        }

        for row in &mut self.identity.path_rows {
            row.diagnostic_range.start += shift;
            row.diagnostic_range.end += shift;
        }
        self.identity.path_rows.extend(prior_path_rows);

        let mut prepared = Vec::with_capacity(shift + self.prepared.len());
        prepared.append(&mut prior_batch.prepared);
        prepared.append(&mut self.prepared);
        self.prepared = prepared;

        let mut required_handles = std::mem::take(&mut prior_batch.identity.required_handles);
        required_handles.append(&mut self.identity.required_handles);
        self.identity.required_handles = required_handles;
        self.identity.string_table = std::mem::take(&mut prior_batch.identity.string_table);
        Ok(())
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
