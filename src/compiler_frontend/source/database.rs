//! Database of source identities and retained snapshots for one frontend compilation lifetime.
//!
//! The database owns source-record identity, path metadata, the exact UTF-8 source snapshot used
//! for compilation, and the line-start table built when that snapshot becomes owned. Source spans
//! remain outside this slice and are deliberately not stored here.

use super::record::{SourceRecordState, ensure_source_snapshot_fits};
use super::{SourceId, SourceKind, SourceProvenance, SourceRecord, SourceRegistrationIndex};
#[cfg(test)]
use crate::builder_surface::SourceFileKind;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::paths::path_resolution::ProjectPathResolver;
use crate::compiler_frontend::symbols::interned_path::{InternedPath, NonUtf8PathComponent};
use crate::compiler_frontend::symbols::string_interning::StringTable;

use rustc_hash::FxHashMap;

use std::path::{Path, PathBuf};

/// Source identity records in deterministic logical-path order.
///
/// The database is not `Clone`. One boundary registers it once and shares it as an `Arc`, so a
/// deep copy would silently duplicate identities that are meant to be unique for the build.
#[derive(Debug)]
pub struct SourceDatabase {
    files: Vec<SourceRecord>,
    canonical_to_id: FxHashMap<PathBuf, SourceId>,
}

impl Default for SourceDatabase {
    fn default() -> Self {
        Self {
            files: vec![compilation_root_record()],
            canonical_to_id: FxHashMap::default(),
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
        let mut rows = logical_rows_for_registration_index(
            registration_index,
            entry_file_path,
            project_path_resolver,
            string_table,
        )?;
        rows.sort_by(|(_, _, left), (_, _, right)| {
            left.portable_sort_key.cmp(&right.portable_sort_key)
        });
        Self::from_ordered_logical_rows(rows)
    }

    /// Build source identities from the ordered candidates produced by Stage 0.
    ///
    /// WHAT: preserves the registration index's already-sorted Stage 0 logical-identity order
    ///       while computing each record's compiler-facing logical path.
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
        self.append_ordered_logical_rows(rows)
    }

    fn append_ordered_logical_rows<I>(&mut self, rows: I) -> Result<(), CompilerError>
    where
        I: IntoIterator<Item = (PathBuf, SourceKind, LogicalSourcePath)>,
    {
        for (canonical, kind, logical) in rows {
            if self.canonical_to_id.contains_key(&canonical) {
                return Err(CompilerError::compiler_error(format!(
                    "Source identity inventory registered canonical source path {} more than once",
                    canonical.display(),
                )));
            }

            self.push_record(canonical, logical.interned, kind);
        }

        Ok(())
    }

    fn from_ordered_logical_rows<I>(rows: I) -> Result<Self, CompilerError>
    where
        I: IntoIterator<Item = (PathBuf, SourceKind, LogicalSourcePath)>,
    {
        let mut database = Self::empty();
        database.append_ordered_logical_rows(rows)?;
        Ok(database)
    }

    pub fn get_by_canonical_path(&self, canonical_path: &Path) -> Option<&SourceRecord> {
        let id = self.canonical_to_id.get(canonical_path)?;
        self.get(*id)
    }

    /// Look up the exact source snapshot retained for a physical source identity.
    ///
    /// The reserved compilation root is excluded by [`Self::get`], so it cannot accidentally
    /// become a physical source frame.
    pub fn retained_text(&self, id: SourceId) -> Option<&str> {
        self.get(id)?.retained_text()
    }

    /// Return the structured error recorded when loading a source snapshot failed.
    pub(crate) fn source_load_error(&self, id: SourceId) -> Option<&CompilerError> {
        self.get(id)?.state.source_load_error()
    }

    /// Move one loaded source snapshot into its preassigned record.
    ///
    /// A record that already holds a snapshot or a load failure is a lifecycle violation, so the
    /// size policy is applied only to a record that can still accept one. Otherwise an oversized
    /// second snapshot would report the user's file instead of the compiler's own mistake.
    pub(crate) fn retain_text(&mut self, id: SourceId, text: String) -> Result<(), CompilerError> {
        let record = self.source_record_mut(id)?;
        if record.state.is_registered() {
            ensure_source_snapshot_fits(
                text.len(),
                record.provenance,
                &record.logical_path,
                record.canonical_os_path.as_deref(),
            )?;
        }
        record.state.retain_text(id, text)
    }

    /// Record a source-read failure in its preassigned slot without aborting the whole build.
    pub(crate) fn record_source_load_error(
        &mut self,
        id: SourceId,
        error: CompilerError,
    ) -> Result<(), CompilerError> {
        let record = self.source_record_mut(id)?;
        record.state.record_load_error(id, error)
    }

