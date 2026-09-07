//! Database of registered source slots, loaded snapshots, and cold load failures for one frontend
//! compilation lifetime.
//!
//! The database owns the ordered source-slot inventory, the dense array of loaded snapshots, and
//! the cold array of load failures. Loaded records retain the exact UTF-8 source text plus
//! line-start table for each successful load. Their extended-span table is installed once, after
//! the final span-producing stage, and remains source-owned for frozen consumers.

use super::line_index::LineIndex;
use super::record::{
    LoadFailureIndex, LoadedSourceIndex, SourceLoadStatus, SourceSlot, ensure_source_snapshot_fits,
};
use super::span::ExtendedSpanTable;
use super::{SourceId, SourceKind, SourceProvenance, SourceRecord, SourceRegistrationIndex};
#[cfg(test)]
use crate::builder_surface::SourceFileKind;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::paths::path_resolution::ProjectPathResolver;
use crate::compiler_frontend::symbols::interned_path::{InternedPath, NonUtf8PathComponent};
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerBuilder, PathTable};
use crate::compiler_frontend::symbols::string_interning::StringTable;

use rustc_hash::FxHashMap;

use std::path::{Path, PathBuf};

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
    load_failures: Vec<CompilerError>,
    canonical_to_id: FxHashMap<PathBuf, SourceId>,
    path_interner: PathInternerBuilder,
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
    ) -> Result<Self, CompilerError>
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
    pub(crate) fn from_registration_index_sorted_by_logical_path(
        registration_index: &SourceRegistrationIndex<'_>,
        entry_file_path: &Path,
        project_path_resolver: Option<&ProjectPathResolver>,
        string_table: &mut StringTable,
    ) -> Result<Self, CompilerError> {
        let mut database = Self::empty();
        let mut rows = logical_rows_for_registration_index(
            registration_index,
            entry_file_path,
            project_path_resolver,
            string_table,
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
    ) -> Result<Self, CompilerError> {
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
    ) -> Result<(), CompilerError> {
        let rows = logical_rows_for_registration_index(
            registration_index,
            entry_file_path,
            project_path_resolver,
            string_table,
        )?;
        self.append_ordered_logical_rows(rows, string_table)
    }

    fn append_ordered_logical_rows(
        &mut self,
        rows: Vec<(PathBuf, SourceKind, LogicalSourcePath)>,
        string_table: &mut StringTable,
    ) -> Result<(), CompilerError> {
        // Canonical ordering precedes both path interning and source identity assignment.
        let interned_rows = rows
            .into_iter()
            .map(|(canonical, kind, logical)| {
                let path_id = self
                    .path_interner
                    .try_intern_filesystem_path(&logical.path, string_table)
                    .map_err(|NonUtf8PathComponent { path }| {
                        non_utf8_logical_path_error(&path, string_table)
                    })?;
                Ok((canonical, kind, path_id))
            })
            .collect::<Result<Vec<_>, CompilerError>>()?;

        for (canonical, kind, path_id) in interned_rows {
            if self.canonical_to_id.contains_key(&canonical) {
                return Err(CompilerError::compiler_error(format!(
                    "Source identity inventory registered canonical source path {} more than once",
                    canonical.display(),
                )));
            }

            self.push_slot(canonical, path_id, kind);
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

    /// Reconstruct the legacy path view for a registered source identity.
    ///
    /// This is a temporary migration bridge. The source slot stores only its `PathId`, so the
    /// component vector is rebuilt from parent links and is never retained by the database.
    pub(crate) fn legacy_logical_path(&self, source: SourceId) -> InternedPath {
        let path_id = self
            .slots
            .get(source.index())
            .unwrap_or_else(|| {
                panic!(
                    "source identity {} is absent from the source database; this is a compiler bug",
                    source.index()
                )
            })
            .logical_path;
        let table = self.paths();
        let mut components = Vec::with_capacity(table.depth(path_id) as usize);
        table.resolve_components(path_id, &mut components);
        InternedPath::from_components(components)
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
    pub(crate) fn line_index(&self, id: SourceId) -> Option<LineIndex<'_>> {
        let record = self.loaded_record(id)?;
        Some(LineIndex::new(&record.text, &record.line_starts))
    }

    pub(crate) fn source_load_error(&self, id: SourceId) -> Option<&CompilerError> {
        match &self.get(id)?.load {
            SourceLoadStatus::Failed(index) => self.load_failures.get(index.index()),
            SourceLoadStatus::Pending | SourceLoadStatus::Loaded(_) => None,
        }
    }

    /// Move one loaded source snapshot into a record owned by its preassigned slot.
    ///
    /// A slot that already holds a snapshot or a load failure is a lifecycle violation, so the
    /// size policy is applied only to a slot that can still accept one. Otherwise an oversized
    /// second snapshot would report the user's file instead of the compiler's own mistake.
    pub(crate) fn retain_text(&mut self, id: SourceId, text: String) -> Result<(), CompilerError> {
        let provenance = {
            let slot = self.source_slot_mut(id)?;
            if !matches!(slot.load, SourceLoadStatus::Pending) {
                return Err(CompilerError::compiler_error(format!(
                    "source text for source identity {} was retained more than once",
                    id.index()
                )));
            }
            slot.provenance
        };

        // Construct the legacy path only on the rare oversized-snapshot error path. Normal loads
        // carry the compact PathId without allocating a component vector.
        if text.len() >= u32::MAX as usize {
            let slot = self
                .get(id)
                .expect("source slot validated before retaining text");
            let logical_path = self.legacy_logical_path(id);
            ensure_source_snapshot_fits(
                text.len(),
                provenance,
                &logical_path,
                slot.canonical_os_path.as_deref(),
            )?;
        }

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
    // The exclusive source owner installs the table after its final span-producing stage.
    pub(crate) fn install_extended_spans(
        &mut self,
        id: SourceId,
        extended_spans: ExtendedSpanTable,
    ) -> Result<(), CompilerError> {
        let loaded_index = {
            let slot = self.slots.get(id.index()).ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "source identity {} is absent from the source database",
                    id.index()
                ))
            })?;

            if slot.provenance == SourceProvenance::CompilationRoot {
                return Err(CompilerError::compiler_error(format!(
                    "extended spans cannot be installed on compilation root source identity {}",
                    id.index()
                )));
            }

            match slot.load {
                SourceLoadStatus::Loaded(index) => index,
                SourceLoadStatus::Pending | SourceLoadStatus::Failed(_) => {
                    return Err(CompilerError::compiler_error(format!(
                        "extended spans for source identity {} were installed before its source \
                         was loaded",
                        id.index()
                    )));
                }
            }
        };

        let record = self
            .loaded
            .get_mut(loaded_index.index())
            .expect("loaded source status points outside the loaded record array");

        if record.extended_spans.is_some() {
            return Err(CompilerError::compiler_error(format!(
                "extended spans for source identity {} were installed more than once",
                id.index()
            )));
        }

        record.extended_spans = Some(extended_spans);
        Ok(())
    }

    /// Record a source-read failure in its preassigned slot without aborting the whole build.
    pub(crate) fn record_source_load_error(
        &mut self,
        id: SourceId,
        error: CompilerError,
    ) -> Result<(), CompilerError> {
        {
            let slot = self.source_slot_mut(id)?;
            if !matches!(slot.load, SourceLoadStatus::Pending) {
                return Err(CompilerError::compiler_error(format!(
                    "source load status for source identity {} was recorded more than once",
                    id.index()
                )));
            }
        }

        let failure_index = LoadFailureIndex::from_index(self.load_failures.len());
        self.load_failures.push(error);

        let slot = self
            .slots
            .get_mut(id.index())
            .expect("source slot validated before recording load error");
        debug_assert!(matches!(slot.load, SourceLoadStatus::Pending));
        slot.load = SourceLoadStatus::Failed(failure_index);
        Ok(())
    }

    fn source_slot_mut(&mut self, id: SourceId) -> Result<&mut SourceSlot, CompilerError> {
        let slot = self.slots.get_mut(id.index()).ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "source identity {} is absent from the source database",
                id.index()
            ))
        })?;
        if slot.provenance == SourceProvenance::CompilationRoot {
            return Err(CompilerError::compiler_error(
                "source snapshots cannot be retained on the compilation root",
            ));
        }
        Ok(slot)
    }

    fn loaded_record(&self, id: SourceId) -> Option<&SourceRecord> {
        let slot = self.get(id)?;
        let loaded_index = match slot.load {
            SourceLoadStatus::Loaded(index) => index,
            SourceLoadStatus::Pending | SourceLoadStatus::Failed(_) => return None,
        };
        self.loaded.get(loaded_index.index())
    }

    /// Resolve the loaded record addressed by a source identity for frozen span consumers.
    ///
    /// Missing, pending, failed and reserved-root identities are compiler bugs at this boundary.
    pub(super) fn source_record(&self, id: SourceId) -> &SourceRecord {
        let slot = self.slots.get(id.index()).unwrap_or_else(|| {
            panic!(
                "source identity {} is absent from the source database; this is a compiler bug",
                id.index()
            )
        });

        if slot.provenance == SourceProvenance::CompilationRoot {
            panic!(
                "source identity {} is the compilation root and has no loaded source record; \
                 this is a compiler bug",
                id.index()
            );
        }

        let loaded_index = match slot.load {
            SourceLoadStatus::Loaded(index) => index,
            SourceLoadStatus::Pending | SourceLoadStatus::Failed(_) => {
                panic!(
                    "source identity {} has no loaded source record; this is a compiler bug",
                    id.index()
                )
            }
        };

        self.loaded.get(loaded_index.index()).unwrap_or_else(|| {
            panic!(
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
    ) -> Result<SourceId, CompilerError> {
        let logical = logical_source_path(
            &canonical_path,
            entry_file_path,
            project_path_resolver,
            string_table,
        )?;
        let path_id = self
            .path_interner
            .try_intern_filesystem_path(&logical.path, string_table)
            .map_err(|NonUtf8PathComponent { path }| {
                non_utf8_logical_path_error(&path, string_table)
            })?;

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
                return Err(CompilerError::compiler_error(format!(
                    "Source identity inventory registered canonical source path {} under \
                     conflicting logical paths {} and {}",
                    canonical_path.display(),
                    existing_path,
                    requested_path,
                )));
            }
            if let Some(stored_kind) = slot.kind
                && stored_kind != kind
            {
                return Err(CompilerError::compiler_error(format!(
                    "Source identity inventory registered canonical source path {} under \
                     conflicting kinds {:?} and {:?}",
                    canonical_path.display(),
                    stored_kind,
                    kind,
                )));
            }
            return Ok(slot.id);
        }
        Ok(self.push_slot(canonical_path, path_id, kind))
    }

    /// Append one slot and return the identity its position assigns.
    fn push_slot(
        &mut self,
        canonical_path: PathBuf,
        logical_path: PathId,
        kind: SourceKind,
    ) -> SourceId {
        let id = SourceId::from_index(self.slots.len());
        self.canonical_to_id.insert(canonical_path.clone(), id);
        self.slots.push(SourceSlot {
            id,
            canonical_os_path: Some(canonical_path),
            logical_path,
            kind: Some(kind),
            provenance: SourceProvenance::AuthoredPhysical,
            load: SourceLoadStatus::Pending,
        });
        id
    }

    /// Resolve one physical source slot.
    ///
    /// The compilation root is addressed by [`SourceId::COMPILATION_ROOT`] but is not a physical
    /// source, so it is never returned here. A consumer that reaches this with the root holds an
    /// identity from the wrong domain, and absence lets it fail in its own lane rather than
    /// reading a pathless slot as though it were a file.
    pub fn get(&self, id: SourceId) -> Option<&SourceSlot> {
        let slot = self.slots.get(id.index())?;
        (slot.provenance != SourceProvenance::CompilationRoot).then_some(slot)
    }

    /// Resolve the unique physical slot for a logical path, if one exists.
    ///
    /// This is deliberately a cold-path linear scan: renderers perform it only while producing a
    /// diagnostic frame, and keeping the source database's compact identity storage free of a
    /// second logical-path index avoids another allocation and synchronization boundary.
    ///
    /// A logical path is safe to render only when it identifies exactly one slot in this
    /// database. Collisions can arise when independently rooted sources share a portable spelling;
    /// returning no slot on ambiguity is safer than guessing and displaying another file's text.
    pub(crate) fn unique_record_for_logical_path(
        &self,
        logical_path: &InternedPath,
    ) -> Option<&SourceSlot> {
        let table = self.paths();
        let mut matches = self
            .iter()
            .filter(|slot| path_id_matches_components(table, slot.logical_path, logical_path));
        let slot = matches.next()?;
        if matches.next().is_some() {
            return None;
        }
        Some(slot)
    }
}

