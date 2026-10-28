//! Prepared diagnostics that cross the pre-freeze boundary as a compact batch.
//!
//! WHAT: owns family drafts with captured type-display snapshots, an explicit string-table
//! domain, path rows, and frozen-identity owners carried by drafts.
//! WHY: the boundary accumulator can merge prepared facts without retaining live stage context.
use super::compiler_errors::InfrastructureFailure;
use super::diagnostic_bag::DiagnosticBag;
use super::diagnostics::draft::DiagnosticDraft;
use super::diagnostics::prepared::{PreparedDiagnostic, prepare_draft};
use crate::compiler_frontend::compiler_messages::{CompilerStage, DiagnosticSeverity};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::source::{FrozenIdentityHandle, SourceId};
use crate::compiler_frontend::symbols::path_interner::{PathId, PathTable};
use crate::compiler_frontend::symbols::string_interning::{StringIdRemap, StringTable};
use std::ops::Range;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub(super) struct PreparedPathRow {
    pub(super) diagnostic_range: Range<usize>,
    pub(super) path_table: Arc<PathTable>,
}

/// Identity ownership for one prepared batch.
pub(super) enum PreparedBatchIdentity {
    /// Module-local IDs that must be merged into the boundary root.
    Local {
        string_table: StringTable,
        path_rows: Vec<PreparedPathRow>,
    },
    /// IDs already belong to the boundary root and must not be remapped.
    Root { path_rows: Vec<PreparedPathRow> },
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
) -> Result<(), InfrastructureFailure> {
    if let Some(index) = super::compiler_errors::unpaired_path_table_component_index(
        path_table,
        string_table,
        required_paths,
    ) {
        return Err(InfrastructureFailure::internal(
            CompilerStage::DiagnosticReporting,
            format!(
                "diagnostic path table is not pairable with this string table: path node \
             {index} is unaddressable or carries a component StringId the string table \
             cannot resolve"
            ),
        ));
    }
    Ok(())
}

impl PreparedDiagnosticBatch {
    fn prepare_diagnostics(
        diagnostics: Vec<DiagnosticDraft>,
        environment: Option<&TypeEnvironment>,
    ) -> Vec<PreparedDiagnostic> {
        diagnostics
            .into_iter()
            .map(|draft| prepare_draft(draft, environment))
            .collect()
    }

    fn prepare_diagnostics_by_environment<'environment>(
        owner_diagnostics: impl IntoIterator<
            Item = (Vec<DiagnosticDraft>, Option<&'environment TypeEnvironment>),
        >,
    ) -> (Vec<PreparedDiagnostic>, Vec<PathId>) {
        let mut prepared = Vec::new();
        let mut required_paths = Vec::new();
        for (diagnostics, environment) in owner_diagnostics {
            for draft in diagnostics {
                draft.append_required_paths(&mut required_paths);
                let diagnostic = prepare_draft(draft, environment);
                diagnostic.append_type_paths(&mut required_paths);
                prepared.push(diagnostic);
            }
        }
        (prepared, required_paths)
    }

    fn attach_required_path_rows(
        prepared: &mut [PreparedDiagnostic],
        required_paths: Vec<PathId>,
        string_table: &StringTable,
        path_table: impl FnOnce() -> Option<Arc<PathTable>>,
    ) -> Result<Vec<PreparedPathRow>, InfrastructureFailure> {
        let path_table = if required_paths.is_empty() {
            None
        } else {
            path_table()
        };
        if let Some(path_table) = &path_table {
            validate_path_table_pairing(path_table, string_table, &required_paths)?;
        }
        Ok(Self::attach_path_table(prepared, path_table))
    }

    fn attach_path_table(
        prepared: &mut [PreparedDiagnostic],
        path_table: Option<Arc<PathTable>>,
    ) -> Vec<PreparedPathRow> {
        let Some(path_table) = path_table.filter(|_| !prepared.is_empty()) else {
            return Vec::new();
        };
        for diagnostic in prepared.iter_mut() {
            diagnostic.set_explicit_path(Some(0));
        }
        vec![PreparedPathRow {
            diagnostic_range: 0..prepared.len(),
            path_table,
        }]
    }

    /// Capture an owning site's bag against the environment it supplies.
    ///
    /// Type displays use the supplied environment independently of path-row availability.
    pub(crate) fn capture_from_bag(
        bag: DiagnosticBag,
        string_table: StringTable,
        environment: Option<&TypeEnvironment>,
        path_table: Option<Arc<PathTable>>,
    ) -> Self {
        let mut bag = bag;
        bag.retain_errors_only_if_failed();
        let mut prepared = Self::prepare_diagnostics(bag.into_diagnostics(), environment);
        let path_rows = Self::attach_path_table(&mut prepared, path_table);

        Self {
            prepared,
            identity: Box::new(PreparedBatchIdentity::Local {
                string_table,
                path_rows,
            }),
        }
    }

