//! Database of registered source slots, loaded snapshots, and cold load failures for one frontend
//! compilation lifetime.
//!
//! The database owns the ordered source-slot inventory, a dense array of loaded snapshots, and a
//! cold array of load failures. Loaded records retain the exact UTF-8 source text plus line-start
//! table for each successful load. Each source's extended-span table is installed at its final
//! producer boundary.
//!
//! [`SourceDatabaseBuilder`] is the exclusive build-stage owner around one database. It retains
//! each loaded source's live [`ExtendedSpanBuilder`] in a dense array aligned to the database's
//! private loaded-record indices, and exposes split views for lookup/mutation. Its terminal finish
//! checks lifecycle invariants without finalizing or moving any source's span table.

use super::line_index::LineIndex;
use super::record::{
    LoadFailureIndex, LoadedSourceIndex, SourceLoadStatus, SourceSlot, ensure_source_snapshot_fits,
};
use super::span::{ExtendedSpanBuilder, ExtendedSpanTable};
use super::{SourceId, SourceKind, SourceProvenance, SourceRecord, SourceRegistrationIndex};
#[cfg(test)]
use crate::builder_surface::SourceFileKind;
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::SourceSpanCapacityResource;
use crate::compiler_frontend::compiler_messages::diagnostics::families::token_lexical;
use crate::compiler_frontend::paths::path_resolution::ProjectPathResolver;
use crate::compiler_frontend::symbols::path_interner::{
    NonUtf8PathComponent, PathId, PathInternError, PathInternerBuilder, PathInternerFork, PathTable,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;

use rustc_hash::FxHashMap;

use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Source identity slots in deterministic logical-path order, plus the snapshots and failures
/// associated with candidates that loaded or failed.
///
/// The database owns three arrays: the ordered source-slot inventory, a dense array of loaded
/// snapshots in load order, and a cold array of load failures. The failure array is separate
/// because failures are rare and too wide for the dense registration row.
///
/// Slot order is the identity order `SourceId` addresses. The loaded array is a separate dense
/// array in load order, reached only through the slot that owns the identity; the failure array is
/// reached through the failure index stored by a failed slot.
///
/// The database is not `Clone`. One boundary registers it once and shares it as an `Arc`, so a
/// deep copy would silently duplicate identities that are meant to be unique for the build.
#[derive(Debug)]
pub struct SourceDatabase {
    slots: Vec<SourceSlot>,
    loaded: Vec<SourceRecord>,
    load_failures: Vec<InfrastructureFailure>,
    canonical_to_id: FxHashMap<PathBuf, SourceId>,
    path_interner: PathInternerBuilder,
}

/// Lookup-only source identity storage published after the mutable build boundary.
///
/// The freeze operation moves every source slot, retained snapshot and path-trie allocation out of
/// [`SourceDatabase`]. Construction-only load failures and canonical-path lookup state are dropped
/// at this boundary. In particular, source text, line-start tables and extended-span entries are
/// never copied. The mutable reverse lookup map owned by [`PathInternerBuilder`] is dropped while
/// its append-only [`PathTable`] is moved here.
///
/// A frozen owner has no registration or loading operations. All identities and snapshots it
/// exposes are the exact ones finalized by [`SourceDatabaseBuilder::finish`].
#[derive(Debug)]
pub struct FrozenSourceDatabase {
    slots: Vec<SourceSlot>,
    loaded: Vec<SourceRecord>,
    paths: Arc<PathTable>,
}
#[cfg(feature = "data_layout_memory_probe")]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SourceDatabaseRetentionMetrics {
    pub(crate) source_snapshot_bytes: usize,
    pub(crate) source_text_capacity_bytes: usize,
    pub(crate) source_line_start_rows: usize,
    pub(crate) source_line_start_bytes: usize,
    pub(crate) source_line_start_capacity_bytes: usize,
    pub(crate) extended_span_rows: usize,
    pub(crate) extended_span_bytes: usize,
    pub(crate) extended_span_duplicate_rows: usize,
    pub(crate) source_identity_slots: usize,
    pub(crate) source_identity_slot_capacity_bytes: usize,
    pub(crate) source_provenance_field_bytes: usize,
    pub(crate) compilation_root_slots: usize,
    pub(crate) authored_physical_slots: usize,
    pub(crate) canonical_os_path_bytes: usize,
    pub(crate) loaded_source_records: usize,
    pub(crate) loaded_source_record_capacity_bytes: usize,
}

#[cfg(feature = "data_layout_memory_probe")]
impl FrozenSourceDatabase {
    pub(crate) fn retention_metrics(&self) -> SourceDatabaseRetentionMetrics {
        source_retention_metrics(
            &self.loaded,
            &self.slots,
            self.loaded.capacity(),
            self.slots.capacity(),
        )
    }
}

#[cfg(feature = "data_layout_memory_probe")]
fn source_retention_metrics(
    loaded: &[SourceRecord],
    slots: &[SourceSlot],
    loaded_capacity: usize,
    slot_capacity: usize,
) -> SourceDatabaseRetentionMetrics {
    use super::span::ExtendedSpanTable;

    SourceDatabaseRetentionMetrics {
        source_snapshot_bytes: loaded.iter().map(|record| record.text.len()).sum(),
        source_text_capacity_bytes: loaded.iter().map(|record| record.text.len()).sum(),
        source_line_start_rows: loaded.iter().map(|record| record.line_starts.len()).sum(),
        source_line_start_bytes: loaded
            .iter()
            .map(|record| record.line_starts.len() * std::mem::size_of::<u32>())
            .sum(),
        source_line_start_capacity_bytes: loaded
            .iter()
            .map(|record| record.line_starts.len() * std::mem::size_of::<u32>())
            .sum(),
        extended_span_rows: loaded
            .iter()
            .filter_map(|record| record.extended_spans.as_ref())
            .map(ExtendedSpanTable::len)
            .sum(),
        extended_span_bytes: loaded
            .iter()
            .filter_map(|record| record.extended_spans.as_ref())
            .map(ExtendedSpanTable::storage_bytes)
            .sum(),
        extended_span_duplicate_rows: loaded
            .iter()
            .filter_map(|record| record.extended_spans.as_ref())
            .map(ExtendedSpanTable::repeated_rows)
            .sum(),
        source_identity_slots: slots.len(),
        source_identity_slot_capacity_bytes: slot_capacity
            .saturating_mul(std::mem::size_of::<SourceSlot>()),
        source_provenance_field_bytes: slots
            .len()
            .saturating_mul(std::mem::size_of::<SourceProvenance>()),
        compilation_root_slots: slots
            .iter()
            .filter(|slot| slot.provenance == SourceProvenance::CompilationRoot)
            .count(),
        authored_physical_slots: slots
            .iter()
            .filter(|slot| slot.provenance == SourceProvenance::AuthoredPhysical)
            .count(),
        canonical_os_path_bytes: slots
            .iter()
            .filter_map(|slot| slot.canonical_os_path.as_deref())
            .map(|path| path.as_os_str().as_encoded_bytes().len())
            .sum(),
        loaded_source_records: loaded.len(),
        loaded_source_record_capacity_bytes: loaded_capacity
            .saturating_mul(std::mem::size_of::<SourceRecord>()),
    }
}

impl Default for SourceDatabase {
    fn default() -> Self {
        Self {
            slots: vec![compilation_root_slot()],
            loaded: Vec::new(),
            load_failures: Vec::new(),
            canonical_to_id: FxHashMap::default(),
            path_interner: PathInternerBuilder::new(),
        }
    }
}

impl SourceDatabase {
    pub fn empty() -> Self {
        Self::default()
    }

