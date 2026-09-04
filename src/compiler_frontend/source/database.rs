//! Database of source identities retained for one frontend compilation lifetime.
//!
//! The database owns source-record identity and path metadata only. Source text, line indexes,
//! spans and build-lifetime registration remain outside this slice and are deliberately not stored
//! here.

use super::{SourceId, SourceProvenance, SourceRecord};
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
    /// assignment does not depend on filesystem iteration order. Callers that already hold a
    /// sorted boundary inventory use [`Self::from_ordered_canonical_files`] instead.
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

    /// Build source identities from an already sorted boundary inventory.
    ///
    /// Stage 0 owns filesystem discovery and sorts its canonical source inventory by portable
    /// logical identity before assigning its boundary-local handles. Directory and source-package
    /// compilation pass that order here so the compiler assigns its own `SourceId` values without
    /// re-walking or re-sorting the inventory.
    pub(crate) fn from_ordered_canonical_files<I>(
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
        let rows = logical_rows_for_canonical_files(
            canonical_files,
            entry_file_path,
            project_path_resolver,
            string_table,
        )?;
        Self::from_ordered_logical_rows(rows)
    }

    fn from_ordered_logical_rows<I>(rows: I) -> Result<Self, CompilerError>
    where
        I: IntoIterator<Item = (PathBuf, LogicalSourcePath)>,
    {
        let mut files = vec![compilation_root_record()];
        let mut canonical_to_id = FxHashMap::default();

        for (canonical, logical) in rows {
            if canonical_to_id.contains_key(&canonical) {
                return Err(CompilerError::compiler_error(format!(
                    "Source identity inventory registered canonical source path {} more than once",
                    canonical.display(),
                )));
            }

            let id = SourceId::from_index(files.len());
            canonical_to_id.insert(canonical.clone(), id);
            files.push(SourceRecord {
                id,
                canonical_os_path: Some(canonical),
                logical_path: logical.interned,
                provenance: SourceProvenance::AuthoredPhysical,
            });
        }

        Ok(Self {
            files,
            canonical_to_id,
        })
    }

    pub fn get_by_canonical_path(&self, canonical_path: &Path) -> Option<&SourceRecord> {
        let id = self.canonical_to_id.get(canonical_path)?;
        self.get(*id)
    }

    /// Register one canonical source file and return its source identity.
    ///
    /// A repeated canonical path returns the existing identity. New records append after the
    /// existing records, matching the traversal-time registration behavior.
    pub fn insert(
        &mut self,
        canonical_path: PathBuf,
        entry_file_path: &Path,
        project_path_resolver: Option<&ProjectPathResolver>,
        string_table: &mut StringTable,
    ) -> Result<SourceId, CompilerError> {
        if let Some(record) = self.get_by_canonical_path(&canonical_path) {
            return Ok(record.id);
        }

        let logical = interned_logical_path(
            &canonical_path,
            entry_file_path,
            project_path_resolver,
            string_table,
        )?;
        let logical_path = logical.interned;

        let id = SourceId::from_index(self.files.len());
        self.canonical_to_id.insert(canonical_path.clone(), id);
        self.files.push(SourceRecord {
            id,
            canonical_os_path: Some(canonical_path),
            logical_path,
            provenance: SourceProvenance::AuthoredPhysical,
        });
        Ok(id)
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

    /// Iterate physical source identities in deterministic logical-path order.
    ///
    /// The compilation-root record is addressed by `SourceId(1)` but is not a physical source and
    /// therefore does not appear in this iterator.
    pub fn iter(&self) -> std::slice::Iter<'_, SourceRecord> {
        debug_assert_eq!(
            self.files.first().map(|record| record.provenance),
            Some(SourceProvenance::CompilationRoot)
        );
        self.files[1..].iter()
    }
}

fn compilation_root_record() -> SourceRecord {
    SourceRecord {
        id: SourceId::from_index(0),
        canonical_os_path: None,
        logical_path: InternedPath::new(),
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
