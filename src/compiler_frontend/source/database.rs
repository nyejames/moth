//! Database of source identities and retained snapshots for one frontend compilation lifetime.
//!
//! The database owns source-record identity, path metadata and the exact UTF-8 source snapshot
//! used for compilation. Source line indexes and spans remain outside this slice and are
//! deliberately not stored here.

use super::record::SourceRecordState;
use super::{SourceId, SourceProvenance, SourceRecord, SourceRegistrationIndex};
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

    /// Build deterministic source identities from an unordered file list.
    ///
    /// Canonical files are sorted by portable logical path before identities are assigned, so
    /// assignment does not depend on filesystem iteration order. Directory boundaries that
    /// already own sorted registration rows use [`Self::from_ordered_registration_index`] instead.
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
        let mut rows = logical_rows_for_canonical_files(
            canonical_files,
            entry_file_path,
            project_path_resolver,
            string_table,
        )?;
        rows.sort_by(|(_, left), (_, right)| left.portable_sort_key.cmp(&right.portable_sort_key));
        Self::from_ordered_logical_rows(rows)
    }

    /// Build source identities from the ordered candidates produced by Stage 0.
    ///
    /// The registration index already carries the canonical logical-identity order. This method
    /// computes each record's compiler-facing logical path but preserves row order, so the
    /// compiler assigns IDs deterministically.
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
    /// A caller may already have registered a bootstrap source, such as `config.moth`, before
    /// discovery supplies the remaining rows. Appending here preserves those earlier identities
    /// and assigns every candidate the next deterministic ID.
    pub(crate) fn append_ordered_registration_index(
        &mut self,
        registration_index: &SourceRegistrationIndex<'_>,
        entry_file_path: &Path,
        project_path_resolver: Option<&ProjectPathResolver>,
        string_table: &mut StringTable,
    ) -> Result<(), CompilerError> {
        let rows = logical_rows_for_canonical_files(
            registration_index.canonical_paths(),
            entry_file_path,
            project_path_resolver,
            string_table,
        )?;
        self.append_ordered_logical_rows(rows)
    }

    fn append_ordered_logical_rows<I>(&mut self, rows: I) -> Result<(), CompilerError>
    where
        I: IntoIterator<Item = (PathBuf, LogicalSourcePath)>,
    {
        for (canonical, logical) in rows {
            if self.canonical_to_id.contains_key(&canonical) {
                return Err(CompilerError::compiler_error(format!(
                    "Source identity inventory registered canonical source path {} more than once",
                    canonical.display(),
                )));
            }

            self.push_record(canonical, logical.interned);
        }

        Ok(())
    }

    fn from_ordered_logical_rows<I>(rows: I) -> Result<Self, CompilerError>
    where
        I: IntoIterator<Item = (PathBuf, LogicalSourcePath)>,
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
        self.get(id)?.state.retained_text()
    }

    /// Return the structured error recorded when loading a source snapshot failed.
    pub(crate) fn source_load_error(&self, id: SourceId) -> Option<&CompilerError> {
        self.get(id)?.state.source_load_error()
    }

    /// Move one loaded source snapshot into its preassigned record.
    pub(crate) fn retain_text(&mut self, id: SourceId, text: String) -> Result<(), CompilerError> {
        let record = self.source_record_mut(id)?;
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
    /// A repeated canonical path returns the existing identity only when its logical path matches.
    /// A conflicting logical spelling is rejected. New records append after the existing records,
    /// matching the traversal-time registration behavior.
    pub fn insert(
        &mut self,
        canonical_path: PathBuf,
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
            return Ok(record.id);
        }

        Ok(self.push_record(canonical_path, logical.interned))
    }

    /// Append one record and return the identity its position assigns.
    fn push_record(&mut self, canonical_path: PathBuf, logical_path: InternedPath) -> SourceId {
        let id = SourceId::from_index(self.files.len());
        self.canonical_to_id.insert(canonical_path.clone(), id);
        self.files.push(SourceRecord {
            id,
            canonical_os_path: Some(canonical_path),
            logical_path,
            state: SourceRecordState::Registered,
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

    /// Resolve a retained source snapshot by logical path.
    ///
    /// This is deliberately a cold-path linear scan: renderers perform it only while producing a
    /// diagnostic frame, and keeping the source database's compact identity storage free of a
    /// second logical-path index avoids another allocation and synchronization boundary.
    ///
    /// A logical path is safe to render only when it identifies exactly one record in this
    /// database. Collisions can arise when independently rooted sources share a portable spelling;
    /// returning no snapshot on ambiguity is safer than guessing and displaying another file's
    /// text.
    pub(crate) fn retained_text_for_logical_path(
        &self,
        logical_path: &InternedPath,
    ) -> Option<&str> {
        let mut matches = self
            .iter()
            .filter(|record| record.logical_path == *logical_path);
        let record = matches.next()?;
        if matches.next().is_some() {
            return None;
        }
        record.state.retained_text()
    }
}

fn compilation_root_record() -> SourceRecord {
    SourceRecord {
        id: SourceId::from_index(0),
        canonical_os_path: None,
        logical_path: InternedPath::new(),
        state: SourceRecordState::Registered,
        provenance: SourceProvenance::CompilationRoot,
    }
}

/// One source's logical path in both the interned form records keep and the portable spelling
/// unsorted inventories order by.
struct LogicalSourcePath {
    interned: InternedPath,
    portable_sort_key: String,
}

fn logical_rows_for_canonical_files<I>(
    canonical_files: I,
    entry_file_path: &Path,
    project_path_resolver: Option<&ProjectPathResolver>,
    string_table: &mut StringTable,
) -> Result<Vec<(PathBuf, LogicalSourcePath)>, CompilerError>
where
    I: IntoIterator,
    I::IntoIter: ExactSizeIterator,
    I::Item: AsRef<Path>,
{
    let canonical_files = canonical_files.into_iter();
    let mut rows = Vec::with_capacity(canonical_files.len());

    for canonical in canonical_files {
        let canonical = canonical.as_ref();
        let logical = interned_logical_path(
            canonical,
            entry_file_path,
            project_path_resolver,
            string_table,
        )?;
        rows.push((canonical.to_path_buf(), logical));
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