    /// Consume the finalized mutable database into lookup-only source storage.
    ///
    /// This is the terminal source lifecycle operation. It moves the registration slots, loaded
    /// source snapshots, line-start tables, installed extended-span tables and path nodes without
    /// copying any of those allocations. Construction-only load failures and canonical-path lookup
    /// state are dropped here. Callers invoke this only after [`SourceDatabaseBuilder::finish`]
    /// has checked that every loaded source received its span table at the final producer
    /// boundary.
    pub(crate) fn freeze(self) -> FrozenSourceDatabase {
        let SourceDatabase {
            slots,
            loaded,
            load_failures: _,
            canonical_to_id: _,
            path_interner,
        } = self;

        FrozenSourceDatabase {
            slots,
            loaded,
            paths: Arc::new(path_interner.freeze()),
        }
    }

    /// Build deterministic source identities from canonical paths alone, for tests.
    ///
    /// Canonical files are sorted by portable logical path before identities are assigned, so
    /// assignment does not depend on filesystem iteration order.
    ///
    /// WHY test-only: every production lane knows the authored kind of what it registers and
    /// supplies it, because canonicalize can resolve a recognized spelling onto a target whose
    /// extension names another kind. A fixture that owns nothing but canonical paths has no such
    /// spelling to lose, so it may derive the kind from the extension instead.
    #[cfg(test)]
    pub fn build<I>(
        canonical_files: I,
        entry_file_path: &Path,
        project_path_resolver: Option<&ProjectPathResolver>,
        string_table: &mut StringTable,
    ) -> Result<Self, SourceDatabaseError>
    where
        I: IntoIterator,
        I::IntoIter: ExactSizeIterator,
        I::Item: AsRef<Path>,
    {
        let owned_items = canonical_files.into_iter().collect::<Vec<_>>();
        let registration_index =
            SourceRegistrationIndex::from_rows(owned_items.iter().map(|item| {
                let path = item.as_ref();
                (path, physical_source_kind(path))
            }));
        Self::from_registration_index_sorted_by_logical_path(
            &registration_index,
            entry_file_path,
            project_path_resolver,
            string_table,
        )
    }

    /// Build source identities from candidates discovered by traversal.
    ///
    /// WHAT: orders the candidates by portable canonical logical path, then assigns identities in
    ///       that order.
    /// WHY: traversal owns no per-source ownership inventory, so it cannot produce Stage 0's
    ///      logical-identity order. Ordering here keeps identity assignment the compiler's
    ///      decision rather than a property of how a producer happened to walk the filesystem.
    #[cfg(test)]
    pub(crate) fn from_registration_index_sorted_by_logical_path(
        registration_index: &SourceRegistrationIndex<'_>,
        entry_file_path: &Path,
        project_path_resolver: Option<&ProjectPathResolver>,
        string_table: &mut StringTable,
    ) -> Result<Self, SourceDatabaseError> {
        Self::from_registration_index_sorted_by_logical_path_with_path_builder(
            registration_index,
            entry_file_path,
            project_path_resolver,
            string_table,
            PathInternerBuilder::new(),
        )
    }

    /// Build deterministic source identities while retaining a pre-existing path domain.
    ///
    /// Discovery prepares source facts against a live path fork. Reusing that fork's complete
    /// builder here keeps every already-issued `PathId` valid when source slots are finalized.
    pub(crate) fn from_registration_index_sorted_by_logical_path_with_path_builder(
        registration_index: &SourceRegistrationIndex<'_>,
        entry_file_path: &Path,
        project_path_resolver: Option<&ProjectPathResolver>,
        string_table: &mut StringTable,
        path_interner: PathInternerBuilder,
    ) -> Result<Self, SourceDatabaseError> {
        let mut database = Self {
            slots: vec![compilation_root_slot()],
            loaded: Vec::new(),
            load_failures: Vec::new(),
            canonical_to_id: FxHashMap::default(),
            path_interner,
        };
        let mut rows = logical_rows_for_registration_index(
            registration_index,
            entry_file_path,
            project_path_resolver,
        )?;
        rows.sort_by(|(_, _, left), (_, _, right)| {
            left.portable_sort_key.cmp(&right.portable_sort_key)
        });
        database.append_ordered_logical_rows(rows, string_table)?;
        Ok(database)
    }

    /// Build source identities from the ordered candidates produced by Stage 0.
    ///
    /// WHAT: preserves the registration index's already-sorted Stage 0 logical-identity order
    ///       while computing each slot's compiler-facing logical path.
    /// WHY: Stage 0 owns module origin and rootedness, so it is the authority that can order by
    ///      `SourceLogicalIdentity`. The compiler must not re-sort those rows by display logical
    ///      path.
    pub(crate) fn from_ordered_registration_index(
        registration_index: &SourceRegistrationIndex<'_>,
        entry_file_path: &Path,
        project_path_resolver: Option<&ProjectPathResolver>,
        string_table: &mut StringTable,
    ) -> Result<Self, SourceDatabaseError> {
        let mut database = Self::empty();
        database.append_ordered_registration_index(
            registration_index,
            entry_file_path,
            project_path_resolver,
            string_table,
        )?;
        Ok(database)
    }

    /// Append the ordered Stage 0 candidates to this boundary's source identity table.
    ///
    /// WHAT: assigns each candidate the next deterministic ID in the index's existing order.
    /// WHY: Stage 0 already sorted by logical identity. A caller may already have registered a
    ///      bootstrap source, such as `config.moth`, before discovery supplies the remaining rows.
    pub(crate) fn append_ordered_registration_index(
        &mut self,
        registration_index: &SourceRegistrationIndex<'_>,
        entry_file_path: &Path,
        project_path_resolver: Option<&ProjectPathResolver>,
        string_table: &mut StringTable,
    ) -> Result<(), SourceDatabaseError> {
        let rows = logical_rows_for_registration_index(
            registration_index,
            entry_file_path,
            project_path_resolver,
        )?;
        self.append_ordered_logical_rows(rows, string_table)
    }

    fn append_ordered_logical_rows(
        &mut self,
        rows: Vec<(PathBuf, SourceKind, LogicalSourcePath)>,
        string_table: &mut StringTable,
    ) -> Result<(), SourceDatabaseError> {
        // Canonical ordering precedes both path interning and source identity assignment.
        let interned_rows = rows
            .into_iter()
            .map(|(canonical, kind, logical)| {
                let path_id = self
                    .path_interner
                    .try_intern_filesystem_path(&logical.path, string_table)
                    .map_err(map_path_intern_error)?;
                Ok((canonical, kind, path_id))
            })
            .collect::<Result<Vec<_>, SourceDatabaseError>>()?;

        for (canonical, kind, path_id) in interned_rows {
            if self.canonical_to_id.contains_key(&canonical) {
                return Err(SourceDatabaseError::Infrastructure(
                    InfrastructureFailure::internal(
                        CompilerStage::SourceLoading,
                        format!(
                            "Source identity inventory registered canonical source path {} more than once",
                            canonical.display(),
                        ),
                    ),
                ));
            }

            self.push_slot(canonical, path_id, kind)?;
        }

        Ok(())
    }

