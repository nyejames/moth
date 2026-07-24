//! Frontend file identity types and source-file tables.
//!
//! WHAT: defines explicit file identifiers plus canonical/logical path metadata.
//! WHY: semantic identity must not be reconstructed from filesystem/path strings.

use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::paths::path_resolution::ProjectPathResolver;
use crate::compiler_frontend::symbols::interned_path::{InternedPath, NonUtf8PathComponent};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use rustc_hash::FxHashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FileId(pub u32);

#[derive(Debug, Clone)]
pub struct SourceFileIdentity {
    pub file_id: FileId,
    pub canonical_os_path: PathBuf,
    pub logical_path: InternedPath,
}

#[derive(Debug, Clone, Default)]
pub struct SourceFileTable {
    files: Vec<SourceFileIdentity>,
    canonical_to_id: FxHashMap<PathBuf, FileId>,
}

impl SourceFileTable {
    pub fn empty() -> Self {
        Self::default()
    }

    /// Builds deterministic file identities for one module.
    ///
    /// WHAT: canonical files are sorted by logical path before IDs are assigned.
    /// WHY: deterministic ID assignment must not depend on host filesystem iteration order.
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
            let file_id = FileId(index as u32);
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

            canonical_to_id.insert(canonical.clone(), file_id);
            let identity = SourceFileIdentity {
                file_id,
                canonical_os_path: canonical,
                logical_path,
            };

            files.push(identity);
        }

        Ok(Self {
            files,
            canonical_to_id,
        })
    }

    pub fn get_by_canonical_path(&self, canonical_path: &Path) -> Option<&SourceFileIdentity> {
        let file_id = self.canonical_to_id.get(canonical_path)?;
        self.get(*file_id)
    }

    pub fn get(&self, file_id: FileId) -> Option<&SourceFileIdentity> {
        self.files.get(file_id.0 as usize)
    }

    /// Iterate the source file identities in deterministic (logical-path) order.
    ///
    /// WHY: semantic compilation derives source logical paths from the retained identity table
    ///      instead of carrying raw source paths, so the build-system/frontend boundary stays
    ///      free of `PreparedSourceInput` after preparation.
    pub fn iter(&self) -> std::slice::Iter<'_, SourceFileIdentity> {
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