    /// Capture diagnostics already using the run root's string IDs.
    ///
    /// Root-domain identity is structural: the batch owns path rows but no local string table.
    /// It may only be appended to an accumulator seeded with the same root table.
    pub(crate) fn capture_from_root_string_table(
        bag: DiagnosticBag,
        root_string_table: &StringTable,
        environment: Option<&TypeEnvironment>,
        path_table: impl FnOnce() -> Option<Arc<PathTable>>,
    ) -> Result<Self, InfrastructureFailure> {
        let mut bag = bag;
        bag.retain_errors_only_if_failed();
        let (mut prepared, required_paths) =
            Self::prepare_diagnostics_by_environment([(bag.into_diagnostics(), environment)]);
        let path_rows = Self::attach_required_path_rows(
            &mut prepared,
            required_paths,
            root_string_table,
            path_table,
        )?;
        Ok(Self {
            prepared,
            identity: Box::new(PreparedBatchIdentity::Root { path_rows }),
        })
    }

    /// Capture diagnostics while retaining path snapshots for rendered or captured path IDs.
    ///
    /// The environment is used whenever supplied; explicit diagnostic paths and type-display
    /// paths independently determine whether a path row is retained.
    pub(crate) fn capture_with_required_paths(
        bag: DiagnosticBag,
        string_table: StringTable,
        environment: Option<&TypeEnvironment>,
        path_table: impl FnOnce() -> Option<Arc<PathTable>>,
    ) -> Result<Self, InfrastructureFailure> {
        let mut bag = bag;
        bag.retain_errors_only_if_failed();
        let (mut prepared, required_paths) =
            Self::prepare_diagnostics_by_environment([(bag.into_diagnostics(), environment)]);
        let path_rows = Self::attach_required_path_rows(
            &mut prepared,
            required_paths,
            &string_table,
            path_table,
        )?;
        Ok(Self {
            prepared,
            identity: Box::new(PreparedBatchIdentity::Local {
                string_table,
                path_rows,
            }),
        })
    }
    /// Capture ordered owner diagnostics with one shared local identity table.
    ///
    /// Each owner's type displays use its own live environment; path requirements share the
    /// supplied table snapshot because the owners share this string/path identity domain.
    pub(crate) fn capture_with_required_paths_for_owners<'environment>(
        owner_diagnostics: impl IntoIterator<
            Item = (Vec<DiagnosticDraft>, &'environment TypeEnvironment),
        >,
        string_table: StringTable,
        path_table: impl FnOnce() -> Option<Arc<PathTable>>,
    ) -> Result<Self, InfrastructureFailure> {
        let (mut prepared, required_paths) = Self::prepare_diagnostics_by_environment(
            owner_diagnostics
                .into_iter()
                .map(|(diagnostics, environment)| (diagnostics, Some(environment))),
        );
        let path_rows = Self::attach_required_path_rows(
            &mut prepared,
            required_paths,
            &string_table,
            path_table,
        )?;
        Ok(Self {
            prepared,
            identity: Box::new(PreparedBatchIdentity::Local {
                string_table,
                path_rows,
            }),
        })
    }
    pub(crate) fn rebind_source_ids(
        &mut self,
        rebind: &mut impl FnMut(SourceId) -> Result<SourceId, InfrastructureFailure>,
    ) -> Result<(), InfrastructureFailure> {
        for diagnostic in &mut self.prepared {
            diagnostic.rebind_source_ids(rebind)?;
        }
        Ok(())
    }

    /// Rebase every prepared string ID onto the run root table and release this batch's local
    /// identity table. After this call, all string IDs belong to the run root table; append the
    /// batch only to an accumulator initialized with that same table.
    pub(crate) fn rebase_strings(&mut self, remap: &StringIdRemap) {
        let identity = std::mem::replace(
            &mut *self.identity,
            PreparedBatchIdentity::Root {
                path_rows: Vec::new(),
            },
        );
        let PreparedBatchIdentity::Local {
            string_table: _,
            mut path_rows,
        } = identity
        else {
            crate::compiler_bug!(
                CompilerStage::DiagnosticReporting,
                "root-domain diagnostics cannot be rebased as module-local"
            );
        };
        if !remap.is_identity() {
            for diagnostic in &mut self.prepared {
                diagnostic.remap_string_ids(remap);
            }
            for row in &mut path_rows {
                Arc::make_mut(&mut row.path_table).remap_string_ids(remap);
            }
        }
        *self.identity = PreparedBatchIdentity::Root { path_rows };
    }

    pub(crate) fn classify_premerge(self) -> Result<Self, InfrastructureFailure> {
        if !self.has_errors() {
            if self.prepared.is_empty() {
                return Err(InfrastructureFailure::internal(
                    CompilerStage::DiagnosticReporting,
                    "module semantic stage returned a premerge batch with no diagnostics",
                ));
            }
            return Err(InfrastructureFailure::internal(
                CompilerStage::DiagnosticReporting,
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

    pub(crate) fn attach_frozen_identity_handle_if_missing(
        &mut self,
        handle: FrozenIdentityHandle,
    ) {
        for diagnostic in &mut self.prepared {
            diagnostic.attach_frozen_identity_handle_if_missing(handle.clone());
        }
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