    pub fn iter(&self) -> std::slice::Iter<'_, SourceSlot> {
        debug_assert_eq!(
            self.slots.first().map(|slot| slot.provenance),
            Some(SourceProvenance::CompilationRoot)
        );
        self.slots[1..].iter()
    }

    pub fn get_by_canonical_path(&self, canonical_path: &Path) -> Option<&SourceSlot> {
        let id = self.canonical_to_id.get(canonical_path)?;
        self.get(*id)
    }

    /// Borrow the shared source-path table while this database remains mutable.
    pub(crate) fn paths(&self) -> &PathTable {
        self.path_interner.paths()
    }

    /// Share the registration prefix with a compilation-owned sibling builder.
    ///
    /// WHAT: seals the current path suffix into shared immutable storage and gives compilation a
    ///       builder that appends its own local delta without mutating this database.
    /// WHY: source logical `PathId`s remain the identity prefix, while module-local nodes append
    ///      after them and are adopted back before freeze without copying the full path table.
    pub(crate) fn fork_path_builder(&mut self) -> PathInternerBuilder {
        self.path_interner.fork_with_shared_prefix()
    }

    /// Borrow the registration builder mutably for an append-only path-snapshot merge.
    ///
    /// WHAT: hands the source-path builder to a converter that merges a frozen path snapshot.
    /// WHY: a converted diagnostic carries its own path IDs, so they must be remapped into this
    ///      builder; the merge only appends, so existing source `PathId`s keep their numbers.
    pub(crate) fn path_builder_mut(&mut self) -> &mut PathInternerBuilder {
        &mut self.path_interner
    }

    /// Fork the registered logical-path table for one local frontend preparation wave.
    pub(crate) fn fork_path_interner(
        &self,
    ) -> crate::compiler_frontend::symbols::path_interner::PathInternerFork {
        self.path_interner.fork_source().fork_for_module()
    }

    /// Adopt a compilation-owned sibling builder back before the freeze.
    ///
    /// WHAT: replaces the registration prefix table with the sibling's merged full table.
    /// WHY: the sibling shares the source prefix, so adopted source-slot `PathId`s keep their
    ///      numeric identities while merged module-local nodes join the frozen table.
    pub(crate) fn adopt_path_builder(&mut self, builder: PathInternerBuilder) {
        debug_assert!(self.path_interner.len() <= builder.len());
        self.path_interner = builder;
    }

    /// Re-intern a registered source's logical path into a caller-owned path fork.
    ///
    /// Source registration owns its own append-only table while discovery is assembling the
    /// closure is complete. Tokenization and header preparation may use a different fork, so the
    /// numeric source path must be walked by components rather than copied as a foreign
    /// `PathId`. Fork-table exhaustion is the same authored capacity rejection as registration.
    pub(crate) fn logical_path_in_fork(
        &self,
        source: SourceId,
        path_fork: &mut PathInternerFork,
    ) -> Result<PathId, SourceDatabaseError> {
        let logical_path = self
            .get(source)
            .ok_or_else(|| {
                SourceDatabaseError::Infrastructure(InfrastructureFailure::internal(
                    CompilerStage::SourceLoading,
                    format!(
                        "source identity {} is absent while re-interning its logical path",
                        source.index()
                    ),
                ))
            })?
            .logical_path;
        let table = self.paths();
        let depth = table.try_depth(logical_path).ok_or_else(|| {
            SourceDatabaseError::Infrastructure(InfrastructureFailure::internal(
                CompilerStage::SourceLoading,
                format!(
                    "source identity {} carries an invalid logical path",
                    source.index()
                ),
            ))
        })?;
        let mut components = Vec::with_capacity(depth as usize);
        table.resolve_components(logical_path, &mut components);
        path_fork
            .try_intern_components(&components)
            .ok_or(SourceDatabaseError::Capacity(
                SourceCapacityError::LogicalPathTableFull,
            ))
    }

    /// Names a rejected authored snapshot by its logical path.
    ///
    /// WHAT: an oversize snapshot becomes a source-span capacity draft named by the source's
    ///       logical path, re-interned in `path_fork`. Every other rejection is returned unchanged.
    /// WHY: only an authored file can exceed the snapshot limit, so the message must name it.
    ///      The draft's path ID lives in `path_fork`, so callers must capture that fork's table.
    pub(crate) fn snapshot_rejection_draft(
        &self,
        error: SourceDatabaseError,
        source_id: SourceId,
        path_fork: &mut PathInternerFork,
    ) -> Result<DiagnosticDraft, SourceDatabaseError> {
        match error {
            SourceDatabaseError::Capacity(SourceCapacityError::SourceSnapshotTooLarge) => {
                let path = self.logical_path_in_fork(source_id, path_fork)?;
                Ok(token_lexical::source_span_capacity_named(
                    0,
                    u32::MAX,
                    SourceSpanCapacityResource::SourceSnapshot,
                    Some(path),
                    None,
                ))
            }
            other => Err(other),
        }
    }

    /// Look up the exact source snapshot retained for a physical source identity.
    ///
    /// The reserved compilation root is excluded by [`Self::get`], so it cannot accidentally
    /// become a physical source frame.
    pub fn retained_text(&self, id: SourceId) -> Option<&str> {
        self.loaded_record(id).map(|record| record.text.as_ref())
    }

    /// Construct a line index for the retained snapshot of one physical source.
    ///
    /// The loaded array remains private to the database; callers address its record through the
    /// source identity carried by the corresponding registration slot.
    #[cfg(test)]
    pub(crate) fn line_index(&self, id: SourceId) -> Option<LineIndex<'_>> {
        let record = self.loaded_record(id)?;
        Some(LineIndex::new(&record.text, &record.line_starts))
    }

    pub(crate) fn source_load_error(&self, id: SourceId) -> Option<&InfrastructureFailure> {
        match &self.get(id)?.load {
            SourceLoadStatus::Failed(index) => self.load_failures.get(index.index()),
            SourceLoadStatus::Pending | SourceLoadStatus::NonUtf8 | SourceLoadStatus::Loaded(_) => {
                None
            }
        }
    }

    /// Move one loaded source snapshot into a record owned by its preassigned slot.
    ///
    /// A slot that already holds a snapshot or a load failure is a lifecycle violation, so the
    /// size policy is applied only to a slot that can still accept one. Otherwise an oversized
    /// second snapshot would report the user's file instead of the compiler's own mistake.
    pub(crate) fn retain_text(
        &mut self,
        id: SourceId,
        text: String,
    ) -> Result<(), SourceDatabaseError> {
        let provenance = {
            let slot = self.source_slot_mut(id)?;
            if !matches!(slot.load, SourceLoadStatus::Pending) {
                return Err(InfrastructureFailure::internal(
                    CompilerStage::SourceLoading,
                    format!(
                        "source text for source identity {} was retained more than once",
                        id.index()
                    ),
                )
                .into());
            }
            slot.provenance
        };

        // Reject before building line starts: the snapshot's offsets must be addressable by `u32`.
        ensure_source_snapshot_fits(text.len(), provenance)?;

        let line_starts = super::line_index::line_start_offsets(&text);
        let loaded_index = LoadedSourceIndex::from_index(self.loaded.len());
        self.loaded.push(SourceRecord {
            text: text.into_boxed_str(),
            line_starts,
            extended_spans: None,
        });

        let slot = self
            .slots
            .get_mut(id.index())
            .expect("source slot validated before retaining text");
        debug_assert!(matches!(slot.load, SourceLoadStatus::Pending));
        slot.load = SourceLoadStatus::Loaded(loaded_index);
        Ok(())
    }

    /// Install the one frozen extended-span table produced for a loaded source.
    ///
    /// Installation is monotonic: a pending or failed source never receives a table, and a
    /// loaded source can transition from absent to installed only once.
    // The exclusive source owner installs each table at its final producer boundary.
    fn install_extended_spans(
        &mut self,
        id: SourceId,
        extended_spans: ExtendedSpanTable,
    ) -> Result<(), InfrastructureFailure> {
        let loaded_index = {
            let slot = self.slots.get(id.index()).ok_or_else(|| {
                InfrastructureFailure::internal(
                    CompilerStage::SourceLoading,
                    format!(
                        "source identity {} is absent from the source database",
                        id.index()
                    ),
                )
            })?;

            if slot.provenance == SourceProvenance::CompilationRoot {
                return Err(InfrastructureFailure::internal(
                    CompilerStage::SourceLoading,
                    format!(
                        "extended spans cannot be installed on compilation root source identity {}",
                        id.index()
                    ),
                ));
            }

            match slot.load {
                SourceLoadStatus::Loaded(index) => index,
                SourceLoadStatus::Pending
                | SourceLoadStatus::Failed(_)
                | SourceLoadStatus::NonUtf8 => {
                    return Err(InfrastructureFailure::internal(
                        CompilerStage::SourceLoading,
                        format!(
                            "extended spans for source identity {} were installed before its source \
                         was loaded",
                            id.index()
                        ),
                    ));
                }
            }
        };

        let record = self
            .loaded
            .get_mut(loaded_index.index())
            .expect("loaded source status points outside the loaded record array");

        if record.extended_spans.is_some() {
            return Err(InfrastructureFailure::internal(
                CompilerStage::SourceLoading,
                format!(
                    "extended spans for source identity {} were installed more than once",
                    id.index()
                ),
            ));
        }

        record.extended_spans = Some(extended_spans);
        Ok(())
    }

    /// Record a source-read failure in its preassigned slot without aborting the whole build.
    pub(crate) fn record_source_load_error(
        &mut self,
        id: SourceId,
        error: InfrastructureFailure,
    ) -> Result<(), InfrastructureFailure> {
        self.ensure_slot_pending(id)?;
        let failure_index = LoadFailureIndex::from_index(self.load_failures.len());
        self.load_failures.push(error);
        self.set_slot_load_status(id, SourceLoadStatus::Failed(failure_index));
        Ok(())
    }

    /// Record that a registered source's bytes were read but are not UTF-8.
    ///
    /// WHAT: moves the slot to the terminal `NonUtf8` status; the caller builds the diagnostic.
    /// WHY: a slot left `Pending` would claim that loading was never attempted.
    pub(crate) fn record_source_non_utf8(
        &mut self,
        id: SourceId,
    ) -> Result<(), InfrastructureFailure> {
        self.ensure_slot_pending(id)?;
        self.set_slot_load_status(id, SourceLoadStatus::NonUtf8);
        Ok(())
    }

    fn ensure_slot_pending(&mut self, id: SourceId) -> Result<(), InfrastructureFailure> {
        let slot = self.source_slot_mut(id)?;
        if !matches!(slot.load, SourceLoadStatus::Pending) {
            return Err(InfrastructureFailure::internal(
                CompilerStage::SourceLoading,
                format!(
                    "source load status for source identity {} was recorded more than once",
                    id.index()
                ),
            ));
        }
        Ok(())
    }

    fn set_slot_load_status(&mut self, id: SourceId, status: SourceLoadStatus) {
        let slot = self
            .slots
            .get_mut(id.index())
            .expect("source slot validated before recording load status");
        debug_assert!(matches!(slot.load, SourceLoadStatus::Pending));
        slot.load = status;
    }

    fn source_slot_mut(&mut self, id: SourceId) -> Result<&mut SourceSlot, InfrastructureFailure> {
        let slot = self.slots.get_mut(id.index()).ok_or_else(|| {
            InfrastructureFailure::internal(
                CompilerStage::SourceLoading,
                format!(
                    "source identity {} is absent from the source database",
                    id.index()
                ),
            )
        })?;
        if slot.provenance == SourceProvenance::CompilationRoot {
            return Err(InfrastructureFailure::internal(
                CompilerStage::SourceLoading,
                "source snapshots cannot be retained on the compilation root",
            ));
        }
        Ok(slot)
    }

    fn loaded_record(&self, id: SourceId) -> Option<&SourceRecord> {
        let slot = self.get(id)?;
        let loaded_index = match slot.load {
            SourceLoadStatus::Loaded(index) => index,
            SourceLoadStatus::Pending | SourceLoadStatus::Failed(_) | SourceLoadStatus::NonUtf8 => {
                return None;
            }
        };
        self.loaded.get(loaded_index.index())
    }

    /// Resolve the loaded record addressed by a source identity for frozen span consumers.
    ///
    /// The reserved compilation root is not resolved here: its database-backed span resolution
    /// is the contract in [`span.rs`](super::span) and needs no record. Missing, pending, failed
    /// and root identities remain compiler bugs at this boundary.
    pub(super) fn source_record(&self, id: SourceId) -> &SourceRecord {
        let slot = self.slots.get(id.index()).unwrap_or_else(|| {
            crate::compiler_bug!(
                CompilerStage::SourceLoading,
                "source identity {} is absent from the source database; this is a compiler bug",
                id.index()
            )
        });

        if slot.provenance == SourceProvenance::CompilationRoot {
            crate::compiler_bug!(
                CompilerStage::SourceLoading,
                "source identity {} is the compilation root and has no loaded source record; \
                 the root resolves only the exact empty span range [0, 0) through the span \
                 module, never a record; this is a compiler bug",
                id.index()
            );
        }

        let loaded_index = match slot.load {
            SourceLoadStatus::Loaded(index) => index,
            SourceLoadStatus::Pending | SourceLoadStatus::Failed(_) | SourceLoadStatus::NonUtf8 => {
                crate::compiler_bug!(
                    CompilerStage::SourceLoading,
                    "source identity {} has no loaded source record; this is a compiler bug",
                    id.index()
                )
            }
        };

        self.loaded.get(loaded_index.index()).unwrap_or_else(|| {
            crate::compiler_bug!(
                CompilerStage::SourceLoading,
                "source identity {} points outside the loaded source records; this is a compiler \
                 bug",
                id.index()
            )
        })
    }

    /// Register a source in a private traversal-only discovery domain.
    ///
    /// Use this only when preparation discovers source membership. The discovery finalization
    /// barrier assigns deterministic final identities and normalizes retained source facts before
    /// publication. Inventory-backed lanes use ordered registration instead.
    ///
    /// Re-registering a canonical path preserves its identity only when the logical path and
    /// authored kind agree. Conflicts indicate a compiler invariant failure.
    pub(crate) fn insert(
        &mut self,
        canonical_path: PathBuf,
        kind: SourceKind,
        entry_file_path: &Path,
        project_path_resolver: Option<&ProjectPathResolver>,
        string_table: &mut StringTable,
    ) -> Result<SourceId, SourceDatabaseError> {
        let logical = logical_source_path(&canonical_path, entry_file_path, project_path_resolver)?;
        let path_id = self
            .path_interner
            .try_intern_filesystem_path(&logical.path, string_table)
            .map_err(map_path_intern_error)?;

        if let Some(slot) = self.get_by_canonical_path(&canonical_path) {
            if slot.logical_path != path_id {
                let existing_path_id = slot.logical_path;
                let mut scratch = Vec::new();
                let existing_path =
                    self.paths()
                        .render_portable(existing_path_id, string_table, &mut scratch);
                let requested_path =
                    self.paths()
                        .render_portable(path_id, string_table, &mut scratch);
                return Err(SourceDatabaseError::Infrastructure(
                    InfrastructureFailure::internal(
                        CompilerStage::SourceLoading,
                        format!(
                            "Source identity inventory registered canonical source path {} under \
                         conflicting logical paths {} and {}",
                            canonical_path.display(),
                            existing_path,
                            requested_path,
                        ),
                    ),
                ));
            }
            if let Some(stored_kind) = slot.kind
                && stored_kind != kind
            {
                return Err(SourceDatabaseError::Infrastructure(
                    InfrastructureFailure::internal(
                        CompilerStage::SourceLoading,
                        format!(
                            "Source identity inventory registered canonical source path {} under \
                         conflicting kinds {:?} and {:?}",
                            canonical_path.display(),
                            stored_kind,
                            kind,
                        ),
                    ),
                ));
            }
            return Ok(slot.id);
        }
        self.push_slot(canonical_path, path_id, kind)
    }

    fn push_slot(
        &mut self,
        canonical_path: PathBuf,
        logical_path: PathId,
        kind: SourceKind,
    ) -> Result<SourceId, SourceDatabaseError> {
        #[cfg(test)]
        if crate::compiler_frontend::symbols::path_interner::test_exhaustion::forced_exhaustion() {
            // Test-only: reject the new source identity so tests reach the exhaustion boundary
            // without registering the full compact source domain.
            return Err(SourceDatabaseError::Capacity(
                SourceCapacityError::SourceIdentityTableFull,
            ));
        }
        let id = SourceId::try_from_index(self.slots.len()).ok_or(
            SourceDatabaseError::Capacity(SourceCapacityError::SourceIdentityTableFull),
        )?;
        self.canonical_to_id.insert(canonical_path.clone(), id);
        self.slots.push(SourceSlot {
            id,
            canonical_os_path: Some(canonical_path.into()),
            logical_path,
            kind: Some(kind),
            provenance: SourceProvenance::AuthoredPhysical,
            load: SourceLoadStatus::Pending,
        });
        Ok(id)
    }

    /// Drop one registered source slot without touching the path table (test-only).
    ///
    /// Identity exhaustion tests re-register a path whose node already exists; removing the slot
    /// keeps the path interner a pure lookup so the forced gate can fail the identity allocation.
    #[cfg(test)]
    pub(crate) fn remove_for_test_reuse(&mut self, canonical_path: &PathBuf) -> Option<()> {
        let id = self.canonical_to_id.remove(canonical_path)?;
        let index = id.index();
        (index < self.slots.len() && self.slots[index].id == id).then(|| {
            self.slots.remove(index);
        })
    }

    pub fn get(&self, id: SourceId) -> Option<&SourceSlot> {
        let slot = self.slots.get(id.index())?;
        (slot.provenance != SourceProvenance::CompilationRoot).then_some(slot)
    }

    /// Resolve the unique physical slot for a logical path, if one exists.
    ///
    /// A logical path is safe to render only when it identifies exactly one slot in this
    /// database. Collisions can arise when independently rooted sources share a portable spelling;
    /// returning no slot on ambiguity is safer than guessing and displaying another file's text.
    #[cfg(test)]
    pub(crate) fn unique_record_for_logical_path(
        &self,
        logical_path: PathId,
    ) -> Option<&SourceSlot> {
        let mut matches = self.iter().filter(|slot| slot.logical_path == logical_path);
        let slot = matches.next()?;
        if matches.next().is_some() {
            return None;
        }
        Some(slot)
    }

    /// Return the compact logical path identity assigned to one physical source.
    #[cfg(test)]
    pub(crate) fn source_logical_path(&self, id: SourceId) -> Option<PathId> {
        self.get(id).map(|slot| slot.logical_path)
    }
}
impl FrozenSourceDatabase {
    /// Builds a frozen database with no sources, so an unowned domain carries only its path trie.
    ///
    /// Every span it names misses the slot table and resolves to no position.
    pub(crate) fn with_paths(paths: Arc<PathTable>) -> Self {
        Self {
            slots: Vec::new(),
            loaded: Vec::new(),
            paths,
        }
    }