fn path_id_matches_components(
    table: &PathTable,
    path_id: PathId,
    components: &InternedPath,
) -> bool {
    let components = components.as_components();
    if table.depth(path_id) as usize != components.len() {
        return false;
    }

    let mut current = path_id;
    for expected_component in components.iter().rev() {
        if table.component(current) != Some(*expected_component) {
            return false;
        }
        current = table
            .parent(current)
            .expect("a non-root path must carry a parent");
    }

    current == PathId::ROOT
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
    string_table: &mut StringTable,
) -> Result<Vec<(PathBuf, SourceKind, LogicalSourcePath)>, CompilerError> {
    let rows_iter = registration_index.rows();
    let mut rows = Vec::with_capacity(rows_iter.len());

    for (canonical, kind) in rows_iter {
        let logical = logical_source_path(
            canonical,
            entry_file_path,
            project_path_resolver,
            string_table,
        )?;
        rows.push((canonical.to_path_buf(), kind, logical));
    }

    Ok(rows)
}

/// Resolve a canonical file's logical spelling before assigning compact identities.
fn logical_source_path(
    canonical_file: &Path,
    entry_file_path: &Path,
    project_path_resolver: Option<&ProjectPathResolver>,
    string_table: &mut StringTable,
) -> Result<LogicalSourcePath, CompilerError> {
    let logical = match project_path_resolver {
        Some(resolver) => resolver.logical_path_for_canonical_file(canonical_file, string_table)?,
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
        .ok_or_else(|| non_utf8_logical_path_error(&logical, string_table))?
        .replace('\\', "/");

    Ok(LogicalSourcePath {
        path: logical,
        portable_sort_key,
    })
}

fn non_utf8_logical_path_error(
    logical_path: &Path,
    string_table: &mut StringTable,
) -> CompilerError {
    CompilerError::file_error(
        logical_path,
        format!(
            "Source file logical path {logical_path:?} contains a non-UTF-8 component; Moth identity requires UTF-8 paths."
        ),
        string_table,
    )
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
