//! The source trees every workspace gate reads, and how it names what it found.
//!
//! WHAT: walks a source root for Rust files and turns an absolute path into the workspace-relative
//!       name a report prints.
//! WHY:  three gates read the same trees. A walk that each gate owns privately is three chances
//!       for one of them to skip a file silently, and three different spellings of the same path
//!       in three reports a reviewer is comparing.
//!
//! # What this module owns
//! - The fail-closed walk of a source root
//! - Workspace-relative display paths and the workspace root itself
//!
//! # What this module does NOT own
//! - Which roots a gate walks, or what it does with a file (see each gate)

use std::fs;
use std::path::{Path, PathBuf};

/// Every `.rs` file under `root`, failing closed on any directory that cannot be read.
///
/// A scan that skips an unreadable directory reports coverage it never measured.
pub(crate) fn walk_rust_files(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    let mut pending = vec![root.to_path_buf()];

    while let Some(directory) = pending.pop() {
        let entries = fs::read_dir(&directory)
            .map_err(|error| format!("failed to read '{}': {error}", directory.display()))?;

        for entry in entries {
            let entry = entry.map_err(|error| {
                format!(
                    "failed to read an entry of '{}': {error}",
                    directory.display()
                )
            })?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)
                .map_err(|error| format!("failed to stat '{}': {error}", path.display()))?;

            if metadata.is_dir() {
                pending.push(path);
            } else if metadata.is_file()
                && path.extension().is_some_and(|extension| extension == "rs")
            {
                files.push(path);
            }
        }
    }

    files.sort();
    Ok(files)
}

/// Workspace-relative path with `/` separators, so the report reads the same on every platform.
///
/// A path component that is not UTF-8 is an error rather than a lossy replacement: the report
/// names files a reader is expected to open, and a substituted character names a different file.
pub(crate) fn relative_display_path(workspace_root: &Path, path: &Path) -> Result<String, String> {
    let relative = path.strip_prefix(workspace_root).unwrap_or(path);
    let mut segments = Vec::new();

    for component in relative.components() {
        let segment = component.as_os_str().to_str().ok_or_else(|| {
            format!(
                "path '{}' has a component that is not valid UTF-8",
                relative.display()
            )
        })?;
        segments.push(segment);
    }

    Ok(segments.join("/"))
}

pub(crate) fn workspace_root() -> Result<PathBuf, String> {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "xtask manifest has no parent directory".to_string())
}

#[cfg(test)]
mod tests;
