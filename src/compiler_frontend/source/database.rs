//! Database of source identities retained for one frontend compilation lifetime.
//!
//! The database owns source-record identity and path metadata only. Source text, line indexes,
//! spans and build-lifetime registration remain outside this slice and are deliberately not stored
//! here.

use super::{SourceId, SourceRecord};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::paths::path_resolution::ProjectPathResolver;
use crate::compiler_frontend::symbols::interned_path::{InternedPath, NonUtf8PathComponent};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use rustc_hash::FxHashMap;
use std::path::{Path, PathBuf};

/// Source identity records in deterministic logical-path order.
#[derive(Debug, Clone, Default)]
pub struct SourceDatabase {
    files: Vec<SourceRecord>,
    canonical_to_id: FxHashMap<PathBuf, SourceId>,
}

impl SourceDatabase {
    pub fn empty() -> Self {
        Self::default()
    }

    /// Build deterministic source identities for one module.
    ///
    /// Canonical files are sorted by portable logical path before identities are assigned, so
    /// assignment does not depend on filesystem iteration order.
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
        let fallback_root = entry_file_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));

        let canonical_files = canonical_files.into_iter();
        let mut rows = Vec::with_capacity(canonical_files.len());

        for canonical in canonical_files {
            let canonical = canonical.as_ref();
            let logical = if let Some(resolver) = project_path_resolver {
                resolver.logical_path_for_canonical_file(canonical, string_table)?
            } else {
                logical_path_for_single_file_mode(canonical, &fallback_root)
            };

            let portable_sort_key = logical
                .to_str()
                .ok_or_else(|| {
                    CompilerError::file_error(
                        &logical,
                        format!(
                            "Source file logical path {logical:?} contains a non-UTF-8 component; Moth identity requires UTF-8 paths."
                        ),
                        string_table,
                    )
                })?
                .replace('\\', "/");

            rows.push((canonical.to_path_buf(), logical, portable_sort_key));
        }

        rows.sort_by(|(_, _, left_key), (_, _, right_key)| left_key.cmp(right_key));

        let mut files = Vec::with_capacity(rows.len());
        let mut canonical_to_id = FxHashMap::default();

        for (index, (canonical, logical, _)) in rows.into_iter().enumerate() {
            let id = SourceId::from_index(index);
            let logical_path = InternedPath::try_from_filesystem_path(&logical, string_table)
                .map_err(|NonUtf8PathComponent { path }| {
                    CompilerError::file_error(
                        &path,
                        format!(
                            "Source file logical path {path:?} contains a non-UTF-8 component; Moth identity requires UTF-8 paths."
                        ),
                        string_table,
                    )
                })?;

            canonical_to_id.insert(canonical.clone(), id);
            files.push(SourceRecord {
                id,
                canonical_os_path: canonical,
                logical_path,
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

        let fallback_root = entry_file_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        let logical = if let Some(resolver) = project_path_resolver {
            resolver.logical_path_for_canonical_file(&canonical_path, string_table)?
        } else {
            logical_path_for_single_file_mode(&canonical_path, &fallback_root)
        };
        let logical_path = InternedPath::try_from_filesystem_path(&logical, string_table)
            .map_err(|NonUtf8PathComponent { path }| {
                CompilerError::file_error(
                    &path,
                    format!(
                        "Source file logical path {path:?} contains a non-UTF-8 component; Moth identity requires UTF-8 paths."
                    ),
                    string_table,
                )
            })?;

        let id = SourceId::from_index(self.files.len());
        self.canonical_to_id.insert(canonical_path.clone(), id);
        self.files.push(SourceRecord {
            id,
            canonical_os_path: canonical_path,
            logical_path,
        });
        Ok(id)
    }

    pub fn get(&self, id: SourceId) -> Option<&SourceRecord> {
        self.files.get(id.index())
    }

    /// Iterate source identities in deterministic logical-path order.
    pub fn iter(&self) -> std::slice::Iter<'_, SourceRecord> {
        self.files.iter()
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