    /// Resolve one physical source slot by its compact identity.
    ///
    /// The compilation-root identity is intentionally excluded: it owns no physical source
    /// record, path or snapshot.
    pub(crate) fn get(&self, id: SourceId) -> Option<&SourceSlot> {
        let slot = self.slots.get(id.index())?;
        (slot.provenance != SourceProvenance::CompilationRoot).then_some(slot)
    }

    /// Borrow the immutable logical-path trie moved across the source freeze boundary.
    #[inline]
    pub(crate) fn paths(&self) -> &PathTable {
        &self.paths
    }

    /// Return the compact logical path identity assigned to one physical source.
    pub(crate) fn source_logical_path(&self, id: SourceId) -> Option<PathId> {
        self.get(id).map(|slot| slot.logical_path)
    }

    /// Construct a line index over one retained frozen snapshot.
    pub(crate) fn line_index(&self, id: SourceId) -> Option<LineIndex<'_>> {
        let record = self.loaded_record(id)?;
        Some(LineIndex::new(&record.text, &record.line_starts))
    }

    /// Resolve the loaded source record addressed by a physical source identity.
    ///
    /// Missing, pending, failed and compilation-root identities are compiler invariant failures
    /// at this boundary, just as they are for the mutable database's frozen-span lookup.
    pub(crate) fn source_record(&self, id: SourceId) -> &SourceRecord {
        let slot = self.slots.get(id.index()).unwrap_or_else(|| {
            crate::compiler_bug!(CompilerStage::SourceLoading,
                "source identity {} is absent from the frozen source database; this is a compiler bug",
                id.index()
            )
        });

        if slot.provenance == SourceProvenance::CompilationRoot {
            crate::compiler_bug!(
                CompilerStage::SourceLoading,
                "source identity {} is the compilation root and has no loaded source record; \
                 this is a compiler bug",
                id.index()
            );
        }

        let loaded_index = match slot.load {
            SourceLoadStatus::Loaded(index) => index,
            SourceLoadStatus::Pending | SourceLoadStatus::Failed(_) | SourceLoadStatus::NonUtf8 => {
                crate::compiler_bug!(
                    CompilerStage::SourceLoading,
                    "source identity {} has no loaded source record in the frozen source \
                     database; this is a compiler bug",
                    id.index()
                )
            }
        };

        self.loaded.get(loaded_index.index()).unwrap_or_else(|| {
            crate::compiler_bug!(
                CompilerStage::SourceLoading,
                "source identity {} points outside the frozen source records; this is a \
                 compiler bug",
                id.index()
            )
        })
    }

    fn loaded_record(&self, id: SourceId) -> Option<&SourceRecord> {
        let slot = self.get(id)?;
        let loaded_index = match slot.load {
            SourceLoadStatus::Loaded(index) => index,
            SourceLoadStatus::Pending | SourceLoadStatus::Failed(_) | SourceLoadStatus::NonUtf8 => {
                return None;
            }
        };
        self.loaded.get(loaded_index.index())
    }
}

