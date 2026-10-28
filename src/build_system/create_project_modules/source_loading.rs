//! Raw file I/O for Moth source files.
//!
//! Reads source file content from disk. Read failures stay typed until a registered source
//! supplies the logical path that a path-named diagnostic needs.

use super::source_discovery_error::SourceDiscoveryError;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::compiler_errors::{
    InfrastructureDetail, InfrastructureFailure,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::token_lexical;
use crate::compiler_frontend::source::{SourceDatabase, SourceDatabaseError, SourceId};
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use std::collections::BTreeMap;
#[cfg(not(test))]
use std::fs;
use std::path::{Path, PathBuf};

// -------------------------
//  Source Extraction
// -------------------------
// WHAT: choose one reader for the whole build rather than branching on every file.
// WHY: test-only read accounting stays in the test support module while shipping I/O remains
//      branch-free.

type SourceReader = fn(&Path) -> Result<String, std::io::Error>;

#[cfg(not(test))]
fn read_source_code_from_disk(file_path: &Path) -> Result<String, std::io::Error> {
    fs::read_to_string(file_path)
}

#[cfg(not(test))]
const SOURCE_READER: SourceReader = read_source_code_from_disk;

#[cfg(test)]
const SOURCE_READER: SourceReader = super::source_loading_test_support::read_source_code_for_test;

/// Reads raw UTF-8 source text without constructing compiler diagnostics.
///
/// WHAT: exposes the filesystem operation separately from classification and diagnostic
///       construction.
/// WHY: missing-source loads read in Rayon workers, then classify each result on the serial
///      boundary, where a registered source can name invalid UTF-8 by its logical path.
pub(crate) fn read_source_code(file_path: &Path) -> Result<String, std::io::Error> {
    SOURCE_READER(file_path)
}

/// Classified failure of one source read, before any diagnostic is built.
///
/// WHAT: separates invalid UTF-8 from filesystem failures.
/// WHY: the path-named `NON_UTF8_SOURCE` diagnostic needs the source's logical path, which only a
///      registered source has. Callers convert `NonUtf8` through `registered_source_read_error`
///      once the source owns a `SourceId`.
#[derive(Debug, Clone)]
pub(crate) enum SourceReadError {
    NonUtf8,
    Infrastructure(InfrastructureFailure),
}

/// Classifies the I/O failure of one read of `file_path`.
///
/// WHAT: `InvalidData` is how `read_to_string` reports bytes that are not UTF-8; every other
///       failure is a filesystem infrastructure fact that names the host path.
/// WHY: the parallel missing-source loader returns raw `io::Error`s, so the serial boundary needs
///      the same classification that `extract_source_code` applies.
pub(crate) fn classify_source_read_error(
    file_path: &Path,
    error: std::io::Error,
) -> SourceReadError {
    if error.kind() == std::io::ErrorKind::InvalidData {
        return SourceReadError::NonUtf8;
    }
    SourceReadError::Infrastructure(source_read_error(file_path, error))
}

/// Reads one source file and classifies a failure without constructing a diagnostic.
///
/// WHAT: invalid UTF-8 becomes `SourceReadError::NonUtf8`; every other I/O failure becomes a
///       path-preserving infrastructure failure.
/// WHY: a registered caller converts the result with `registered_source_read_error`, so every
///      invalid-UTF-8 source is reported through the same path-named diagnostic.
pub(crate) fn extract_source_code(file_path: &Path) -> Result<String, SourceReadError> {
    read_source_code(file_path).map_err(|error| classify_source_read_error(file_path, error))
}

/// Converts a read failure for a source that already owns a database slot into its diagnostic.
///
/// WHAT: invalid UTF-8 becomes `NON_UTF8_SOURCE` named by the slot's logical path, re-interned in
///       the caller's fork; infrastructure failures pass through unchanged.
/// WHY: a diagnostic may name only the portable logical path, and only a registered source has one.
pub(crate) fn registered_source_read_error(
    error: SourceReadError,
    source_id: SourceId,
    source_files: &SourceDatabase,
    path_fork: &mut PathInternerFork,
) -> SourceDiscoveryError {
    match error {
        SourceReadError::NonUtf8 => match source_files.logical_path_in_fork(source_id, path_fork) {
            Ok(path) => token_lexical::non_utf8_source(path, None).into(),
            Err(error) => error.into(),
        },
        SourceReadError::Infrastructure(error) => error.into(),
    }
}

/// Converts a retention failure for one registered source into its discovery error.
///
/// WHAT: a snapshot too large for `u32` byte offsets becomes a source-span capacity diagnostic
///       named by the source's logical path, re-interned in the caller's fork. Every other
///       database failure keeps its typed conversion.
/// WHY: the size limit belongs to one user file, so the diagnostic must say which file to shorten.
pub(crate) fn retained_source_error(
    error: SourceDatabaseError,
    source_id: SourceId,
    source_files: &SourceDatabase,
    path_fork: &mut PathInternerFork,
) -> SourceDiscoveryError {
    match source_files.snapshot_rejection_draft(error, source_id, path_fork) {
        Ok(draft) => draft.into(),
        Err(error) => error.into(),
    }
}

/// Reads a registered source's text, naming its logical path when the bytes are not UTF-8.
///
/// WHAT: classifies the read failure once. Invalid UTF-8 becomes a `NON_UTF8_SOURCE` diagnostic
///       through `registered_source_read_error` and moves the slot to `NonUtf8`; an infrastructure
///       failure fails the slot and passes through typed.
/// WHY: discovery registers a source before reading it, so every read failure can name the
///      portable logical path, and the database records the terminal state of the slot.
pub(crate) fn read_registered_source(
    file_path: &Path,
    source_id: SourceId,
    source_files: &mut SourceDatabase,
    path_fork: &mut PathInternerFork,
) -> Result<String, SourceDiscoveryError> {
    match extract_source_code(file_path) {
        Ok(source) => Ok(source),
        Err(SourceReadError::NonUtf8) => {
            source_files.record_source_non_utf8(source_id)?;
            Err(registered_source_read_error(
                SourceReadError::NonUtf8,
                source_id,
                source_files,
                path_fork,
            ))
        }
        Err(SourceReadError::Infrastructure(failure)) => {
            source_files.record_source_load_error(source_id, failure.clone())?;
            Err(SourceDiscoveryError::Infrastructure(failure))
        }
    }
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
    entries: BTreeMap<PathBuf, Result<String, SourceReadError>>,
}

impl SelectedSourceTextMap {
    /// Read one selected source at most once and return its cached text.
    ///
    /// A failed read is cached as well, so a repeated selection reports the same classified
    /// failure without issuing another filesystem read. Invalid UTF-8 is a terminal `NonUtf8`
    /// outcome; the caller builds the path-named diagnostic from the registered slot.
    pub(crate) fn load(&mut self, canonical_path: &Path) -> Result<&str, SourceReadError> {
        let entry = self
            .entries
            .entry(canonical_path.to_path_buf())
            .or_insert_with(|| extract_source_code(canonical_path));
        match entry {
            Ok(source) => Ok(source.as_str()),
            Err(error) => Err(error.clone()),
        }
    }

    /// Whether a selected read succeeded while its text is still waiting for database retention.
    pub(crate) fn contains_successful(&self, canonical_path: &Path) -> bool {
        matches!(self.entries.get(canonical_path), Some(Ok(_)))
    }

    /// Source identities whose cached reads succeeded and will become loaded records.
    ///
    /// Source discovery owns the last producer outcome for every successful selected read. The
    /// source owner uses this list after `retain_into` and pending-builder adoption to finalize
    /// those records individually.
    pub(crate) fn selected_source_ids(&self, source_files: &SourceDatabase) -> Vec<SourceId> {
        self.entries
            .iter()
            .filter(|(_, cached)| cached.is_ok())
            .filter_map(|(canonical_path, _)| {
                source_files
                    .get_by_canonical_path(canonical_path)
                    .map(|slot| slot.id)
            })
            .collect()
    }

    /// Move selected snapshots and infrastructure failures into their registration slots.
    ///
    /// The selected preparation boundary already returned every failed read, so this pass only
    /// records infrastructure failures and invalid-UTF-8 terminal states on their slots, and
    /// retains successful text. Invalid UTF-8 has no snapshot; its path-named diagnostic is the
    /// outcome that the boundary returned. An oversized snapshot or a database lifecycle violation
    /// is returned here as a path-named diagnostic.
    pub(crate) fn retain_into(
        &mut self,
        source_files: &mut SourceDatabase,
        path_fork: &mut PathInternerFork,
    ) -> Result<(), SourceDiscoveryError> {
        let entries = std::mem::take(&mut self.entries);
        let mut first_error: Option<SourceDiscoveryError> = None;
        for (canonical_path, cached) in entries {
            let Some(source_id) = source_files
                .get_by_canonical_path(&canonical_path)
                .map(|record| record.id)
            else {
                first_error.get_or_insert_with(|| {
                    InfrastructureFailure::internal(
                        CompilerStage::SourceLoading,
                        format!(
                            "selected source path {} has no source database slot",
                            canonical_path.display()
                        ),
                    )
                    .into()
                });
                continue;
            };

            match cached {
                Ok(source) => {
                    if let Err(error) = source_files.retain_text(source_id, source) {
                        first_error.get_or_insert_with(|| {
                            retained_source_error(error, source_id, source_files, path_fork)
                        });
                    }
                }
                Err(SourceReadError::Infrastructure(error)) => {
                    if let Err(record_error) =
                        source_files.record_source_load_error(source_id, error)
                    {
                        first_error.get_or_insert(record_error.into());
                    }
                }
                Err(SourceReadError::NonUtf8) => {
                    if let Err(record_error) = source_files.record_source_non_utf8(source_id) {
                        first_error.get_or_insert(record_error.into());
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

/// Converts raw source-read failures into typed filesystem infrastructure facts.
pub(crate) fn source_read_error(file_path: &Path, error: std::io::Error) -> InfrastructureFailure {
    let suggestion: &'static str = if error.kind() == std::io::ErrorKind::NotFound {
        "Check that the file exists at the specified path"
    } else if error.kind() == std::io::ErrorKind::PermissionDenied {
        "Check that you have permission to read this file"
    } else {
        "Verify the file is accessible and not corrupted"
    };

    InfrastructureFailure::filesystem(
        CompilerStage::SourceLoading,
        file_path,
        format!(
            "Error reading file when adding new moth files to parse: {:?}",
            error
        ),
    )
    .with_detail(InfrastructureDetail::PrimarySuggestion(suggestion.into()))
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
        super::super::source_loading_test_support::reset_source_read_count_for_test(&tracked_root);
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

        assert_eq!(
            super::super::source_loading_test_support::source_read_count_for_path_for_test(
                &selected_path
            ),
            1
        );
        assert_eq!(
            super::super::source_loading_test_support::source_read_count_for_path_for_test(
                &provider_path
            ),
            0
        );
        assert_eq!(selected.entries.len(), 1);
        assert!(selected.entries.contains_key(&selected_path));
    }

    #[test]
    fn selected_source_text_map_caches_invalid_utf8_as_a_terminal_non_utf8_outcome() {
        let _test_guard = crate::timing::lock_instrumentation_tests();
        let temp_root = tempfile::tempdir().expect("source-loading test root should exist");
        let invalid_path = temp_root.path().join("invalid.moth");
        fs::write(&invalid_path, [0xff_u8, 0xfe, b'x']).expect("invalid source should be writable");
        let invalid_path =
            fs::canonicalize(invalid_path).expect("invalid source should canonicalize");

        let tracked_root = fs::canonicalize(temp_root.path())
            .expect("source-loading test root should canonicalize");
        super::super::source_loading_test_support::reset_source_read_count_for_test(&tracked_root);
        let mut selected = SelectedSourceTextMap::default();
        assert!(matches!(
            selected.load(&invalid_path),
            Err(SourceReadError::NonUtf8)
        ));
        assert!(matches!(
            selected.load(&invalid_path),
            Err(SourceReadError::NonUtf8)
        ));

        assert_eq!(
            super::super::source_loading_test_support::source_read_count_for_path_for_test(
                &invalid_path
            ),
            1
        );
        assert!(!selected.contains_successful(&invalid_path));
    }
}
