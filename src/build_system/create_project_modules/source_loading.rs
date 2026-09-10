//! Raw file I/O for Moth source files.
//!
//! Reads source file content from disk with structured error diagnostics.

use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerErrorMetadataKey};
use crate::compiler_frontend::source::SourceDatabase;
#[cfg(test)]
use crate::compiler_frontend::source::SourceRegistrationIndex;
use crate::compiler_frontend::symbols::string_interning::StringTable;

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

#[cfg(test)]
static SOURCE_READ_TRACK_PREFIX_FOR_TEST: std::sync::Mutex<Option<PathBuf>> =
    std::sync::Mutex::new(None);

#[cfg(test)]
static SOURCE_READ_COUNTS_BY_PATH_FOR_TEST: std::sync::LazyLock<
    std::sync::Mutex<HashMap<PathBuf, usize>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));

// -------------------------
//  Source Extraction
// -------------------------

/// Reads raw UTF-8 source text without constructing compiler diagnostics.
///
/// WHAT: exposes the filesystem operation separately from diagnostic construction.
/// WHY: the single-file synthetic Stage 0 path can load cache-miss source files in Rayon workers,
///      then convert any `std::io::Error` into a path-preserving infrastructure error on the
///      serial boundary.
pub(crate) fn read_source_code(file_path: &Path) -> Result<String, std::io::Error> {
    #[cfg(test)]
    if should_count_source_read_for_test(file_path) {
        *SOURCE_READ_COUNTS_BY_PATH_FOR_TEST
            .lock()
            .expect("source read path-count test hook lock poisoned")
            .entry(file_path.to_path_buf())
            .or_insert(0) += 1;
    }

    fs::read_to_string(file_path)
}

/// Source snapshots selected by the directory module-discovery walk.
///
/// Registration assigns every candidate a deterministic identity slot, but only sources reached
/// by compiler selection are read. Keeping this small path-keyed cache beside the discovery pass
/// lets that pass prepare a selected source before the final `SourceDatabaseBuilder` can retain
/// the text. Entries are drained into the builder after selection; unselected and provider-owned
/// slots remain pending and never allocate a snapshot.
#[derive(Debug, Default)]
pub(crate) struct SelectedSourceTextMap {
    entries: BTreeMap<PathBuf, Result<String, CompilerError>>,
}

impl SelectedSourceTextMap {
    /// Read one selected source at most once and return its cached text.
    ///
    /// A failed read is cached as well, so a repeated selection reports the same per-source
    /// failure without issuing another filesystem read.
    pub(crate) fn load(&mut self, canonical_path: &Path) -> Result<&str, CompilerError> {
        let entry = self
            .entries
            .entry(canonical_path.to_path_buf())
            .or_insert_with(|| {
                read_source_code(canonical_path)
                    .map_err(|error| source_read_error(canonical_path, error))
            });
        match entry {
            Ok(source) => Ok(source.as_str()),
            Err(error) => Err(error.clone()),
        }
    }

