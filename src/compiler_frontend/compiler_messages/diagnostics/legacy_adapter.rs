//! Converts legacy diagnostics into the compact family drafts of the frozen boundary.
//!
//! WHAT: `into_boundary_failure` converts a whole legacy vessel into one frozen report set. Each diagnostic maps onto
//! its family draft, severity, labels and foreign sites, and its types are captured as display ids. Its path ids
//! follow the path table it was rendered against, merged into the owning database's builder. A draft that overflows a
//! capacity is stored as its capacity record, which renders as a compiler limit.
//! WHY: the boundary replaces the legacy vessel without changing rendered output. `compact_for_render` is a test-only
//! per-diagnostic store shim.

#[cfg(test)]
use super::compaction::CompactOutcome;
use super::draft::DiagnosticDraft;
use super::families;
use super::foreign::{ForeignSiteDraft, ForeignSlot};
use super::freeze::{DomainSource, InstallMode, PendingDiagnostics};
use super::resource::DiagnosticLimits;
#[cfg(test)]
use super::store::{DiagnosticId, DiagnosticStore};
#[cfg(test)]
use super::view::DiagnosticView;
use crate::compiler_frontend::compiler_messages::compiler_errors::{
    BoundaryFailure, CompilerMessages, InfrastructureErrors, RenderPathContext, RenderSourceContext,
};
use crate::compiler_frontend::compiler_messages::diagnostic_types::{
    type_display_snapshot::TypeDisplaySnapshot,
    type_display_snapshot_builder::TypeDisplaySnapshotBuilder,
};
#[cfg(test)]
use crate::compiler_frontend::compiler_messages::render::DiagnosticRenderContext;
use crate::compiler_frontend::compiler_messages::{
    CapacityResource, CompilerDiagnostic, DiagnosticCapacityResource, SourceSpanCapacityResource,
};
use crate::compiler_frontend::semantic_identity::StablePackageIdentity;
use crate::compiler_frontend::source::{FrozenIdentityHandle, SourceDatabase};
use crate::compiler_frontend::symbols::path_interner::{
    PathIdRemap, PathInternError, PathInternerBuilder,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use std::ops::Range;
use std::sync::Arc;

/// One compacted diagnostic, with the store and the type snapshot that its record reads from.
#[cfg(test)]
pub(crate) struct CompactRender {
    store: DiagnosticStore,
    id: DiagnosticId,
    types: TypeDisplaySnapshot,
}

#[cfg(test)]
impl CompactRender {
    /// The typed view of the stored record.
    pub(crate) fn view(&self) -> DiagnosticView<'_> {
        DiagnosticView::new(&self.store, self.id, &self.types)
    }
}

/// Compacts `diagnostic` when its kind and payload match a family adapter arm, for render tests.
///
/// Returns `None` when the adapter declines the diagnostic.
#[cfg(test)]
pub(crate) fn compact_for_render(
    diagnostic: &CompilerDiagnostic,
    context: DiagnosticRenderContext<'_>,
) -> Option<CompactRender> {
    let mut draft = finish(families::from_legacy(diagnostic)?, diagnostic);

    // Spellings resolve only when the live environment and the path table both exist, which is the condition
    // `diagnostic_type_name` applies. Any other type becomes an unresolved root that prints its raw id.
    let environment = context
        .type_environment
        .filter(|_| context.path_table().is_some());
    let mut builder = TypeDisplaySnapshotBuilder::new(environment);
    draft
        .capture_types(&mut |type_id| builder.display_top_level(type_id))
        .expect("one diagnostic's types always fit the u32 display-id space");
    let types = builder.finish();

    let mut store = DiagnosticStore::new();
    // A refused draft latches the fresh store, whose only record is then its capacity record at index 0.
    let id = match store.compact(draft) {
        CompactOutcome::Stored(id) => id,
        CompactOutcome::Rejected(_) => DiagnosticId::from_index(0),
    };
    Some(CompactRender { store, id, types })
}

/// Compacts `diagnostic` for a render test. A diagnostic without a compact family cannot reach a render entry, so
/// this panics where the boundary converter does.
#[cfg(test)]
pub(crate) fn compact_render_for_test(
    diagnostic: &CompilerDiagnostic,
    context: DiagnosticRenderContext<'_>,
) -> CompactRender {
    compact_for_render(diagnostic, context).unwrap_or_else(|| {
        panic!(
            "legacy diagnostic has no compact family; the boundary converter cannot represent it"
        )
    })
}

/// Adds the legacy severity, labels and foreign sites to a family draft.
///
/// A label without a message is kept as `NONE`. Compaction checks that the family declares each label's role. A
/// faulted draft is kept: it already is the capacity record.
fn finish(mut draft: DiagnosticDraft, diagnostic: &CompilerDiagnostic) -> DiagnosticDraft {
    draft = draft.with_severity(diagnostic.severity);
    for label in &diagnostic.labels {
        draft = draft.with_label(label.span, label.message.clone());
    }

    for (index, label) in diagnostic.labels.iter().enumerate() {
        if let Some(handle) = &label.frozen_identity_handle {
            draft = draft.with_foreign(ForeignSiteDraft {
                slot: ForeignSlot::Label(index),
                context: handle.clone(),
            });
        }
    }
    if let Some(handle) = &diagnostic.primary_frozen_identity_handle {
        draft = draft.with_foreign(ForeignSiteDraft {
            slot: ForeignSlot::Primary,
            context: handle.clone(),
        });
    }

    draft
}