    fn source_record_mut(&mut self, id: SourceId) -> Result<&mut SourceRecord, CompilerError> {
        let record = self.files.get_mut(id.index()).ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "source identity {} is absent from the source database",
                id.index()
            ))
        })?;
        if record.provenance == SourceProvenance::CompilationRoot {
            return Err(CompilerError::compiler_error(
                "source snapshots cannot be retained on the compilation root",
            ));
        }
        Ok(record)
    }

    /// Register one canonical source file and return its source identity.
    ///
    /// A repeated canonical path returns the existing identity only when its logical path and
    /// authored kind match. A conflicting logical spelling or kind is rejected. New records
    /// append after the existing records, matching the traversal-time registration behavior.
    ///
    /// Callers supply the authored kind their own lane compiles the source as. Kind is a property
    /// of that unique record: a second registration of the same canonical path with a different
    /// kind is a compiler invariant failure, because producers supply the kind rather than
    /// deriving it from the path. Path-only registration that holds no authored spelling uses
    /// [`Self::build`].
    pub fn insert(
        &mut self,
        canonical_path: PathBuf,
        kind: SourceKind,
        entry_file_path: &Path,
        project_path_resolver: Option<&ProjectPathResolver>,
        string_table: &mut StringTable,
    ) -> Result<SourceId, CompilerError> {
        let logical = interned_logical_path(
            &canonical_path,
            entry_file_path,
            project_path_resolver,
            string_table,
        )?;

        if let Some(record) = self.get_by_canonical_path(&canonical_path) {
            if record.logical_path != logical.interned {
                return Err(CompilerError::compiler_error(format!(
                    "Source identity inventory registered canonical source path {} under \
                     conflicting logical paths {} and {}",
                    canonical_path.display(),
                    record.logical_path.to_portable_string(string_table),
                    logical.interned.to_portable_string(string_table),
                )));
            }
            if let Some(stored_kind) = record.kind
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
            return Ok(record.id);
        }
        Ok(self.push_record(canonical_path, logical.interned, kind))
    }

    /// Append one record and return the identity its position assigns.
    fn push_record(
        &mut self,
        canonical_path: PathBuf,
        logical_path: InternedPath,
        kind: SourceKind,
    ) -> SourceId {
        let id = SourceId::from_index(self.files.len());
        self.canonical_to_id.insert(canonical_path.clone(), id);
        self.files.push(SourceRecord {
            id,
            canonical_os_path: Some(canonical_path),
            logical_path,
            state: SourceRecordState::Registered,
            kind: Some(kind),
            provenance: SourceProvenance::AuthoredPhysical,
        });
        id
    }

    /// Resolve one physical source record.
    ///
    /// The compilation root is addressed by `SourceId(1)` but is not a physical source, so it is
    /// never returned here. A consumer that reaches this with the root holds an identity from the
    /// wrong domain, and absence lets it fail in its own lane rather than reading a pathless
    /// record as though it were a file.
    pub fn get(&self, id: SourceId) -> Option<&SourceRecord> {
        let record = self.files.get(id.index())?;
        (record.provenance != SourceProvenance::CompilationRoot).then_some(record)
    }

    pub fn iter(&self) -> std::slice::Iter<'_, SourceRecord> {
        debug_assert_eq!(
            self.files.first().map(|record| record.provenance),
            Some(SourceProvenance::CompilationRoot)
        );
        self.files[1..].iter()
    }

    /// Resolve the unique physical record for a logical path, if one exists.
    ///
    /// This is deliberately a cold-path linear scan: renderers perform it only while producing a
    /// diagnostic frame, and keeping the source database's compact identity storage free of a
    /// second logical-path index avoids another allocation and synchronization boundary.
    ///
    /// A logical path is safe to render only when it identifies exactly one record in this
    /// database. Collisions can arise when independently rooted sources share a portable spelling;
    /// returning no record on ambiguity is safer than guessing and displaying another file's
    /// text.
    pub(crate) fn unique_record_for_logical_path(
        &self,
        logical_path: &InternedPath,
    ) -> Option<&SourceRecord> {
        let mut matches = self
            .iter()
            .filter(|record| record.logical_path == *logical_path);
        let record = matches.next()?;
        if matches.next().is_some() {
            return None;
        }
        Some(record)
    }
}

fn compilation_root_record() -> SourceRecord {
    SourceRecord {
        id: SourceId::from_index(0),
        canonical_os_path: None,
        logical_path: InternedPath::new(),
        state: SourceRecordState::Registered,
        kind: None,
        provenance: SourceProvenance::CompilationRoot,
    }
}

/// Derive a physical source kind from a canonical path extension, for [`SourceDatabase::build`].
///
/// Every production lane supplies an authored kind through the registration index,
/// [`SourceDatabase::insert`] or a Stage 0 constructor instead, so this derivation only
/// serves test fixtures. Recognized extensions become compiler kinds; every other physical path is
/// provider-owned, so `None` on a record can only mean the reserved compilation root.
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

/// One source's logical path in both the interned form records keep and the portable spelling
/// unsorted inventories order by.
struct LogicalSourcePath {
    interned: InternedPath,
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
        let logical = interned_logical_path(
            canonical,
            entry_file_path,
            project_path_resolver,
            string_table,
        )?;
        rows.push((canonical.to_path_buf(), kind, logical));
    }

    Ok(rows)
}

/// Resolve one canonical file's logical path and intern it.
///
/// Single-file mode has no project resolver, so it falls back to the entry file's directory.
fn interned_logical_path(
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
    let interned = InternedPath::try_from_filesystem_path(&logical, string_table).map_err(
        |NonUtf8PathComponent { path }| non_utf8_logical_path_error(&path, string_table),
    )?;

    Ok(LogicalSourcePath {
        interned,
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