    /// Move selected snapshots and failures into their deterministic registration slots.
    ///
    /// Read failures are recorded on their slots but are not returned here: the selected
    /// preparation boundary already returns that source failure, and retaining it is needed so a
    /// diagnosed/failed result still has the complete source context. Only database lifecycle
    /// violations are returned.
    pub(crate) fn retain_into(
        &mut self,
        source_files: &mut SourceDatabase,
    ) -> Result<(), CompilerError> {
        let entries = std::mem::take(&mut self.entries);
        let mut first_error = None;
        for (canonical_path, result) in entries {
            let Some(source_id) = source_files
                .get_by_canonical_path(&canonical_path)
                .map(|record| record.id)
            else {
                first_error.get_or_insert_with(|| {
                    CompilerError::compiler_error(format!(
                        "selected source path {} has no source database slot",
                        canonical_path.display()
                    ))
                });
                continue;
            };

            match result {
                Ok(source) => {
                    if let Err(error) = source_files.retain_text(source_id, source) {
                        first_error.get_or_insert(error);
                    }
                }
                Err(error) => {
                    if let Err(record_error) =
                        source_files.record_source_load_error(source_id, error)
                    {
                        first_error.get_or_insert(record_error);
                    }
                }
            }
        }

        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
fn should_count_source_read_for_test(file_path: &Path) -> bool {
    let prefix = SOURCE_READ_TRACK_PREFIX_FOR_TEST
        .lock()
        .expect("source read test hook lock poisoned");

    prefix
        .as_ref()
        .is_none_or(|tracked_prefix| file_path.starts_with(tracked_prefix))
}

#[cfg(test)]
pub(crate) fn reset_source_read_count_for_test(tracked_prefix: &Path) {
    let mut prefix = SOURCE_READ_TRACK_PREFIX_FOR_TEST
        .lock()
        .expect("source read test hook lock poisoned");
    *prefix = Some(tracked_prefix.to_path_buf());
    SOURCE_READ_COUNTS_BY_PATH_FOR_TEST
        .lock()
        .expect("source read path-count test hook lock poisoned")
        .clear();
}

#[cfg(test)]
pub(crate) fn source_read_count_for_path_for_test(path: &Path) -> usize {
    SOURCE_READ_COUNTS_BY_PATH_FOR_TEST
        .lock()
        .expect("source read path-count test hook lock poisoned")
        .get(path)
        .copied()
        .unwrap_or(0)
}

/// Reads the contents of a source file from disk.
///
/// WHAT: performs UTF-8 file read with structured `CompilerError` diagnostics for common
///       failure modes (not found, permission denied).
/// WHY: every source file entering the compiler pipeline goes through this single boundary
///      so I/O failures are reported uniformly instead of leaking `std::io::Error`.
pub fn extract_source_code(
    file_path: &Path,
    _string_table: &mut StringTable,
) -> Result<String, CompilerError> {
    match read_source_code(file_path) {
        Ok(content) => Ok(content),

        Err(error) => Err(source_read_error(file_path, error)),
    }
}

#[cfg(test)]
/// Load every source already assigned a slot by the registration boundary.
///
/// Successful reads move their `String` directly into the corresponding source record. Read and
/// UTF-8 failures are retained as per-source errors so callers can surface them in the same lane
/// as the old on-demand read without aborting unrelated source preparation.
pub(crate) fn load_registered_source_texts(
    source_files: &mut SourceDatabase,
    registration_index: &SourceRegistrationIndex<'_>,
    _string_table: &mut StringTable,
) -> Result<(), CompilerError> {
    for canonical_path in registration_index.canonical_paths() {
        let source_id = source_files
            .get_by_canonical_path(canonical_path)
            .map(|record| record.id)
            .ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "registered source path {} has no source database slot",
                    canonical_path.display()
                ))
            })?;

        match read_source_code(canonical_path) {
            Ok(source) => source_files.retain_text(source_id, source)?,
            Err(error) => {
                source_files.record_source_load_error(
                    source_id,
                    source_read_error(canonical_path, error),
                )?;
            }
        }
    }
    Ok(())
}

/// Converts raw source-read failures into the existing structured compiler error shape.
pub(crate) fn source_read_error(file_path: &Path, error: std::io::Error) -> CompilerError {
    let suggestion: &'static str = if error.kind() == std::io::ErrorKind::NotFound {
        "Check that the file exists at the specified path"
    } else if error.kind() == std::io::ErrorKind::PermissionDenied {
        "Check that you have permission to read this file"
    } else {
        "Verify the file is accessible and not corrupted"
    };

    CompilerError::new_file_error(
        file_path,
        format!(
            "Error reading file when adding new moth files to parse: {:?}",
            error
        ),
        {
            let mut metadata = HashMap::new();
            metadata.insert(
                CompilerErrorMetadataKey::CompilationStage,
                String::from("File System"),
            );
            metadata.insert(
                CompilerErrorMetadataKey::PrimarySuggestion,
                String::from(suggestion),
            );
            metadata
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::fs;

    #[test]
    fn selected_source_text_map_reads_selected_once_and_leaves_other_slots_pending() {
        let _test_guard = crate::timing::lock_instrumentation_tests();
        let temp_root = tempfile::tempdir().expect("source-loading test root should exist");
        let selected_path = temp_root.path().join("selected.moth");
        let provider_path = temp_root.path().join("provider-owned.js");
        fs::write(&selected_path, "selected").expect("selected source should be writable");
        fs::write(&provider_path, "provider").expect("provider source should be writable");
        let selected_path =
            fs::canonicalize(selected_path).expect("selected source should canonicalize");
        let provider_path =
            fs::canonicalize(provider_path).expect("provider source should canonicalize");

        let tracked_root = fs::canonicalize(temp_root.path())
            .expect("source-loading test root should canonicalize");
        reset_source_read_count_for_test(&tracked_root);
        let mut selected = SelectedSourceTextMap::default();
        assert_eq!(
            selected
                .load(&selected_path)
                .expect("selected source should load"),
            "selected"
        );
        assert_eq!(
            selected
                .load(&selected_path)
                .expect("selected source should be cached"),
            "selected"
        );

        assert_eq!(source_read_count_for_path_for_test(&selected_path), 1);
        assert_eq!(source_read_count_for_path_for_test(&provider_path), 0);
        assert_eq!(selected.entries.len(), 1);
        assert!(selected.entries.contains_key(&selected_path));
    }
}