fn compilation_root_slot() -> SourceSlot {
    SourceSlot {
        id: SourceId::COMPILATION_ROOT,
        canonical_os_path: None,
        logical_path: PathId::ROOT,
        kind: None,
        provenance: SourceProvenance::CompilationRoot,
        load: SourceLoadStatus::Pending,
    }
}

/// The reserved compilation-root slot keeps no snapshot and never loads. Its database-backed
/// span resolution accepts only the exact empty range `[0, 0)` and is owned by the span module.
///
/// Exclusive source ownership until all preparation and semantic producers have finished.
///
/// Span states use the database's private dense loaded-record index. Registration-only slots
/// allocate no builder, and a split borrow lets producers read snapshots without sharing mutation.
/// Semantic Stage 0 facts may hold private immutable snapshot handles during a call. All such
/// handles must be released before source-local finalization recovers exclusive ownership to
/// install each table in its preassigned record.
///
/// Construction state enters owned through [`SourceDatabaseBuilder::new`] and leaves owned
/// through [`SourceDatabaseBuilder::finish`]. The interior `Arc` is only the transient share
/// handle that boundary compilation clones for retained Stage 0 facts; every such clone is
/// dropped before the terminal publish `Arc` is minted after the consume.
#[derive(Debug)]
pub(crate) struct SourceDatabaseBuilder {
    sources: Arc<SourceDatabase>,
    live_builders: Vec<SpanBuilderState>,
    pending_builders: FxHashMap<SourceId, PendingSpanBuilder>,
}

#[derive(Debug)]
enum PendingSpanBuilder {
    CheckedOut,
    Live(ExtendedSpanBuilder),
    Finished(ExtendedSpanTable),
}

#[derive(Debug)]
enum SpanBuilderState {
    Unprepared,
    CheckedOut,
    Live(ExtendedSpanBuilder),
    Finished(ExtendedSpanTable),
    Installed,
}
impl SourceDatabaseBuilder {
    /// Take exclusive construction ownership of one registered database.
    ///
    /// WHY: an `Arc` into construction would smuggle outstanding snapshots past the freeze.
    /// Callers holding a previously published `Arc` resolve exclusivity explicitly with
    /// `Arc::try_unwrap` before calling this.
    pub(crate) fn new(sources: SourceDatabase) -> Self {
        Self {
            sources: Arc::new(sources),
            live_builders: Vec::new(),
            pending_builders: FxHashMap::default(),
        }
    }

    pub(crate) fn sources(&self) -> &Arc<SourceDatabase> {
        &self.sources
    }

    pub(crate) fn sources_mut(&mut self) -> &mut SourceDatabase {
        Arc::get_mut(&mut self.sources).expect(
            "source registration changed while snapshots were shared; this is a compiler bug",
        )
    }

    pub(crate) fn split(&mut self) -> (&Arc<SourceDatabase>, SourceSpanBuilders<'_>) {
        self.sync_loaded_records();
        (
            &self.sources,
            SourceSpanBuilders {
                sources: &self.sources,
                live_builders: &mut self.live_builders,
                pending_builders: &mut self.pending_builders,
            },
        )
    }

