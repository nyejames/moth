//! Raw file I/O for Moth source files.
//!
//! Reads source file content from disk with structured error diagnostics.

use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerErrorMetadataKey};
use crate::compiler_frontend::symbols::string_interning::StringTable;

use std::collections::HashMap;
use std::fs;
use std::path::Path;

#[cfg(test)]
use std::path::PathBuf;

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
///      then convert any `std::io::Error` on the serial boundary where the shared `StringTable`
///      is available.
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
    string_table: &mut StringTable,
) -> Result<String, CompilerError> {
    match read_source_code(file_path) {
        Ok(content) => Ok(content),

        Err(error) => Err(source_read_error(file_path, error, string_table)),
    }
}

/// Converts raw source-read failures into the existing structured compiler error shape.
pub(crate) fn source_read_error(
    file_path: &Path,
    error: std::io::Error,
    string_table: &mut StringTable,
) -> CompilerError {
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
        string_table,
    )
}
