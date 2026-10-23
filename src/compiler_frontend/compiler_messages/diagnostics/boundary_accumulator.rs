//! Run-scoped merge, ownership capture, and canonical freeze for prepared diagnostics.
//!
//! WHAT: joins batches and infrastructure causes, resolves identity/path owners, then freezes once.
//! WHY: source and path ownership is resolved after the complete diagnostic order is known.

use super::draft::DiagnosticDraft;
use super::freeze::{DomainSource, InstallMode, PendingDiagnostics};
use super::prepared::{PreparedDiagnostic, prepare_draft};
use super::resource::DiagnosticLimits;
use crate::compiler_frontend::compiler_messages::compiler_errors::{
    BoundaryFailure, InfrastructureErrors,
};
use crate::compiler_frontend::compiler_messages::diagnostic_types::type_display_snapshot::TypeDisplaySnapshot;
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

/// Accumulates prepared diagnostics and infrastructure causes before one canonical freeze.
///
/// Empty construction has no allocations; the first prepared append adopts its batch table.
pub(crate) struct BoundaryAccumulator {
    state: Option<BoundaryAccumulatorState>,
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
        Self { state: None }
    }

    /// Creates an accumulator seeded with the run's root string table.
    ///
    /// Causes that carry a source database must enter through an accumulator built this way,
    /// because the frozen identity resolves strings through the table seeded here.
    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "P4.5 S7b root-seeded boundary callers")
    )]
    pub(crate) fn with_root_string_table(root: &StringTable) -> Self {
        Self {
            state: Some(BoundaryAccumulatorState {
                string_table: root.clone(),
                prepared: Vec::new(),
                source_rows: Vec::new(),
                path_rows: Vec::new(),
                infrastructure_errors: None,
            }),
        }
    }

    /// Append one batch prepared at its owning stage.
    pub(crate) fn append_prepared(
        &mut self,
        batch: PreparedDiagnosticBatch,
        source: Option<(Arc<SourceDatabase>, Option<StablePackageIdentity>)>,
    ) {
        let PreparedDiagnosticBatch {
            mut prepared,
            identity,
        } = batch;
        let PreparedBatchIdentity {
            string_table,
            mut path_rows,
        } = *identity;

        let Some(mut state) = self.state.take() else {
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

    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "P4.5 S7b infrastructure boundary callers")
    )]
    /// Append infrastructure causes without fabricating a diagnostic.
    pub(crate) fn append_infrastructure(
        &mut self,
        mut errors: Box<InfrastructureErrors>,
        source: Option<Arc<SourceDatabase>>,
    ) {
        assert!(
            source.is_none() || self.state.is_some(),
            "source-carrying infrastructure requires a root-seeded accumulator",
        );
        if let Some(source_database) = source {
            errors.pair_unowned_causes(source_database);
        }
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

    /// Append warnings that already reference this run's root string table.
    ///
    /// These diagnostics already use the run's root string table, so no table merge is needed.
    pub(crate) fn append_diagnostics(&mut self, diagnostics: Vec<DiagnosticDraft>) {
        if diagnostics.is_empty() {
            return;
        }
        let prepared = diagnostics
            .into_iter()
            .map(|draft| prepare_draft(draft, None))
            .collect::<Vec<_>>();
        let state = self
            .state
            .as_mut()
            .expect("diagnostic batches require an initialized string-table root");
        state.prepared.extend(prepared);
    }

    /// The number of drafts currently retained in this run.
    pub(crate) fn diagnostic_count(&self) -> usize {
        self.state.as_ref().map_or(0, |state| state.prepared.len())
    }

    /// The family titles in diagnostic push order, before any capacity replacement at freeze.
    pub(crate) fn family_titles(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.state.iter().flat_map(|state| {
            state
                .prepared
                .iter()
                .map(|diagnostic| diagnostic.draft.core.family.descriptor().title)
        })
    }

    /// The stable family codes in diagnostic push order.
    pub(crate) fn family_codes(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.state.iter().flat_map(|state| {
            state
                .prepared
                .iter()
                .map(|diagnostic| diagnostic.draft.core.family.descriptor().code)
        })
    }

    /// The run's mutable root table, used by backend and output producers before the terminal freeze.
    pub(crate) fn string_table_mut(&mut self) -> &mut StringTable {
        &mut self
            .state
            .as_mut()
            .expect("the build warning accumulator must own its string-table root")
            .string_table
    }

    /// Attach the build's project fallback source owner after all diagnostic producers have appended.
    pub(crate) fn set_project_source_database(&mut self, source_database: Arc<SourceDatabase>) {
        let Some(state) = self.state.as_mut() else {
            return;
        };
        if let Some(infrastructure_errors) = state.infrastructure_errors.as_mut() {
            infrastructure_errors.pair_unowned_causes(Arc::clone(&source_database));
        }
        let diagnostic_end = state.prepared.len();
        if diagnostic_end != 0 {
            state.source_rows.push(SourceRow {
                diagnostic_range: 0..diagnostic_end,
                source_database,
                domain: None,
            });
        }
    }

    /// Resolves the run-wide install policy, merges retained owners, and freezes once.
    pub(crate) fn finish(self, install: InstallMode) -> BoundaryFailure {
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
            mut infrastructure_errors,
        } = state;

        // Source/path rows may be appended after a batch was prepared (build fallback owners and
        // late output diagnostics). Resolve them only after the complete push-ordered run exists.
        for (index, diagnostic) in prepared.iter_mut().enumerate() {
            diagnostic.owner = source_rows
                .iter()
                .position(|row| row.diagnostic_range.contains(&index));
            diagnostic.identityless_owner = source_rows
                .iter()
                .position(|row| row.domain.is_none() && row.diagnostic_range.contains(&index));
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

        // Infrastructure cause rows name databases too. Their databases join the same list.
        let mut cause_rows: Vec<(Range<usize>, usize)> = Vec::new();
        if let Some(infrastructure) = infrastructure_errors.as_deref_mut() {
            for (range, database) in infrastructure.take_source_rows() {
                let slot = intern_database(&mut databases, database);
                cause_rows.push((range, slot));
            }
        }

        // Every owner has been released, so each database must be uniquely owned here.
        let mut owned: Vec<Option<SourceDatabase>> = databases
            .into_iter()
            .map(|database| {
                Some(Arc::try_unwrap(database).unwrap_or_else(|_| {
                    panic!("source database was unexpectedly shared at the frozen render handoff")
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

        let mut cause_domains: Vec<(Range<usize>, u32)> = Vec::with_capacity(cause_rows.len());
        for (range, slot) in cause_rows {
            let domain = match database_domain[slot] {
                Some(domain) => domain,
                None => {
                    let database = owned[slot].take().expect("a database registers once");
                    let domain = pending.add_domain(None, DomainSource::Sources(database));
                    database_domain[slot] = Some(domain);
                    domain
                }
            };
            cause_domains.push((range, domain));
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
                            None,
                        );
                        types = TypeDisplaySnapshot::default();
                    }
                    Err(error) => panic!("a path snapshot merge failed with {error:?}"),
                }
            }
            push_rebased(&mut pending, domain, none_domain, draft, types);
        }

        let reports = pending.freeze(
            string_table.freeze_flattening(),
            DiagnosticLimits::production(),
            install,
        );

        if let Some(infrastructure) = infrastructure_errors.as_deref_mut() {
            let rows = cause_domains
                .into_iter()
                .map(|(range, domain)| (range, reports.report(domain).shared_context()))
                .collect();
            infrastructure.set_frozen_source_rows(rows);
        }

        BoundaryFailure {
            reports,
            infrastructure: infrastructure_errors,
        }
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