    pub(crate) fn retain_span_builder(&mut self, source: SourceId, builder: ExtendedSpanBuilder) {
        self.split().1.retain_span_builder(source, builder);
    }

    /// Finalize one source's span table at its last producer boundary.
    ///
    /// The builder state owns the frozen table until terminal container assembly, so this never
    /// mutates the shared source database while compilation still borrows its snapshots.
    pub(crate) fn finish_source(&mut self, source: SourceId) -> Result<(), InfrastructureFailure> {
        self.split().1.finish_source(source)
    }

    /// Whether one source's span table has crossed its final producer boundary.
    pub(crate) fn is_source_finalized(
        &mut self,
        source: SourceId,
    ) -> Result<bool, InfrastructureFailure> {
        self.split().1.is_source_finalized(source)
    }

    /// Move finalized tables into their preassigned records after every shared source view drops.
    ///
    /// This is a pure ownership transfer: it does not freeze builders or allocate replacement
    /// tables. [`Self::finish`] remains invariant checking only.
    pub(crate) fn assemble_finalized_tables(&mut self) -> Result<(), InfrastructureFailure> {
        self.sync_loaded_records();
        if !self.pending_builders.is_empty() {
            return Err(InfrastructureFailure::internal(
                CompilerStage::SourceLoading,
                "pending span builders escaped source finalization",
            ));
        }

        for slot_index in 0..self.sources.slots.len() {
            let (source, loaded_index) = {
                let slot = &self.sources.slots[slot_index];
                let SourceLoadStatus::Loaded(loaded_index) = slot.load else {
                    continue;
                };
                (slot.id, loaded_index)
            };
            let index = loaded_index.index();
            match &self.live_builders[index] {
                SpanBuilderState::Installed => {
                    if self.sources.loaded[index].extended_spans.is_none() {
                        return Err(InfrastructureFailure::internal(
                            CompilerStage::SourceLoading,
                            format!(
                                "source identity {} was marked installed without an extended-span table",
                                source.index()
                            ),
                        ));
                    }
                }
                SpanBuilderState::Finished(_) => {
                    let SpanBuilderState::Finished(table) = std::mem::replace(
                        &mut self.live_builders[index],
                        SpanBuilderState::Unprepared,
                    ) else {
                        crate::compiler_bug!(
                            CompilerStage::SourceLoading,
                            "finished span table changed during assembly"
                        )
                    };
                    Arc::get_mut(&mut self.sources)
                        .expect(
                            "source context escaped before finalized table assembly; this is a compiler bug",
                        )
                        .install_extended_spans(source, table)?;
                    self.live_builders[index] = SpanBuilderState::Installed;
                }
                SpanBuilderState::Unprepared => {
                    return Err(InfrastructureFailure::internal(
                        CompilerStage::SourceLoading,
                        format!(
                            "source identity {} was not finalized before source table assembly",
                            source.index()
                        ),
                    ));
                }
                SpanBuilderState::Live(_) => {
                    return Err(InfrastructureFailure::internal(
                        CompilerStage::SourceLoading,
                        format!(
                            "source identity {} still owns a live span builder at source table assembly",
                            source.index()
                        ),
                    ));
                }
                SpanBuilderState::CheckedOut => {
                    return Err(InfrastructureFailure::internal(
                        CompilerStage::SourceLoading,
                        format!(
                            "source identity {} still has an outstanding span producer at source table assembly",
                            source.index()
                        ),
                    ));
                }
            }
        }
        Ok(())
    }

    /// Move builders collected while discovery was loading selected source text into the
    /// database's dense source state.
    ///
    /// Discovery can finish a source-local table before retaining its selected snapshot. The
    /// finished table stays in the pending side table until the snapshot is adopted.
    pub(crate) fn adopt_pending_span_builders(&mut self) {
        self.sync_loaded_records();
        let pending = std::mem::take(&mut self.pending_builders);
        for (source, pending_builder) in pending {
            let slot = self
                .sources
                .get(source)
                .expect("pending span producer source must be registered; this is a compiler bug");
            let SourceLoadStatus::Loaded(index) = slot.load else {
                crate::compiler_bug!(
                    CompilerStage::SourceLoading,
                    "pending span producer source {} was not retained before adoption; this is a compiler bug",
                    source.index()
                );
            };
            let live = &mut self.live_builders[index.index()];
            assert!(
                matches!(live, SpanBuilderState::Unprepared),
                "source {} already owns a live span builder during pending adoption; this is a compiler bug",
                source.index()
            );
            *live = match pending_builder {
                PendingSpanBuilder::CheckedOut => crate::compiler_bug!(
                    CompilerStage::SourceLoading,
                    "source {} still has an outstanding span producer during pending adoption; this is a compiler bug",
                    source.index()
                ),
                PendingSpanBuilder::Live(builder) => SpanBuilderState::Live(builder),
                PendingSpanBuilder::Finished(table) => SpanBuilderState::Finished(table),
            };
        }
    }
    /// Check that every loaded source was assembled at its producer boundary and consume
    /// construction state into lookup-only storage.
    ///
    /// WHY: the terminal publish `Arc` must be minted after this consume, never shared out of
    /// the builder. Construction-only load failures and canonical-path lookup state are dropped
    /// before publishing finalized slots, snapshots and the path table. An outstanding transient
    /// share is an ownership bug.
    pub(crate) fn finish(self) -> Result<SourceDatabase, InfrastructureFailure> {
        if !self.pending_builders.is_empty() {
            return Err(InfrastructureFailure::internal(
                CompilerStage::SourceLoading,
                "pending span builders escaped source finalization",
            ));
        }
        let sources = Arc::try_unwrap(self.sources)
            .expect("source context escaped before span finalization; this is a compiler bug");
        if self.live_builders.len() != sources.loaded.len() {
            return Err(InfrastructureFailure::internal(
                CompilerStage::SourceLoading,
                "source span-builder table is out of sync with loaded source records",
            ));
        }
        for slot in &sources.slots {
            let SourceLoadStatus::Loaded(loaded_index) = slot.load else {
                continue;
            };
            let index = loaded_index.index();
            match &self.live_builders[index] {
                SpanBuilderState::Installed => {
                    if sources.loaded[index].extended_spans.is_none() {
                        return Err(InfrastructureFailure::internal(
                            CompilerStage::SourceLoading,
                            format!(
                                "source identity {} was finalized without an extended-span table",
                                slot.id.index()
                            ),
                        ));
                    }
                }
                SpanBuilderState::Finished(_) => {
                    return Err(InfrastructureFailure::internal(
                        CompilerStage::SourceLoading,
                        format!(
                            "source identity {} was finalized but not assembled before source database finish",
                            slot.id.index()
                        ),
                    ));
                }
                SpanBuilderState::Unprepared => {
                    return Err(InfrastructureFailure::internal(
                        CompilerStage::SourceLoading,
                        format!(
                            "source identity {} was not finalized before source database finish",
                            slot.id.index()
                        ),
                    ));
                }
                SpanBuilderState::Live(_) => {
                    return Err(InfrastructureFailure::internal(
                        CompilerStage::SourceLoading,
                        format!(
                            "source identity {} still owns a live span builder at source database finish",
                            slot.id.index()
                        ),
                    ));
                }
                SpanBuilderState::CheckedOut => {
                    return Err(InfrastructureFailure::internal(
                        CompilerStage::SourceLoading,
                        format!(
                            "source identity {} still has an outstanding span producer at source database finish",
                            slot.id.index()
                        ),
                    ));
                }
            }
        }
        Ok(sources)
    }

    fn sync_loaded_records(&mut self) {
        assert!(
            self.live_builders.len() <= self.sources.loaded.len(),
            "loaded source records were removed while builders were live; this is a compiler bug"
        );
        let first_unsynced = self.live_builders.len();
        self.live_builders
            .extend(self.sources.loaded[first_unsynced..].iter().map(|record| {
                if record.extended_spans.is_some() {
                    SpanBuilderState::Installed
                } else {
                    SpanBuilderState::Unprepared
                }
            }));
    }
}

