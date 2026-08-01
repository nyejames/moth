//! Build-system output policy: the pure output-folder classifier and its durable result.
//!
//! WHAT: owns `ValidatedOutputFolder` and the pure classifier shared by config diagnostics and
//! output planning.
//! WHY: directory output roots must be classified and validated once so config diagnostics and
//! the output plan agree on the same result.

use crate::build_system::output::output_path::parse_relative_path;
use crate::compiler_frontend::compiler_messages::InvalidOutputFolderReason;

use std::path::{Path, PathBuf};

// -------------------------
//  Validated Output Folder
// -------------------------

/// A validated project-relative output folder with its resolved filesystem path.
///
/// WHAT: carries the canonical relative spelling and the resolved absolute path of one directory
/// output setting after classification.
/// WHY: config validation carries this value forward instead of re-joining and re-checking output
/// paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedOutputFolder {
    pub relative_path: PathBuf,
    pub resolved_path: PathBuf,
}

// -------------------------
//  Output-Folder Classifier
// -------------------------

/// Classify one directory output setting against the project boundary.
///
/// WHAT: validates that `relative` is a non-empty, relative, normal portable path that resolves to
/// a folder strictly inside the project root but outside the source entry root.
/// WHY: directory output roots must be safe and distinct before any output writing or cleanup
/// runs. This pure classifier is shared by config diagnostics and plan construction.
///
/// Rejects empty paths, absolute and rooted paths, platform-prefix paths, parent-directory and
/// authored `.` segments anywhere in the path, and a resolved path equal to or inside an
/// explicitly configured non-root `entry_root`.
///
/// `resolved_entry_root` is `None` for the transitional empty or `.` entry root form, where
/// entry-root containment is not enforced and the output is validated against the project root
/// only.
pub(crate) fn classify_output_folder(
    relative: &Path,
    project_root: &Path,
    resolved_entry_root: Option<&Path>,
) -> Result<ValidatedOutputFolder, InvalidOutputFolderReason> {
    parse_relative_path(&relative.to_string_lossy())?;

    let resolved_path = project_root.join(relative);
    if let Some(entry_root) = resolved_entry_root
        && (resolved_path == entry_root || resolved_path.starts_with(entry_root))
    {
        return Err(InvalidOutputFolderReason::InsideOrEqualToEntryRoot);
    }

    Ok(ValidatedOutputFolder {
        relative_path: relative.to_path_buf(),
        resolved_path,
    })
}
