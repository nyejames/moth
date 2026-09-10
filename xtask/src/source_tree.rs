//! The source trees every workspace gate reads, and how it names what it found.
//!
//! WHAT: walks a source root fail-closed and turns an absolute path into the workspace-relative
//!       name a report prints.
//! WHY:  four gates read the same trees. A walk that each gate owns privately is four chances
//!       for one of them to skip a file silently, and four different spellings of the same path
//!       in four reports a reviewer is comparing.
//!
//! # What this module owns
//! - The fail-closed walk of a source root
//! - Workspace-relative display paths and the workspace root itself
//!
//! # What this module does NOT own
//! - Which roots a gate walks, or what it does with a file (see each gate)

use std::fs;
use std::path::{Path, PathBuf};

/// Whether a directory visitor should descend into the current directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WalkDecision {
    Continue,
    SkipDescendants,
}

/// Walk every directory entry under `root`, failing closed on any path that cannot be read.
///
/// Directory entries are visited in lexicographic path order so gates produce deterministic
/// findings without each implementing a private sorter.
pub(crate) fn walk_source_tree<F>(root: &Path, mut visit: F) -> Result<(), String>
where
    F: FnMut(&Path, &fs::Metadata) -> Result<WalkDecision, String>,
{
    walk_source_directory(root, &mut visit)
}

fn walk_source_directory(
    directory: &Path,
    visit: &mut dyn FnMut(&Path, &fs::Metadata) -> Result<WalkDecision, String>,
) -> Result<(), String> {
    let mut entries = Vec::new();
    let reader = fs::read_dir(directory)
        .map_err(|error| format!("failed to read '{}': {error}", directory.display()))?;

    for entry in reader {
        entries.push(entry.map_err(|error| {
            format!(
                "failed to read an entry of '{}': {error}",
                directory.display()
            )
        })?);
    }

    entries.sort_by_key(|entry| entry.path());

    for entry in entries {
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| format!("failed to stat '{}': {error}", path.display()))?;

        match visit(&path, &metadata)? {
            WalkDecision::Continue if metadata.is_dir() => {
                walk_source_directory(&path, visit)?;
            }
            WalkDecision::Continue | WalkDecision::SkipDescendants => {}
        }
    }

    Ok(())
}

/// Every `.rs` file under `root`, failing closed on any directory that cannot be read.
///
/// A scan that skips an unreadable directory reports coverage it never measured.
pub(crate) fn walk_rust_files(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();

    walk_source_tree(root, |path, metadata| {
        if metadata.is_file() && path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path.to_path_buf());
        }

        Ok(WalkDecision::Continue)
    })?;

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