/// Disjoint mutable span ownership beside an immutable source lookup borrow.
pub(crate) struct SourceSpanBuilders<'a> {
    sources: &'a SourceDatabase,
    live_builders: &'a mut [SpanBuilderState],
    pending_builders: &'a mut FxHashMap<SourceId, PendingSpanBuilder>,
}

impl<'a> SourceSpanBuilders<'a> {
    /// Snapshot lookup borrows the database, independently of the mutable span-state borrow.
    pub(crate) fn sources(&self) -> &'a SourceDatabase {
        self.sources
    }

    pub(crate) fn is_source_finalized(
        &self,
        source: SourceId,
    ) -> Result<bool, InfrastructureFailure> {
        let slot = self.sources.get(source).ok_or_else(|| {
            InfrastructureFailure::internal(
                CompilerStage::SourceLoading,
                format!(
                    "source identity {} is absent from the source database",
                    source.index()
                ),
            )
        })?;
        Ok(match slot.load {
            SourceLoadStatus::Pending => matches!(
                self.pending_builders.get(&source),
                Some(PendingSpanBuilder::Finished(_))
            ),
            SourceLoadStatus::Failed(_) | SourceLoadStatus::NonUtf8 => false,
            SourceLoadStatus::Loaded(index) => matches!(
                &self.live_builders[index.index()],
                SpanBuilderState::Finished(_) | SpanBuilderState::Installed
            ),
        })
    }

    /// Freeze one source's builder at the last producer boundary, keeping the table owner-local
    /// until final container assembly.
    pub(crate) fn finish_source(&mut self, source: SourceId) -> Result<(), InfrastructureFailure> {
        if source == SourceId::COMPILATION_ROOT {
            return Err(InfrastructureFailure::internal(
                CompilerStage::SourceLoading,
                format!(
                    "source identity {} cannot be finalized before its source was loaded",
                    source.index()
                ),
            ));
        }
        let slot = self.sources.get(source).ok_or_else(|| {
            InfrastructureFailure::internal(
                CompilerStage::SourceLoading,
                format!(
                    "source identity {} is absent from the source database",
                    source.index()
                ),
            )
        })?;
        match slot.load {
            SourceLoadStatus::Pending => match self.pending_builders.remove(&source) {
                Some(PendingSpanBuilder::Live(builder)) => {
                    self.pending_builders
                        .insert(source, PendingSpanBuilder::Finished(builder.freeze()));
                }
                Some(PendingSpanBuilder::CheckedOut) => {
                    self.pending_builders
                        .insert(source, PendingSpanBuilder::CheckedOut);
                    return Err(InfrastructureFailure::internal(
                        CompilerStage::SourceLoading,
                        format!(
                            "source identity {} still has an outstanding span producer",
                            source.index()
                        ),
                    ));
                }
                Some(PendingSpanBuilder::Finished(table)) => {
                    self.pending_builders
                        .insert(source, PendingSpanBuilder::Finished(table));
                    return Err(InfrastructureFailure::internal(
                        CompilerStage::SourceLoading,
                        format!(
                            "source identity {} was finalized more than once",
                            source.index()
                        ),
                    ));
                }
                None => {
                    self.pending_builders.insert(
                        source,
                        PendingSpanBuilder::Finished(ExtendedSpanBuilder::new().freeze()),
                    );
                }
            },
            SourceLoadStatus::Failed(_) | SourceLoadStatus::NonUtf8 => {
                return Err(InfrastructureFailure::internal(
                    CompilerStage::SourceLoading,
                    format!(
                        "source identity {} cannot be finalized after its source failed to load",
                        source.index()
                    ),
                ));
            }
            SourceLoadStatus::Loaded(loaded_index) => {
                let index = loaded_index.index();
                match &self.live_builders[index] {
                    SpanBuilderState::Installed | SpanBuilderState::Finished(_) => {
                        return Err(InfrastructureFailure::internal(
                            CompilerStage::SourceLoading,
                            format!(
                                "source identity {} was finalized more than once",
                                source.index()
                            ),
                        ));
                    }
                    SpanBuilderState::CheckedOut => {
                        return Err(InfrastructureFailure::internal(
                            CompilerStage::SourceLoading,
                            format!(
                                "source identity {} still has an outstanding span producer",
                                source.index()
                            ),
                        ));
                    }
                    SpanBuilderState::Unprepared | SpanBuilderState::Live(_) => {}
                }
                let state =
                    std::mem::replace(&mut self.live_builders[index], SpanBuilderState::Unprepared);
                let builder = match state {
                    SpanBuilderState::Unprepared => ExtendedSpanBuilder::new(),
                    SpanBuilderState::Live(builder) => builder,
                    _ => crate::compiler_bug!(
                        CompilerStage::SourceLoading,
                        "span-builder state was checked before source finalization"
                    ),
                };
                self.live_builders[index] = SpanBuilderState::Finished(builder.freeze());
            }
        }
        Ok(())
    }

    pub(crate) fn take_span_builder(&mut self, source: SourceId) -> ExtendedSpanBuilder {
        let slot = self
            .sources
            .get(source)
            .expect("span producer source must be registered; this is a compiler bug");
        match slot.load {
            SourceLoadStatus::Pending => match self.pending_builders.remove(&source) {
                Some(PendingSpanBuilder::Live(builder)) => {
                    self.pending_builders
                        .insert(source, PendingSpanBuilder::CheckedOut);
                    builder
                }
                Some(PendingSpanBuilder::CheckedOut) => crate::compiler_bug!(
                    CompilerStage::SourceLoading,
                    "source {} already has an outstanding span producer; this is a compiler bug",
                    source.index()
                ),
                Some(PendingSpanBuilder::Finished(table)) => {
                    self.pending_builders
                        .insert(source, PendingSpanBuilder::Finished(table));
                    crate::compiler_bug!(
                        CompilerStage::SourceLoading,
                        "source {} was already finalized and cannot take another span builder; this is a compiler bug",
                        source.index()
                    )
                }
                None => {
                    self.pending_builders
                        .insert(source, PendingSpanBuilder::CheckedOut);
                    ExtendedSpanBuilder::new()
                }
            },
            SourceLoadStatus::Failed(_) | SourceLoadStatus::NonUtf8 => crate::compiler_bug!(
                CompilerStage::SourceLoading,
                "span producer source {} has a recorded load failure; this is a compiler bug",
                source.index()
            ),
            SourceLoadStatus::Loaded(index) => {
                match &self.live_builders[index.index()] {
                    SpanBuilderState::Installed | SpanBuilderState::Finished(_) => {
                        crate::compiler_bug!(
                            CompilerStage::SourceLoading,
                            "source {} was already finalized and cannot take another span builder; this is a compiler bug",
                            source.index()
                        )
                    }
                    SpanBuilderState::CheckedOut => crate::compiler_bug!(
                        CompilerStage::SourceLoading,
                        "source {} already has an outstanding span producer; this is a compiler bug",
                        source.index()
                    ),
                    SpanBuilderState::Unprepared | SpanBuilderState::Live(_) => {}
                }
                match std::mem::replace(
                    &mut self.live_builders[index.index()],
                    SpanBuilderState::CheckedOut,
                ) {
                    SpanBuilderState::Unprepared => ExtendedSpanBuilder::new(),
                    SpanBuilderState::Live(builder) => builder,
                    _ => crate::compiler_bug!(
                        CompilerStage::SourceLoading,
                        "span-builder state was checked before it was taken"
                    ),
                }
            }
        }
    }

    /// Accept a producer's returned builder or adopt a discovery-prepared source's original.
    pub(crate) fn retain_span_builder(&mut self, source: SourceId, builder: ExtendedSpanBuilder) {
        let slot = self
            .sources
            .get(source)
            .expect("span producer source must be registered; this is a compiler bug");
        match slot.load {
            SourceLoadStatus::Pending => match self.pending_builders.entry(source) {
                std::collections::hash_map::Entry::Vacant(entry) => {
                    entry.insert(PendingSpanBuilder::Live(builder));
                }
                std::collections::hash_map::Entry::Occupied(mut entry) => {
                    if matches!(entry.get(), PendingSpanBuilder::Finished(_)) {
                        crate::compiler_bug!(
                            CompilerStage::SourceLoading,
                            "source {} was already finalized and cannot retain another span builder; this is a compiler bug",
                            source.index()
                        );
                    }
                    if matches!(entry.get(), PendingSpanBuilder::Live(_)) {
                        crate::compiler_bug!(
                            CompilerStage::SourceLoading,
                            "source {} already owns a pending span builder; this is a compiler bug",
                            source.index()
                        );
                    }
                    *entry.get_mut() = PendingSpanBuilder::Live(builder);
                }
            },
            SourceLoadStatus::Failed(_) | SourceLoadStatus::NonUtf8 => crate::compiler_bug!(
                CompilerStage::SourceLoading,
                "span producer source {} has a recorded load failure; this is a compiler bug",
                source.index()
            ),
            SourceLoadStatus::Loaded(index) => {
                let live = &mut self.live_builders[index.index()];
                match live {
                    SpanBuilderState::Installed | SpanBuilderState::Finished(_) => {
                        crate::compiler_bug!(
                            CompilerStage::SourceLoading,
                            "source {} was already finalized and cannot retain another span builder; this is a compiler bug",
                            source.index()
                        )
                    }
                    SpanBuilderState::Live(_) => crate::compiler_bug!(
                        CompilerStage::SourceLoading,
                        "source {} already owns a live span builder; this is a compiler bug",
                        source.index()
                    ),
                    SpanBuilderState::Unprepared | SpanBuilderState::CheckedOut => {
                        *live = SpanBuilderState::Live(builder);
                    }
                }
            }
        }
    }
}

