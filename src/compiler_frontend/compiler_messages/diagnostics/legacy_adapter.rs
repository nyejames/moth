//! Compacts a diagnostic when its kind and payload match a family adapter arm.
//!
//! WHAT: the adapter maps a matching diagnostic onto its family draft, severity, labels and foreign sites, and its
//! types are captured as display ids. The draft is stored and read back through a typed view. When the adapter
//! declines a diagnostic, the caller keeps the legacy payload path until P4.6. No label is declined: a label without
//! a message is kept as `NONE`, and compaction checks that the family declares each label's role. A draft that faults
//! while it is filled, or that the store refuses, is stored as its capacity record, which renders as a compiler limit.
//! WHY: matching diagnostics render from compact records; declined diagnostics keep their legacy output until P4.6.
//! Each call builds a store for its one diagnostic; a shared store replaces it once the boundary owns one.
//!
//! `into_boundary_failure` converts a whole legacy vessel into one frozen report set. It merges each diagnostic's
//! path table into its owner's builder, captures its types against that table, and freezes the batch once.

use super::compaction::CompactOutcome;
use super::draft::DiagnosticDraft;
use super::families;
use super::foreign::{ForeignSiteDraft, ForeignSlot};
use super::freeze::{DomainSource, InstallMode, PendingDiagnostics};
use super::resource::DiagnosticLimits;
use super::store::{DiagnosticId, DiagnosticStore};
use super::view::DiagnosticView;
use crate::compiler_frontend::compiler_messages::compiler_errors::{
    BoundaryFailure, CompilerMessages,
};
use crate::compiler_frontend::compiler_messages::diagnostic_types::{
    type_display_snapshot::TypeDisplaySnapshot,
    type_display_snapshot_builder::TypeDisplaySnapshotBuilder,
};
use crate::compiler_frontend::compiler_messages::render::DiagnosticRenderContext;
use crate::compiler_frontend::compiler_messages::{
    CapacityResource, CompilerDiagnostic, DiagnosticCapacityResource, SourceSpanCapacityResource,
};
use crate::compiler_frontend::semantic_identity::StablePackageIdentity;
use crate::compiler_frontend::source::SourceDatabase;
use crate::compiler_frontend::symbols::path_interner::{
    PathIdRemap, PathInternError, PathInternerBuilder,
};
use std::ops::Range;
use std::sync::Arc;

/// One compacted diagnostic, with the store and the type snapshot that its record reads from.
pub(crate) struct CompactRender {
    store: DiagnosticStore,
    id: DiagnosticId,
    types: TypeDisplaySnapshot,
}

impl CompactRender {
    /// The typed view of the stored record.
    pub(crate) fn view(&self) -> DiagnosticView<'_> {
        DiagnosticView::new(&self.store, self.id, &self.types)
    }
}

/// Compacts `diagnostic` when its kind and payload match a family adapter arm.
///
/// Returns `None` when the adapter declines the diagnostic. The caller then keeps the legacy payload path.
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
#[cfg_attr(not(test), allow(dead_code, reason = "P4.5 S3.2 converter"))]
struct PreparedDiagnostic {
    draft: DiagnosticDraft,
    types: TypeDisplaySnapshot,
    /// The first source row that covers the diagnostic. `None` leaves the diagnostic unowned.
    owner: Option<usize>,
    /// The first path row that covers the diagnostic. Its table replaces the owner's table for this diagnostic.
    explicit_path: Option<usize>,
    /// The source row whose domain resolves the diagnostic's identity-less foreign sites.
    none_row: Option<usize>,
    /// The merge whose remap the diagnostic's path ids follow.
    merge: Option<usize>,
}

/// A source row that the converter keeps: its identity, and the slot of its database.
#[cfg_attr(not(test), allow(dead_code, reason = "P4.5 S3.2 converter"))]
struct KeptSourceRow {
    domain: Option<StablePackageIdentity>,
    slot: usize,
}

/// One `merge_snapshot` call into a database's builder. Diagnostics that name the same table share it.
#[cfg_attr(not(test), allow(dead_code, reason = "P4.5 S3.2 converter"))]
struct PathMerge {
    slot: usize,
    /// The first path row that named the table. Later rows with the same allocation reuse this merge.
    first_path: usize,
    outcome: Result<PathIdRemap, PathInternError>,
}