/// The draft and rows of one legacy diagnostic, read before its vessel is consumed.
struct PreparedDiagnostic {
    draft: DiagnosticDraft,
    types: TypeDisplaySnapshot,
    /// The first source row that covers the diagnostic. `None` leaves the diagnostic unowned.
    owner: Option<usize>,
    /// The first path row that covers the diagnostic. Its table replaces the owner's table for this diagnostic.
    explicit_path: Option<usize>,
    /// The first identity-less source row that covers the diagnostic, selected only in Frozen mode.
    identityless_owner: Option<usize>,
    /// The merge whose remap the diagnostic's path ids follow.
    merge: Option<usize>,
}

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

/// Accumulates legacy vessels before one canonical freeze.
///
/// Empty construction has no allocations; the first append adopts its string table as the root.
pub(crate) struct BoundaryAccumulator {
    state: Option<BoundaryAccumulatorState>,
}

struct BoundaryAccumulatorState {
    string_table: Box<StringTable>,
    prepared: Vec<PreparedDiagnostic>,
    source_rows: Vec<RenderSourceContext>,
    path_rows: Vec<RenderPathContext>,
    required_handles: Vec<FrozenIdentityHandle>,
    infrastructure_errors: Option<Box<InfrastructureErrors>>,
}

/// Converts one legacy vessel through the run-scoped accumulator.
///
/// The vessel's mode remains authoritative for this transitional one-vessel boundary.
pub(crate) fn into_boundary_failure(messages: CompilerMessages) -> BoundaryFailure {
    let install = messages.install_mode;
    let mut accumulator = BoundaryAccumulator::new();
    accumulator.append(messages);
    accumulator.finish(install)
}

impl BoundaryAccumulator {
    /// Creates an empty accumulator without allocating.
    pub(crate) fn new() -> Self {
        Self { state: None }
    }