/// Derive a physical source kind from a canonical path extension, for [`SourceDatabase::build`].
///
/// Every production lane supplies an authored kind through the registration index,
/// [`SourceDatabase::insert`] or a Stage 0 constructor instead, so this derivation only
/// serves test fixtures. Recognized extensions become compiler kinds; every other physical path is
/// provider-owned, so `None` on a slot can only mean the reserved compilation root.
#[cfg(test)]
fn physical_source_kind(canonical_path: &Path) -> SourceKind {
    let extension = canonical_path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("");
    match SourceFileKind::from_extension(extension) {
        Some(kind) => SourceKind::Compiler(kind),
        None => SourceKind::ProviderOwned,
    }
}

/// Temporary registration spelling, retained only until canonical ordering and interning finish.
struct LogicalSourcePath {
    path: PathBuf,
    portable_sort_key: String,
}

fn logical_rows_for_registration_index(
    registration_index: &SourceRegistrationIndex<'_>,
    entry_file_path: &Path,
    project_path_resolver: Option<&ProjectPathResolver>,
) -> Result<Vec<(PathBuf, SourceKind, LogicalSourcePath)>, InfrastructureFailure> {
    let rows_iter = registration_index.rows();
    let mut rows = Vec::with_capacity(rows_iter.len());
    for (canonical, kind) in rows_iter {
        let logical = logical_source_path(canonical, entry_file_path, project_path_resolver)?;
        rows.push((canonical.to_path_buf(), kind, logical));
    }

    Ok(rows)
}

/// Resolve a canonical file's logical spelling before assigning compact identities.
fn logical_source_path(
    canonical_file: &Path,
    entry_file_path: &Path,
    project_path_resolver: Option<&ProjectPathResolver>,
) -> Result<LogicalSourcePath, InfrastructureFailure> {
    let logical = match project_path_resolver {
        Some(resolver) => resolver.logical_path_for_canonical_file(canonical_file)?,
        None => {
            let fallback_root = entry_file_path
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| PathBuf::from("."));
            logical_path_for_single_file_mode(canonical_file, &fallback_root)
        }
    };

    let portable_sort_key = logical
        .to_str()
        .ok_or_else(|| non_utf8_logical_path_error(&logical))?
        .replace('\\', "/");

    Ok(LogicalSourcePath {
        path: logical,
        portable_sort_key,
    })
}

/// Typed source-capacity failure from one compact source-owned identity table.
///
/// Authored exhaustion is deterministic user-facing rejection: the project supplied more
/// logical-path nodes or source files than the four-byte identity table can address. The
/// carrying boundary converts the resource into the typed capacity diagnostic batch; every
/// other database failure stays an infrastructure [`InfrastructureFailure`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SourceCapacityError {
    /// The build-lifetime logical-path node table cannot address another entry.
    LogicalPathTableFull,
    /// The source-slot identity table cannot address another registered source.
    SourceIdentityTableFull,
    /// An authored physical snapshot cannot be addressed by u32 byte offsets.
    SourceSnapshotTooLarge,
}

impl SourceCapacityError {
    pub(crate) fn resource(
        self,
    ) -> crate::compiler_frontend::compiler_messages::SourceSpanCapacityResource {
        match self {
            Self::LogicalPathTableFull => {
                crate::compiler_frontend::compiler_messages::SourceSpanCapacityResource::LogicalPath
            }
            Self::SourceIdentityTableFull => {
                crate::compiler_frontend::compiler_messages::SourceSpanCapacityResource::SourceIdentity
            }
            Self::SourceSnapshotTooLarge => {
                crate::compiler_frontend::compiler_messages::SourceSpanCapacityResource::SourceSnapshot
            }
        }
    }
}

/// One source-database construction failure.
///
/// WHAT: separates authored compact-table exhaustion from every other registration,
///       load and lifecycle failure.
/// WHY: capacity rejection must reach the typed user diagnostic batch unchanged, while
///      remaining failures stay on the infrastructure `InfrastructureFailure` lane. Wrapping both
///      in one local sum keeps the three fallible construction APIs honest without
///      introducing a `source -> compiler_messages` module cycle.
#[derive(Debug, Clone)]
pub(crate) enum SourceDatabaseError {
    Capacity(SourceCapacityError),
    Infrastructure(InfrastructureFailure),
}

impl From<InfrastructureFailure> for SourceDatabaseError {
    fn from(error: InfrastructureFailure) -> Self {
        Self::Infrastructure(error)
    }
}

fn non_utf8_logical_path_error(logical_path: &Path) -> InfrastructureFailure {
    InfrastructureFailure::filesystem(
        crate::compiler_frontend::compiler_messages::CompilerStage::SourceLoading,
        logical_path,
        format!(
            "Source file logical path {logical_path:?} contains a non-UTF-8 component; Moth identity requires UTF-8 paths."
        ),
    )
}

/// Translate one path-interning outcome into the owning database's failure lane.
///
/// Authored exhaustion of a compact identity table is a deterministic source failure: the
/// project supplied more entries than the four-byte table can address, so compilation aborts
/// with the typed capacity failure instead of wrapping an index or panicking. Strict UTF-8
/// violations keep their existing infrastructure file error.
fn map_path_intern_error(error: PathInternError) -> SourceDatabaseError {
    match error {
        PathInternError::NonUtf8(NonUtf8PathComponent { path }) => {
            SourceDatabaseError::Infrastructure(non_utf8_logical_path_error(&path))
        }
        PathInternError::TableFull => {
            SourceDatabaseError::Capacity(SourceCapacityError::LogicalPathTableFull)
        }
        PathInternError::BaseMismatch { .. } => {
            SourceDatabaseError::Infrastructure(InfrastructureFailure::internal(
                CompilerStage::SourceLoading,
                "logical path merge base is not a structural prefix of the destination table",
            ))
        }
    }
}

fn logical_path_for_single_file_mode(canonical_file: &Path, source_root: &Path) -> PathBuf {
    if let Ok(relative) = canonical_file.strip_prefix(source_root) {
        return relative.to_path_buf();
    }

    canonical_file
        .file_name()
        .map(PathBuf::from)
        .unwrap_or_else(|| canonical_file.to_path_buf())
}