/// Converts a legacy vessel into the frozen boundary failure.
///
/// WHAT: each legacy diagnostic becomes one compact draft in one report set. Its path ids follow the path table it was
/// rendered against, merged into the owning database's builder. Its type words are captured against its own snapshot.
/// Each database and identity pair becomes one identity domain.
/// WHY: the boundary replaces the legacy vessel without changing rendered output.
#[cfg_attr(not(test), allow(dead_code, reason = "P4.5 S3.2 converter"))]
pub(crate) fn into_boundary_failure(
    messages: CompilerMessages,
    install: InstallMode,
) -> BoundaryFailure {
    assert!(
        messages.render_frozen_contexts.is_empty(),
        "pre-frozen render contexts reached the boundary converter"
    );

    // Pass A reads the vessel. Each draft is captured against the legacy context of its own diagnostic.
    let mut required = Vec::new();
    let mut prepared: Vec<PreparedDiagnostic> = (0..messages.diagnostics.len())
        .map(|index| prepare(&messages, index, install, &mut required))
        .collect();

    // Pass B takes ownership. A source row is kept when it covers a diagnostic, or when a frozen handle must install
    // its identity. Each database is owned once, however many rows name it.
    let CompilerMessages {
        mut infrastructure_errors,
        string_table,
        render_source_contexts,
        render_path_contexts,
        ..
    } = messages;

    let mut databases: Vec<Arc<SourceDatabase>> = Vec::new();
    let mut kept: Vec<Option<KeptSourceRow>> = Vec::with_capacity(render_source_contexts.len());
    for row in render_source_contexts {
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
    let path_rows = render_path_contexts.map(Vec::from).unwrap_or_default();
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
                        let database = owned[row.slot].take().expect("a database registers once");
                        let domain =
                            pending.add_domain(row.domain.clone(), DomainSource::Sources(database));
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
            none_row,
            merge,
        } = diagnostic;

        let domain = match (owner, explicit_path) {
            (Some(row), _) => row_domain[row].expect("a covering source row is always registered"),
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

    let reports = pending.freeze(*string_table, DiagnosticLimits::production(), install);

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

/// Pass A for one diagnostic: its draft with the type words it names, and the rows that cover it. It only borrows the
/// vessel, so Pass B can take ownership afterwards.
#[cfg_attr(not(test), allow(dead_code, reason = "P4.5 S3.2 converter"))]
fn prepare(
    messages: &CompilerMessages,
    index: usize,
    install: InstallMode,
    required: &mut Vec<StablePackageIdentity>,
) -> PreparedDiagnostic {
    let diagnostic = &messages.diagnostics[index];

    let owner = messages
        .render_source_contexts
        .iter()
        .position(|row| row.diagnostic_range.contains(&index));
    let explicit_path = messages.render_path_contexts.as_deref().and_then(|rows| {
        rows.iter()
            .position(|row| row.diagnostic_range.contains(&index))
    });
    let none_row = match install {
        InstallMode::Frozen => messages
            .render_source_contexts
            .iter()
            .position(|row| row.domain.is_none() && row.diagnostic_range.contains(&index)),
        InstallMode::Unfrozen => owner,
    };

    if install == InstallMode::Frozen {
        let handles = diagnostic.primary_frozen_identity_handle.iter().chain(
            diagnostic
                .labels
                .iter()
                .filter_map(|label| label.frozen_identity_handle.as_ref()),
        );
        for handle in handles.filter(|handle| handle.get().is_none()) {
            if let Some(identity) = handle
                .domain()
                .filter(|identity| !required.contains(identity))
            {
                required.push(identity.clone());
            }
        }
    }

    let context = messages.diagnostic_render_context(index);
    let environment = context
        .type_environment
        .filter(|_| context.path_table().is_some());
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
        none_row,
        merge: None,
    }
}

/// Returns the slot of `database`, adding it when no earlier row named the same allocation.
#[cfg_attr(not(test), allow(dead_code, reason = "P4.5 S3.2 converter"))]
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
#[cfg_attr(not(test), allow(dead_code, reason = "P4.5 S3.2 converter"))]
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
#[cfg_attr(not(test), allow(dead_code, reason = "P4.5 S3.2 converter"))]
fn table_capacity_draft() -> DiagnosticDraft {
    let diagnostic =
        CompilerDiagnostic::source_table_capacity(SourceSpanCapacityResource::LogicalPath);
    let family = families::from_legacy(&diagnostic)
        .expect("a logical-path capacity record always has a compact family");
    finish(family, &diagnostic)
}
