//! Run-scoped merge, ownership capture, and canonical freeze for prepared diagnostics.
//!
//! WHAT: joins batches and infrastructure causes, resolves identity/path owners, then freezes once.
//! WHY: source and path ownership is resolved after the complete diagnostic order is known.

use super::draft::DiagnosticDraft;
use super::freeze::{DomainSource, InstallMode, PendingDiagnostics};
use super::prepared::PreparedDiagnostic;
use super::resource::DiagnosticLimits;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::compiler_errors::{
    CompilationOutcome, InfrastructureErrors,
};
use crate::compiler_frontend::compiler_messages::diagnostic_types::type_display_snapshot::TypeDisplaySnapshot;
use crate::compiler_frontend::compiler_messages::diagnostics::report::DiagnosticReportSet;
use crate::compiler_frontend::compiler_messages::prepared_batch::{
    PreparedBatchIdentity, PreparedDiagnosticBatch, PreparedPathRow,
};
use crate::compiler_frontend::compiler_messages::{
    DiagnosticCapacityResource, SourceSpanCapacityResource,
};
use crate::compiler_frontend::semantic_identity::StablePackageIdentity;
use crate::compiler_frontend::source::SourceDatabase;
use crate::compiler_frontend::symbols::path_interner::{
    PathIdRemap, PathInternError, PathInternerBuilder,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use std::ops::Range;
use std::sync::Arc;
/// A source row that the converter keeps: its identity, and the slot of its database.
struct KeptSourceRow {
    domain: Option<StablePackageIdentity>,
    slot: usize,
}

/// One `merge_snapshot` call into a database's builder. Diagnostics that name the same table share it.
struct PathMerge {
    slot: usize,
    /// The first path row that named the table. Later rows with the same allocation reuse this merge.
    first_path: usize,
    outcome: Result<PathIdRemap, PathInternError>,
}

#[derive(Debug)]
struct SourceRow {
    diagnostic_range: Range<usize>,
    source_database: Arc<SourceDatabase>,
    domain: Option<StablePackageIdentity>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SuccessfulWarningBoundary {
    Project,
    SourcePackage(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SuccessfulWarningPosition {
    #[cfg(test)]
    Configuration,
    BaseModule(usize),
    GeneratedSidecar(usize),
    /// Warnings with no canonical graph slot follow all project/package module warnings.
    CommandTail,
}

struct PendingSuccessfulWarning {
    boundary: SuccessfulWarningBoundary,
    position: SuccessfulWarningPosition,
    sequence: usize,
    batch: PreparedDiagnosticBatch,
    source_package_identity: Option<StablePackageIdentity>,
    source: Option<(Arc<SourceDatabase>, Option<StablePackageIdentity>)>,
    committed: bool,
}

#[derive(Default)]
struct SuccessfulWarningStore {
    batches: Vec<PendingSuccessfulWarning>,
    identity_domains: Vec<(StablePackageIdentity, Arc<SourceDatabase>)>,
    next_sequence: usize,
    active_source_package: Option<(SuccessfulWarningBoundary, StablePackageIdentity)>,
}

impl SuccessfulWarningStore {
    fn commit_boundary(
        &mut self,
        boundary: SuccessfulWarningBoundary,
        source_database: Option<Arc<SourceDatabase>>,
    ) {
        for warning in &mut self.batches {
            if warning.boundary != boundary {
                continue;
            }
            warning.source = match boundary {
                SuccessfulWarningBoundary::Project => source_database
                    .as_ref()
                    .map(|source| (Arc::clone(source), None)),
                SuccessfulWarningBoundary::SourcePackage(_) => {
                    let source = source_database
                        .as_ref()
                        .expect("completed source-package warnings require their source database");
                    let identity = warning.source_package_identity.clone().expect(
                        "source-package warnings require their package identity at staging",
                    );
                    Some((Arc::clone(source), Some(identity)))
                }
            };
            warning.committed = true;
        }
    }
    fn retain_identity_domain(
        &mut self,
        identity: StablePackageIdentity,
        source_database: Arc<SourceDatabase>,
    ) {
        if !self
            .batches
            .iter()
            .any(|warning| warning.batch.diagnostic_count() != 0)
            || self
                .identity_domains
                .iter()
                .any(|(retained, _)| *retained == identity)
        {
            return;
        }
        self.identity_domains.push((identity, source_database));
    }
}

/// Borrowed command-owned sink for warnings staged during frontend publication.
pub(crate) struct SuccessfulWarningSink<'a> {
    store: &'a mut SuccessfulWarningStore,
}

impl SuccessfulWarningSink<'_> {
    pub(crate) fn set_source_package_identity(
        &mut self,
        boundary: SuccessfulWarningBoundary,
        package_identity: StablePackageIdentity,
    ) {
        assert!(
            matches!(boundary, SuccessfulWarningBoundary::SourcePackage(_)),
            "only source-package warnings have a package identity",
        );
        self.store.active_source_package = Some((boundary, package_identity));
    }

    pub(crate) fn append(
        &mut self,
        boundary: SuccessfulWarningBoundary,
        position: SuccessfulWarningPosition,
        batch: PreparedDiagnosticBatch,
    ) {
        let source_package_identity = match boundary {
            SuccessfulWarningBoundary::Project => None,
            SuccessfulWarningBoundary::SourcePackage(_) => {
                let (active_boundary, identity) = self
                    .store
                    .active_source_package
                    .as_ref()
                    .filter(|(active_boundary, _)| *active_boundary == boundary)
                    .expect("source-package warning staging requires its package owner");
                debug_assert_eq!(*active_boundary, boundary);
                Some(identity.clone())
            }
        };
        let sequence = self.store.next_sequence;
        self.store.next_sequence += 1;
        self.store.batches.push(PendingSuccessfulWarning {
            boundary,
            position,
            sequence,
            batch,
            source_package_identity,
            source: None,
            committed: false,
        });
    }

    pub(crate) fn commit_boundary(
        &mut self,
        boundary: SuccessfulWarningBoundary,
        source_database: Option<Arc<SourceDatabase>>,
    ) {
        self.store.commit_boundary(boundary, source_database);
    }
}

/// Accumulates prepared diagnostics and infrastructure causes before one canonical freeze.
///
/// Empty construction has no allocations; the first local prepared append adopts its table.
pub(crate) struct BoundaryAccumulator {
    state: Option<BoundaryAccumulatorState>,
    successful_warnings: SuccessfulWarningStore,
    #[cfg(test)]
    test_limits: Option<DiagnosticLimits>,
}

struct BoundaryAccumulatorState {
    string_table: StringTable,
    prepared: Vec<PreparedDiagnostic>,
    source_rows: Vec<SourceRow>,
    path_rows: Vec<PreparedPathRow>,
    infrastructure_errors: Option<Box<InfrastructureErrors>>,
}

impl BoundaryAccumulator {
    /// Creates an empty accumulator without allocating.
    pub(crate) fn new() -> Self {
        Self {
            state: None,
            successful_warnings: SuccessfulWarningStore::default(),
            #[cfg(test)]
            test_limits: None,
        }
    }

    /// Creates an accumulator by taking ownership of the run's root string table.
    ///
    /// Frozen diagnostic identity resolves interned strings and paths through this table.
    /// Infrastructure failures do not carry source databases or compact source identity.
    pub(crate) fn with_root_string_table(root: StringTable) -> Self {
        Self {
            state: Some(BoundaryAccumulatorState {
                string_table: root,
                prepared: Vec::new(),
                source_rows: Vec::new(),
                path_rows: Vec::new(),
                infrastructure_errors: None,
            }),
            successful_warnings: SuccessfulWarningStore::default(),
            #[cfg(test)]
            test_limits: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn set_record_limit_for_test(&mut self, records: usize) {
        self.test_limits = Some(DiagnosticLimits {
            records,
            ..DiagnosticLimits::production()
        });
    }

    /// Split the command's compiler identity and warning owners for frontend publication.
    pub(crate) fn compiler_parts_mut(&mut self) -> (&mut StringTable, SuccessfulWarningSink<'_>) {
        let state = self
            .state
            .as_mut()
            .expect("frontend compilation requires an initialized command string-table root");
        let warning_store = &mut self.successful_warnings;
        (
            &mut state.string_table,
            SuccessfulWarningSink {
                store: warning_store,
            },
        )
    }

    /// Commit one successfully completed frontend boundary to the warning owner.
    pub(crate) fn commit_successful_warnings(
        &mut self,
        boundary: SuccessfulWarningBoundary,
        source_database: Option<Arc<SourceDatabase>>,
    ) {
        self.successful_warnings
            .commit_boundary(boundary, source_database);
    }
    /// Retain named source owners referenced by successful warning handles until report freeze.
    pub(crate) fn retain_successful_warning_identity_domain(
        &mut self,
        identity: &StablePackageIdentity,
        source_database: &Arc<SourceDatabase>,
    ) {
        self.successful_warnings
            .retain_identity_domain(identity.clone(), Arc::clone(source_database));
    }

    pub(crate) fn discard_successful_warnings(&mut self) {
        self.successful_warnings.batches.clear();
        self.successful_warnings.identity_domains.clear();
        self.successful_warnings.active_source_package = None;
    }

    /// Freeze committed warnings separately while leaving the mutable identity table available
    /// to later successful stages. A capacity error promotes the frozen set to `Diagnosed`,
    /// discarding any unrelated successful warnings.
    pub(crate) fn finish_successful_warnings<T>(
        &mut self,
        value: T,
    ) -> Result<CompilationOutcome<T>, InfrastructureErrors> {
        let mut warnings = std::mem::take(&mut self.successful_warnings.batches);
        let identity_domains = std::mem::take(&mut self.successful_warnings.identity_domains);
        warnings.sort_unstable_by_key(|warning| {
            let command_tail = matches!(warning.position, SuccessfulWarningPosition::CommandTail);
            let boundary = if command_tail {
                0
            } else {
                match warning.boundary {
                    SuccessfulWarningBoundary::Project => 0,
                    SuccessfulWarningBoundary::SourcePackage(index) => index + 1,
                }
            };
            let (lane, index) = match warning.position {
                #[cfg(test)]
                SuccessfulWarningPosition::Configuration => (0, 0),
                SuccessfulWarningPosition::BaseModule(module_id) => (1, module_id),
                SuccessfulWarningPosition::GeneratedSidecar(publication_index) => {
                    (2, publication_index)
                }
                SuccessfulWarningPosition::CommandTail => (3, 0),
            };
            (command_tail, boundary, lane, index, warning.sequence)
        });
        if !warnings
            .iter()
            .any(|warning| warning.committed && warning.batch.diagnostic_count() != 0)
        {
            return Ok(CompilationOutcome::Success {
                value,
                warnings: None,
            });
        }

        let mut warning_accumulator = BoundaryAccumulator::with_root_string_table(
            self.string_table().clone_preserving_inherited_prefix(),
        );
        #[cfg(test)]
        {
            warning_accumulator.test_limits = self.test_limits;
        }
        for warning in warnings {
            if warning.committed {
                warning_accumulator.append_prepared_inner(warning.batch, warning.source);
            }
        }
        for (identity, source_database) in identity_domains {
            warning_accumulator.append_identity_source(identity, source_database);
        }

        let reports = warning_accumulator.finish(InstallMode::Frozen)?;
        if reports.counts().errors != 0 {
            Ok(CompilationOutcome::Diagnosed(reports.into_errors()))
        } else {
            Ok(CompilationOutcome::Success {
                value,
                warnings: Some(reports),
            })
        }
    }

    fn append_identity_source(
        &mut self,
        identity: StablePackageIdentity,
        source_database: Arc<SourceDatabase>,
    ) {
        let state = self
            .state
            .as_mut()
            .expect("successful warning identity sources require an initialized root table");
        let end = state.prepared.len();
        state.source_rows.push(SourceRow {
            diagnostic_range: end..end,
            source_database,
            domain: Some(identity),
        });
    }
    /// Append one batch prepared at its owning stage.
    pub(crate) fn append_prepared(
        &mut self,
        batch: PreparedDiagnosticBatch,
        source: Option<(Arc<SourceDatabase>, Option<StablePackageIdentity>)>,
    ) {
        self.append_prepared_inner(batch, source);
    }

    fn append_prepared_inner(
        &mut self,
        batch: PreparedDiagnosticBatch,
        source: Option<(Arc<SourceDatabase>, Option<StablePackageIdentity>)>,
    ) {
        let PreparedDiagnosticBatch {
            mut prepared,
            identity,
        } = batch;
        let (string_table, mut path_rows) = match *identity {
            PreparedBatchIdentity::Local {
                string_table,
                path_rows,
            } => (Some(string_table), path_rows),
            PreparedBatchIdentity::Root { path_rows } => (None, path_rows),
        };

        let Some(mut state) = self.state.take() else {
            let Some(string_table) = string_table else {
                crate::compiler_bug!(
                    CompilerStage::DiagnosticReporting,
                    "root batch appends require a root-seeded accumulator"
                );
            };
            let mut source_rows = Vec::new();
            if let Some((source_database, domain)) = source {
                source_rows.push(SourceRow {
                    diagnostic_range: 0..prepared.len(),
                    source_database,
                    domain,
                });
            }
            self.state = Some(BoundaryAccumulatorState {
                string_table,
                prepared,
                source_rows,
                path_rows,
                infrastructure_errors: None,
            });
            return;
        };

        let diagnostic_shift = state.prepared.len();
        let path_shift = state.path_rows.len();
        if let Some(string_table) = string_table {
            let remap = state.string_table.merge_from(&string_table);
            if !remap.is_identity() {
                for diagnostic in &mut prepared {
                    diagnostic.draft.remap_string_ids(&remap);
                    diagnostic.types.remap_string_ids(&remap);
                }
                for row in &mut path_rows {
                    Arc::make_mut(&mut row.path_table).remap_string_ids(&remap);
                }
            }
        }

        for row in &mut path_rows {
            row.diagnostic_range.start += diagnostic_shift;
            row.diagnostic_range.end += diagnostic_shift;
        }
        for diagnostic in &mut prepared {
            diagnostic.explicit_path = diagnostic.explicit_path.map(|row| row + path_shift);
        }
        let diagnostic_end = diagnostic_shift + prepared.len();
        state.prepared.extend(prepared);
        state.path_rows.extend(path_rows);
        if let Some((source_database, domain)) = source {
            state.source_rows.push(SourceRow {
                diagnostic_range: diagnostic_shift..diagnostic_end,
                source_database,
                domain,
            });
        }
        self.state = Some(state);
    }

    /// Append infrastructure causes without fabricating a diagnostic.
    pub(crate) fn append_infrastructure(&mut self, errors: Box<InfrastructureErrors>) {
        let state = self.state.get_or_insert_with(|| BoundaryAccumulatorState {
            string_table: StringTable::empty_unallocated(),
            prepared: Vec::new(),
            source_rows: Vec::new(),
            path_rows: Vec::new(),
            infrastructure_errors: None,
        });
        match state.infrastructure_errors.as_mut() {
            Some(existing) => existing.append(*errors),
            None => state.infrastructure_errors = Some(errors),
        }
    }

    /// The number of drafts currently retained in this run.
    pub(crate) fn diagnostic_count(&self) -> usize {
        self.state.as_ref().map_or(0, |state| state.prepared.len())
            + self
                .successful_warnings
                .batches
                .iter()
                .map(|warning| warning.batch.diagnostic_count())
                .sum::<usize>()
    }
    #[cfg(feature = "data_layout_memory_probe")]
    pub(crate) fn prepared_label_count(&self) -> usize {
        let retained = self.state.as_ref().map_or(0, |state| {
            state
                .prepared
                .iter()
                .map(PreparedDiagnostic::label_count)
                .sum::<usize>()
        });
        retained
            + self
                .successful_warnings
                .batches
                .iter()
                .map(|warning| warning.batch.label_count())
                .sum::<usize>()
    }

    /// The run's mutable root table, used by backend and output producers before the terminal freeze.
    pub(crate) fn string_table_mut(&mut self) -> &mut StringTable {
        &mut self
            .state
            .as_mut()
            .expect("the build warning accumulator must own its string-table root")
            .string_table
    }
    /// The run's immutable root table, used by lookup-only backend consumers.
    pub(crate) fn string_table(&self) -> &StringTable {
        &self
            .state
            .as_ref()
            .expect("the build warning accumulator must own its string-table root")
            .string_table
    }
    /// Return the root table after a successful Boracle stop, dropping staged warnings as HEAD did.
    #[cfg(feature = "boracle")]
    pub(crate) fn into_root_string_table(self) -> StringTable {
        let Self { state, .. } = self;
        state
            .expect("the Boracle frontend must retain its command string-table root")
            .string_table
    }

    /// Attach the build's project fallback source owner after diagnostic producers have appended.
    pub(crate) fn set_project_source_database(&mut self, source_database: Arc<SourceDatabase>) {
        let Some(state) = self.state.as_mut() else {
            return;
        };
        let diagnostic_end = state.prepared.len();
        if diagnostic_end != 0 {
            state.source_rows.push(SourceRow {
                diagnostic_range: 0..diagnostic_end,
                source_database,
                domain: None,
            });
        }
    }

    pub(crate) fn finish(
        self,
        install: InstallMode,
    ) -> Result<DiagnosticReportSet, InfrastructureErrors> {
        #[cfg(test)]
        let limits = self
            .test_limits
            .unwrap_or_else(DiagnosticLimits::production);
        #[cfg(not(test))]
        let limits = DiagnosticLimits::production();

        let state = self.state.unwrap_or_else(|| BoundaryAccumulatorState {
            string_table: StringTable::new(),
            prepared: Vec::new(),
            source_rows: Vec::new(),
            path_rows: Vec::new(),
            infrastructure_errors: None,
        });
        let BoundaryAccumulatorState {
            string_table,
            mut prepared,
            source_rows,
            path_rows,
            infrastructure_errors,
        } = state;
        if let Some(errors) = infrastructure_errors {
            return Err(*errors);
        }

        // Source/path rows may be appended after a batch was prepared (build fallback owners and
        // late output diagnostics). Resolve them only after the complete push-ordered run exists.
        for (index, diagnostic) in prepared.iter_mut().enumerate() {
            let owner = source_rows
                .iter()
                .position(|row| row.diagnostic_range.contains(&index));
            let identityless_owner = source_rows
                .iter()
                .position(|row| row.domain.is_none() && row.diagnostic_range.contains(&index));
            // A foreign primary resolves through its explicit handle; keep the report's
            // unqualified SourceIds in the identityless project context when one is available.
            diagnostic.owner = if diagnostic.draft.primary_frozen_identity_handle().is_some() {
                identityless_owner.or(owner)
            } else {
                owner
            };
            diagnostic.identityless_owner = identityless_owner;
            diagnostic.explicit_path = path_rows
                .iter()
                .position(|row| row.diagnostic_range.contains(&index));
        }

        // Only a frozen run retains domains named by uninstalled foreign-site owners.
        let mut required = Vec::new();
        if install == InstallMode::Frozen {
            for diagnostic in &prepared {
                diagnostic.draft.visit_identity_handles(|handle| {
                    if handle.get().is_none()
                        && let Some(identity) = handle.domain()
                        && !required.contains(identity)
                    {
                        required.push(identity.clone());
                    }
                });
            }
        }

        // A source row is kept when it covers a diagnostic, or when a frozen handle needs its identity.
        let mut databases: Vec<Arc<SourceDatabase>> = Vec::new();
        let mut kept: Vec<Option<KeptSourceRow>> = Vec::with_capacity(source_rows.len());
        for row in source_rows {
            let needed = !row.diagnostic_range.is_empty()
                || (install == InstallMode::Frozen
                    && row
                        .domain
                        .as_ref()
                        .is_some_and(|domain| required.contains(domain)));
            if needed {
                let slot = intern_database(&mut databases, row.source_database);
                kept.push(Some(KeptSourceRow {
                    domain: row.domain,
                    slot,
                }));
            } else {
                kept.push(None);
            }
        }

        // Every owner has been released, so each database must be uniquely owned here.
        let mut owned: Vec<Option<SourceDatabase>> = databases
            .into_iter()
            .map(|database| {
                Some(Arc::try_unwrap(database).unwrap_or_else(|_| {
                    crate::compiler_bug!(
                        CompilerStage::DiagnosticReporting,
                        "source database was unexpectedly shared at the frozen render handoff"
                    )
                }))
            })
            .collect();

        // Each owned diagnostic with an explicit path table merges that table into its owner's builder, once per table.
        let mut merges: Vec<PathMerge> = Vec::new();
        for diagnostic in &mut prepared {
            let (Some(owner), Some(explicit)) = (diagnostic.owner, diagnostic.explicit_path) else {
                continue;
            };
            let slot = kept[owner]
                .as_ref()
                .expect("a covering source row is always kept")
                .slot;
            let table = &path_rows[explicit].path_table;
            let known = merges.iter().position(|merge| {
                merge.slot == slot && Arc::ptr_eq(&path_rows[merge.first_path].path_table, table)
            });
            let merge = match known {
                Some(merge) => merge,
                None => {
                    let outcome = owned[slot]
                        .as_mut()
                        .expect("a database is merged before it registers")
                        .path_builder_mut()
                        .merge_snapshot(table);
                    merges.push(PathMerge {
                        slot,
                        first_path: explicit,
                        outcome,
                    });
                    merges.len() - 1
                }
            };
            diagnostic.merge = Some(merge);
        }

        // Domains: one per database and identity pair. A database already registered under another identity shares its
        // first domain's context.
        let mut pending = PendingDiagnostics::default();
        let mut row_domain: Vec<Option<u32>> = vec![None; kept.len()];
        let mut database_domain: Vec<Option<u32>> = vec![None; owned.len()];
        let mut registered: Vec<(usize, Option<StablePackageIdentity>, u32)> = Vec::new();
        for (index, row) in kept.iter().enumerate() {
            let Some(row) = row else {
                continue;
            };
            let known = registered
                .iter()
                .find(|(slot, identity, _)| *slot == row.slot && *identity == row.domain)
                .map(|&(_, _, domain)| domain);
            let domain = match known {
                Some(domain) => domain,
                None => {
                    let domain = match database_domain[row.slot] {
                        Some(first) => {
                            pending.add_domain(row.domain.clone(), DomainSource::Shares(first))
                        }
                        None => {
                            let database =
                                owned[row.slot].take().expect("a database registers once");
                            let domain = pending
                                .add_domain(row.domain.clone(), DomainSource::Sources(database));
                            database_domain[row.slot] = Some(domain);
                            domain
                        }
                    };
                    registered.push((row.slot, row.domain.clone(), domain));
                    domain
                }
            };
            row_domain[index] = Some(domain);
        }
        // Unowned diagnostics share one domain per explicit path table. Without one, they share an empty table.
        let mut shared_root: Option<u32> = None;
        let mut unowned_tables: Vec<(usize, u32)> = Vec::new();
        for diagnostic in prepared {
            let PreparedDiagnostic {
                mut draft,
                mut types,
                owner,
                explicit_path,
                identityless_owner,
                merge,
            } = diagnostic;

            let domain = match (owner, explicit_path) {
                (Some(row), _) => {
                    row_domain[row].expect("a covering source row is always registered")
                }
                (None, Some(explicit)) => {
                    let table = &path_rows[explicit].path_table;
                    let known = unowned_tables
                        .iter()
                        .find(|(first, _)| Arc::ptr_eq(&path_rows[*first].path_table, table))
                        .map(|&(_, domain)| domain);
                    match known {
                        Some(domain) => domain,
                        None => {
                            let domain =
                                pending.add_domain(None, DomainSource::Unowned(Arc::clone(table)));
                            unowned_tables.push((explicit, domain));
                            domain
                        }
                    }
                }
                (None, None) => *shared_root.get_or_insert_with(|| {
                    let empty = Arc::new(PathInternerBuilder::new().freeze());
                    pending.add_domain(None, DomainSource::Unowned(empty))
                }),
            };
            let none_row = match install {
                InstallMode::Frozen => identityless_owner,
                InstallMode::Unfrozen => owner,
            };
            let none_domain = none_row.and_then(|row| row_domain[row]);

            if let Some(index) = merge {
                match &merges[index].outcome {
                    Ok(remap) => {
                        if !remap.is_identity() {
                            draft.remap_path_ids(remap);
                            types.remap_path_ids(remap);
                        }
                    }
                    Err(PathInternError::TableFull) => {
                        draft = super::families::token_lexical::source_span_capacity(
                            0,
                            u32::MAX,
                            SourceSpanCapacityResource::LogicalPath,
                            draft.capacity_span(),
                        );
                        types = TypeDisplaySnapshot::default();
                    }
                    Err(error) => crate::compiler_bug!(
                        CompilerStage::DiagnosticReporting,
                        "a path snapshot merge failed with {error:?}"
                    ),
                }
            }
            push_rebased(&mut pending, domain, none_domain, draft, types);
        }

        let reports = pending.freeze(string_table.freeze_flattening(), limits, install);
        Ok(reports)
    }
}
/// Returns the slot of `database`, adding it when no earlier row named the same allocation.
fn intern_database(
    databases: &mut Vec<Arc<SourceDatabase>>,
    database: Arc<SourceDatabase>,
) -> usize {
    match databases
        .iter()
        .position(|known| Arc::ptr_eq(known, &database))
    {
        Some(slot) => slot,
        None => {
            databases.push(database);
            databases.len() - 1
        }
    }
}

/// Appends a diagnostic's type words to its domain, then rebases its draft onto them. A capacity failure at either
/// step replaces the draft with its type-display capacity record, which names no type words.
fn push_rebased(
    pending: &mut PendingDiagnostics,
    domain: u32,
    none_domain: Option<u32>,
    mut draft: DiagnosticDraft,
    types: TypeDisplaySnapshot,
) {
    let rebased = pending
        .append_types(domain, types)
        .and_then(|base| draft.rebase_type_displays(base));
    match rebased {
        Ok(()) => pending.push(domain, none_domain, draft),
        Err(_) => {
            let capacity = draft.faulted_by(DiagnosticCapacityResource::TypeDisplays);
            pending.push(domain, none_domain, capacity);
        }
    }
}
