//! Stage 0 source-backed package-tree collision detection.
//!
//! WHAT: scans source-backed package roots for sibling `.moth` file / folder name collisions and reports
//! them as typed project-structure diagnostics.
//! WHY: unambiguous import path segments are a prerequisite for correct import resolution. The
//! collision rule is Stage 0-owned, not HTML-builder-specific.
//!
//! Entry-root collisions are owned by `SourceTreeIndex::discover` during the single entry-root
//! traversal.

use crate::builder_surface::SourcePackageRegistry;
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages};
use crate::compiler_frontend::compiler_messages::InvalidConfigReason;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::projects::settings::LANGUAGE_SOURCE_EXTENSION;

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use super::project_structure_diagnostics::{
    non_utf8_filesystem_name_error, path_id, project_structure_messages,
};

/// Reject sibling `.moth` file stems and folder names that share the same import name inside
/// source-backed package trees.
///
/// WHAT: for every source-backed package root, collects the set of `.moth` file stems (excluding `.js`
/// files) and folder names. If any stem collides with a folder name, emits a typed diagnostic.
/// WHY: Moth imports resolve a path segment to either a `.moth` file or a folder; sharing the
/// same stem makes the import name ambiguous.
///
/// Entry-root collisions are checked by `SourceTreeIndex::discover` during the single entry-root
/// traversal. Source-backed package trees remain separate because registered source-backed package traversal
/// lives outside entry-root indexing.
///
/// The rule applies even when the folder is empty or contains no Moth files.
pub(super) fn validate_source_package_tree_collisions(
    source_packages: &SourcePackageRegistry,
    string_table: &mut StringTable,
) -> Result<(), CompilerMessages> {
    for package in source_packages.iter() {
        let crate::builder_surface::ProvidedSourceRoot::Filesystem(root) = &package.root;
        if root.is_dir() {
            validate_directory_tree_collisions(root, string_table)?;
        }
    }

    Ok(())
}

/// Walk one directory tree and check every directory for sibling collisions.
fn validate_directory_tree_collisions(
    root: &Path,
    string_table: &mut StringTable,
) -> Result<(), CompilerMessages> {
    let mut directories_to_scan = vec![root.to_path_buf()];

    while let Some(directory) = directories_to_scan.pop() {
        let entries = fs::read_dir(&directory).map_err(|error| {
            CompilerMessages::from_error_ref(
                CompilerError::file_error(
                    &directory,
                    format!(
                        "Failed to read directory while checking import-name collisions: {error}"
                    ),
                    string_table,
                ),
                string_table,
            )
        })?;

        let mut file_stems: BTreeSet<String> = BTreeSet::new();
        let mut folder_names: BTreeSet<String> = BTreeSet::new();
        let mut subdirectories: Vec<PathBuf> = Vec::new();

        for entry in entries {
            let entry = entry.map_err(|error| {
                CompilerMessages::from_error_ref(
                    CompilerError::file_error(
                        &directory,
                        format!(
                            "Failed to read directory entry while checking import-name collisions: {error}"
                        ),
                        string_table,
                    ),
                    string_table,
                )
            })?;

            let path = entry.path();
            let name = path.file_name().and_then(|n| n.to_str()).ok_or_else(|| {
                non_utf8_filesystem_name_error(
                    &path,
                    "import-name collision check entry",
                    string_table,
                )
            })?;

            if path.is_dir() {
                folder_names.insert(name.to_owned());
                subdirectories.push(path);
            } else if let Some(extension) = path.extension().and_then(|e| e.to_str())
                && extension == LANGUAGE_SOURCE_EXTENSION
                && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
            {
                file_stems.insert(stem.to_owned());
            }
        }

        // Report the first collision found in this directory.
        for stem in &file_stems {
            if folder_names.contains(stem) {
                let file_name_id = string_table.intern(&format!("{stem}.moth"));
                let folder_name_id = string_table.intern(stem);

                return Err(project_structure_messages(
                    &directory,
                    InvalidConfigReason::BstFileFolderCollision {
                        file_name: file_name_id,
                        folder_name: folder_name_id,
                        directory: path_id(&directory, string_table),
                    },
                    string_table,
                ));
            }
        }

        subdirectories.sort();
        directories_to_scan.extend(subdirectories);
    }

    Ok(())
}
