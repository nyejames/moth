//! Prepared diagnostics that cross the pre-freeze boundary as a compact batch.
//!
//! WHAT: owns family drafts with captured type-display snapshots, the batch-local string table,
//! explicit path rows, and frozen-identity owners carried by drafts.
//! WHY: the boundary accumulator can merge prepared facts without retaining live stage context.
use super::compiler_errors::CompilerError;
use super::diagnostic_bag::DiagnosticBag;
use super::diagnostics::draft::DiagnosticDraft;
use super::diagnostics::prepared::{PreparedDiagnostic, prepare_draft};
use crate::compiler_frontend::compiler_messages::DiagnosticSeverity;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::source::SourceId;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathTable};
use crate::compiler_frontend::symbols::string_interning::{StringIdRemap, StringTable};
use std::ops::Range;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub(super) struct PreparedPathRow {
    pub(super) diagnostic_range: Range<usize>,
    pub(super) path_table: Arc<PathTable>,
}

/// Batch-local string and path ownership.
pub(super) struct PreparedBatchIdentity {
    pub(super) string_table: StringTable,
    pub(super) path_rows: Vec<PreparedPathRow>,
}

/// Move-only pre-freeze diagnostics ready to append directly to a boundary run.
///
/// The boxed identity side store keeps this carrier at most 48 bytes. Each draft owns its captured
/// type snapshot so append remaps the pair in place without a second conversion.
pub struct PreparedDiagnosticBatch {
    pub(super) prepared: Vec<PreparedDiagnostic>,
    pub(super) identity: Box<PreparedBatchIdentity>,
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

fn validate_path_table_pairing(
    path_table: &PathTable,
    string_table: &StringTable,
    required_paths: &[PathId],
) -> Result<(), CompilerError> {
    if let Some(index) = super::compiler_errors::unpaired_path_table_component_index(
        path_table,
        string_table,
        required_paths,
    ) {
        return Err(CompilerError::compiler_error(format!(
            "diagnostic path table is not pairable with this string table: path node \
             {index} is unaddressable or carries a component StringId the string table \
             cannot resolve"
        )));
    }
    Ok(())
}

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
        let diagnostic_count = diagnostics.len();
        let environment = if path_table_is_available {
            environment
        } else {
            None
        };
        let has_path_row = path_table.is_some() && diagnostic_count != 0;
        let mut prepared = Vec::with_capacity(diagnostic_count);
        for draft in diagnostics {
            let mut item = prepare_draft(draft, environment);
            item.set_explicit_path(has_path_row.then_some(0));
            prepared.push(item);
        }
        let path_rows = match path_table {
            Some(path_table) if has_path_row => vec![PreparedPathRow {
                diagnostic_range: 0..diagnostic_count,
                path_table,
            }],
            _ => Vec::new(),
        };

        Self {
            prepared,
            identity: Box::new(PreparedBatchIdentity {
                string_table,
                path_rows,
            }),
        }
    }
    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "P4.5 S7b backend root-string diagnostics")
    )]
    /// Capture diagnostics already using the run root's string IDs.
    ///
    /// The returned batch owns an empty, unallocated string table: its diagnostic and path-table
    /// IDs remain in `root_string_table`'s domain and it may only be appended to an accumulator
    /// seeded with that same root table.
    pub(crate) fn capture_from_root_string_table(
        bag: DiagnosticBag,
        root_string_table: &StringTable,
        environment: Option<&TypeEnvironment>,
        path_table: impl FnOnce() -> Option<Arc<PathTable>>,
    ) -> Result<Self, CompilerError> {
        let diagnostics = bag.into_diagnostics();
        let mut required_paths = Vec::new();
        for diagnostic in &diagnostics {
            diagnostic.append_required_paths(&mut required_paths);
        }

        let retain_type_context = environment.is_some() && !diagnostics.is_empty();
        let path_table = if required_paths.is_empty() && !retain_type_context {
            None
        } else {
            path_table()
        };
        if let Some(path_table) = &path_table {
            validate_path_table_pairing(path_table, root_string_table, &required_paths)?;
        }

        let batch = Self::capture_from_bag(
            DiagnosticBag::from_diagnostics(diagnostics),
            StringTable::empty_unallocated(),
            environment,
            path_table.is_some(),
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
            validate_path_table_pairing(path_table, root_string_table, &type_paths)?;
        }
        Ok(batch)
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
            diagnostic.append_required_paths(&mut required_paths);
        }

        let retain_type_context =
            capture_types_without_paths && environment.is_some() && !diagnostics.is_empty();
        let path_table = if required_paths.is_empty() && !retain_type_context {
            None
        } else {
            path_table()
        };
        if let Some(path_table) = &path_table {
            validate_path_table_pairing(path_table, &string_table, &required_paths)?;
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
            validate_path_table_pairing(path_table, &batch.identity.string_table, &type_paths)?;
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

    pub(crate) fn prepend_diagnostics<D>(&mut self, prior_diagnostics: impl IntoIterator<Item = D>)
    where
        D: Into<DiagnosticDraft>,
    {
        let prior_diagnostics = prior_diagnostics
            .into_iter()
            .map(Into::into)
            .collect::<Vec<_>>();
        let shift = prior_diagnostics.len();
        if shift == 0 {
            return;
        }

        let mut prepared = Vec::with_capacity(self.prepared.len() + shift);
        for draft in prior_diagnostics {
            let mut item = prepare_draft(draft, None);
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
    pub(crate) fn prepend_diagnostics_with_required_paths<D>(
        &mut self,
        prior_diagnostics: impl IntoIterator<Item = D>,
        path_table: impl FnOnce() -> Option<Arc<PathTable>>,
    ) -> Result<(), CompilerError>
    where
        D: Into<DiagnosticDraft>,
    {
        let prior_diagnostics = prior_diagnostics
            .into_iter()
            .map(Into::into)
            .collect::<Vec<_>>();
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