    /// Captures one vessel against its own legacy contexts, then joins its rows to this run.
    pub(crate) fn append(&mut self, messages: CompilerMessages) {
        let mut required_handles = Vec::new();
        let mut prepared: Vec<PreparedDiagnostic> = (0..messages.diagnostics.len())
            .map(|index| prepare(&messages, index, &mut required_handles))
            .collect();
        let CompilerMessages {
            infrastructure_errors,
            string_table,
            render_source_contexts,
            render_path_contexts,
            ..
        } = messages;
        let mut path_rows = render_path_contexts
            .map(|rows| rows.into_vec())
            .unwrap_or_default();

        let Some(mut state) = self.state.take() else {
            self.state = Some(BoundaryAccumulatorState {
                string_table,
                prepared,
                source_rows: render_source_contexts,
                path_rows,
                required_handles,
                infrastructure_errors,
            });
            return;
        };

        let diagnostic_shift = state.prepared.len();
        let source_shift = state.source_rows.len();
        let path_shift = state.path_rows.len();
        let remap = state.string_table.merge_from(string_table.as_ref());
        if !remap.is_identity() {
            for diagnostic in &mut prepared {
                diagnostic.draft.remap_string_ids(&remap);
                diagnostic.types.remap_string_ids(&remap);
            }
            for row in &mut path_rows {
                Arc::make_mut(&mut row.path_table).remap_string_ids(&remap);
            }
        }

        for diagnostic in &mut prepared {
            diagnostic.owner = diagnostic.owner.map(|row| row + source_shift);
            diagnostic.identityless_owner =
                diagnostic.identityless_owner.map(|row| row + source_shift);
            diagnostic.explicit_path = diagnostic.explicit_path.map(|row| row + path_shift);
        }
        state.prepared.extend(prepared);
        for mut row in render_source_contexts {
            row.diagnostic_range.start += diagnostic_shift;
            row.diagnostic_range.end += diagnostic_shift;
            state.source_rows.push(row);
        }
        for mut row in path_rows {
            row.diagnostic_range.start += diagnostic_shift;
            row.diagnostic_range.end += diagnostic_shift;
            state.path_rows.push(row);
        }
        state.required_handles.extend(required_handles);
        if let Some(incoming) = infrastructure_errors {
            match state.infrastructure_errors.as_mut() {
                Some(existing) => existing.append(*incoming),
                None => state.infrastructure_errors = Some(incoming),
            }
        }
        self.state = Some(state);
    }
    /// Append warnings that already reference this run's root string table.
    ///
    /// Unlike a legacy vessel, this batch has no independent table to merge or remap.
    pub(crate) fn append_diagnostics(&mut self, diagnostics: Vec<CompilerDiagnostic>) {
        if diagnostics.is_empty() {
            return;
        }
        let messages = CompilerMessages::from_diagnostics(diagnostics, StringTable::new());
        let mut required_handles = Vec::new();
        let prepared = (0..messages.diagnostics.len())
            .map(|index| prepare(&messages, index, &mut required_handles))
            .collect::<Vec<_>>();
        let state = self
            .state
            .as_mut()
            .expect("diagnostic batches require an initialized string-table root");
        state.prepared.extend(prepared);
        state.required_handles.extend(required_handles);
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
            infrastructure_errors.set_source_database(Arc::clone(&source_database));
        }
        let diagnostic_end = state.prepared.len();
        if diagnostic_end != 0 {
            state.source_rows.push(RenderSourceContext {
                diagnostic_range: 0..diagnostic_end,
                source_database,
                domain: None,
            });
        }
    }

    /// Keep domain-less project source rows paired through output diagnostics appended later.
    pub(crate) fn extend_project_source_context_to_diagnostics(&mut self) {
        let Some(state) = self.state.as_mut() else {
            return;
        };
        let diagnostic_end = state.prepared.len();
        for row in &mut state.source_rows {
            if row.domain.is_none() && !row.diagnostic_range.is_empty() {
                row.diagnostic_range.end = row.diagnostic_range.end.max(diagnostic_end);
            }
        }
    }

    /// Resolves the run-wide install policy, merges retained owners, and freezes once.
    pub(crate) fn finish(self, install: InstallMode) -> BoundaryFailure {
        let state = self.state.unwrap_or_else(|| BoundaryAccumulatorState {
            string_table: Box::new(StringTable::new()),
            prepared: Vec::new(),
            source_rows: Vec::new(),
            path_rows: Vec::new(),
            required_handles: Vec::new(),
            infrastructure_errors: None,
        });
        let BoundaryAccumulatorState {
            string_table,
            mut prepared,
            source_rows,
            path_rows,
            required_handles,
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

        // Only a frozen run installs identity-named handles and retains their donor rows.
        let mut required = Vec::new();
        if install == InstallMode::Frozen {
            for handle in required_handles
                .iter()
                .filter(|handle| handle.get().is_none())
            {
                if let Some(identity) = handle.domain()
                    && !required.contains(identity)
                {
                    required.push(identity.clone());
                }
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
                        draft = table_capacity_draft();
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

/// Pass A for one diagnostic: build its family draft and capture type words against its own vessel context.
fn prepare(
    messages: &CompilerMessages,
    index: usize,
    required_handles: &mut Vec<FrozenIdentityHandle>,
) -> PreparedDiagnostic {
    let diagnostic = &messages.diagnostics[index];

    let owner = messages
        .render_source_contexts
        .iter()
        .position(|row| row.diagnostic_range.contains(&index));
    let identityless_owner = messages
        .render_source_contexts
        .iter()
        .position(|row| row.domain.is_none() && row.diagnostic_range.contains(&index));
    let explicit_path = messages.render_path_contexts.as_deref().and_then(|rows| {
        rows.iter()
            .position(|row| row.diagnostic_range.contains(&index))
    });

    let handles = diagnostic.primary_frozen_identity_handle.iter().chain(
        diagnostic
            .labels
            .iter()
            .filter_map(|label| label.frozen_identity_handle.as_ref()),
    );
    required_handles.extend(handles.cloned());

    let environment = messages
        .type_environment_for_diagnostic(index)
        .filter(|_| messages.path_table_for_diagnostic(index).is_some());
    let mut builder = TypeDisplaySnapshotBuilder::new(environment);
    let family = families::from_legacy(diagnostic).unwrap_or_else(|| {
        panic!(
            "legacy diagnostic has no compact family; the boundary converter cannot represent it"
        )
    });
    let mut draft = finish(family, diagnostic);
    let captured = draft.capture_types(&mut |type_id| builder.display_top_level(type_id));
    let types = match captured {
        Ok(()) => builder.finish(),
        Err(_) => {
            draft = DiagnosticDraft::capacity_fault(
                draft.capacity_span(),
                CapacityResource::Store(DiagnosticCapacityResource::TypeDisplays),
            );
            TypeDisplaySnapshot::default()
        }
    };

    PreparedDiagnostic {
        draft,
        types,
        owner,
        explicit_path,
        identityless_owner,
        merge: None,
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
            let capacity = DiagnosticDraft::capacity_fault(
                draft.capacity_span(),
                CapacityResource::Store(DiagnosticCapacityResource::TypeDisplays),
            );
            pending.push(domain, none_domain, capacity);
        }
    }
}

/// The record that replaces a diagnostic whose owner's path table cannot absorb the table that it names.
fn table_capacity_draft() -> DiagnosticDraft {
    let diagnostic =
        CompilerDiagnostic::source_table_capacity(SourceSpanCapacityResource::LogicalPath);
    let family = families::from_legacy(&diagnostic)
        .expect("a logical-path capacity record always has a compact family");
    finish(family, &diagnostic)
}
