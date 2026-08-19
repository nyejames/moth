//! Prepared output destinations and filesystem-safe artifact emission.
//!
//! WHAT: validates a complete output batch, resolves every destination once, and emits only the
//! prepared destinations.
//! WHY: output safety must be decided before the first filesystem mutation, while route and
//! builder semantics remain owned by the backend that produced the output records.

use super::manifest::{BUILD_MANIFEST_FILENAME, validate_relative_output_path};
use super::output_path::{
    OutputPathIdentity, canonicalize_output_path, is_lossless_portable_relative_path,
    output_path_component_identities, output_path_identity, path_starts_with_component_identity,
};
use crate::build_system::build::{FileKind, Project};
use crate::build_system::output::{OutputPlan, WriteMode};
use crate::build_system::utils::{file_error_with_rejection_reason, should_skip_unchanged_write};
use crate::compiler_frontend::compiler_errors::CompilerMessages;
use crate::compiler_frontend::symbols::string_interning::StringTable;

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

/// Typed reason for output-write rejection, used as a test-visible seam.
///
/// WHAT: distinguishes the specific safety contract violated when the writer
/// rejects an output batch, so tests can assert the exact rejection reason
/// rather than accepting any `ErrorType::File` infrastructure error.
/// WHY: all writer rejections share `ErrorType::File`, but they enforce
/// distinct safety contracts (path validity, destination uniqueness, symlink
/// containment, hard-link safety, etc.). Without a typed reason, a test for
/// one contract could pass when a different contract is violated.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum OutputRejectionReason {
    DanglingSymlinkInOutputRoot,
    NonPortablePathComponents,
    ReservedManifestDestination,
    DuplicateDestination,
    CaseOnlyCollision,
    DanglingSymlinkInDestination,
    EscapesOutputRoot,
    DestinationIsOutputRoot,
    NonLosslessCanonicalPath,
    ReservedManifestDestinationCanonical,
    DirectoryDestinationExistsAsNonDirectory,
    FileDestinationExistsAsNonFile,
    HardLinkedDestination,
    DestinationInspectionFailed,
    CanonicalDestinationCollision,
    FileAncestorConflict,
    ManifestHardLinkInspectionFailed,
    ManifestHardLinked,
    ManifestNotRegularFile,
    ManifestInspectionFailed,
    DirectoryCreationFailed,
    ParentDirCreationFailed,
    FileWriteFailed,
    InvalidRelativeOutputPath,
    OutputRootDanglingSymlink,
    OutputRootDangerousSystemPath,
    OutputRootNotInsideProject,
    OutputRootNotAdjacentToProject,
    OutputRootInspectionFailed,
}

impl OutputRejectionReason {
    pub(crate) fn as_metadata_value(&self) -> &'static str {
        match self {
            Self::DanglingSymlinkInOutputRoot => "dangling-symlink-in-output-root",
            Self::NonPortablePathComponents => "non-portable-path-components",
            Self::ReservedManifestDestination => "reserved-manifest-destination",
            Self::DuplicateDestination => "duplicate-destination",
            Self::CaseOnlyCollision => "case-only-collision",
            Self::DanglingSymlinkInDestination => "dangling-symlink-in-destination",
            Self::EscapesOutputRoot => "escapes-output-root",
            Self::DestinationIsOutputRoot => "destination-is-output-root",
            Self::NonLosslessCanonicalPath => "non-lossless-canonical-path",
            Self::ReservedManifestDestinationCanonical => "reserved-manifest-destination-canonical",
            Self::DirectoryDestinationExistsAsNonDirectory => {
                "directory-destination-exists-as-non-directory"
            }
            Self::FileDestinationExistsAsNonFile => "file-destination-exists-as-non-file",
            Self::HardLinkedDestination => "hard-linked-destination",
            Self::DestinationInspectionFailed => "destination-inspection-failed",
            Self::CanonicalDestinationCollision => "canonical-destination-collision",
            Self::FileAncestorConflict => "file-ancestor-conflict",
            Self::ManifestHardLinkInspectionFailed => "manifest-hard-link-inspection-failed",
            Self::ManifestHardLinked => "manifest-hard-linked",
            Self::ManifestNotRegularFile => "manifest-not-regular-file",
            Self::ManifestInspectionFailed => "manifest-inspection-failed",
            Self::DirectoryCreationFailed => "directory-creation-failed",
            Self::ParentDirCreationFailed => "parent-dir-creation-failed",
            Self::FileWriteFailed => "file-write-failed",
            Self::InvalidRelativeOutputPath => "invalid-relative-output-path",
            Self::OutputRootDanglingSymlink => "output-root-dangling-symlink",
            Self::OutputRootDangerousSystemPath => "output-root-dangerous-system-path",
            Self::OutputRootNotInsideProject => "output-root-not-inside-project",
            Self::OutputRootNotAdjacentToProject => "output-root-not-adjacent-to-project",
            Self::OutputRootInspectionFailed => "output-root-inspection-failed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PreparedOutputWrite {
    destinations: Vec<PreparedDestination>,
    pub(crate) managed_artifact_paths: HashSet<PathBuf>,
    pub(crate) explicit_directory_paths: HashSet<PathBuf>,
    pub(crate) manifest_destination: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PreparedDestination {
    output_file_index: usize,
    relative_path: PathBuf,
    canonical_relative_path: PathBuf,
    destination: PathBuf,
    is_directory: bool,
}

/// Prepare all final destinations before the writer creates a root or emits one artifact.
pub(crate) fn prepare_output_write(
    project: &Project,
    output_plan: &OutputPlan,
    string_table: &StringTable,
) -> Result<PreparedOutputWrite, CompilerMessages> {
    let output_root = output_plan.output_root();
    let canonical_output_root = canonicalize_output_path(output_root).map_err(|_| {
        file_error_with_rejection_reason(
            output_root,
            "Output root contains a dangling symlink component and cannot be prepared safely.",
            OutputRejectionReason::DanglingSymlinkInOutputRoot,
            string_table,
        )
    })?;
    let mut destinations = Vec::new();
    let mut managed_artifact_paths = HashSet::new();
    let mut explicit_directory_paths = HashSet::new();
    let mut identities: HashMap<OutputPathIdentity, PathBuf> = HashMap::new();

    for (output_file_index, output_file) in project.output_files.iter().enumerate() {
        if matches!(output_file.file_kind(), FileKind::NotBuilt) {
            continue;
        }

        let relative_path =
            validate_relative_output_path(output_file.relative_output_path(), string_table)?;

        let identity = output_path_identity(&relative_path).map_err(|_| {
            file_error_with_rejection_reason(
                &relative_path,
                "Output path must use normal portable path components.",
                OutputRejectionReason::NonPortablePathComponents,
                string_table,
            )
        })?;
        if path_starts_with_component_identity(&relative_path, Path::new(BUILD_MANIFEST_FILENAME)) {
            return Err(file_error_with_rejection_reason(
                &relative_path,
                format!(
                    "Output destination '{}' uses a path reserved for the internal build manifest.",
                    relative_path.display()
                ),
                OutputRejectionReason::ReservedManifestDestination,
                string_table,
            ));
        }
        if let Some(existing_path) = identities.get(&identity) {
            let message = if existing_path == &relative_path {
                format!(
                    "Duplicate output destination '{}'. Each output path must be unique.",
                    relative_path.display()
                )
            } else {
                format!(
                    "Output destinations '{}' and '{}' differ only by ASCII case or path spelling and cannot coexist safely.",
                    existing_path.display(),
                    relative_path.display()
                )
            };
            return Err(file_error_with_rejection_reason(
                &relative_path,
                message,
                OutputRejectionReason::DuplicateDestination,
                string_table,
            ));
        }
        identities.insert(identity.clone(), relative_path.clone());

        let destination = output_root.join(&relative_path);
        let canonical_destination = canonicalize_output_path(&destination).map_err(|_| {
            file_error_with_rejection_reason(
                &relative_path,
                "Output destination contains a dangling symlink component and cannot be prepared safely.",
                OutputRejectionReason::DanglingSymlinkInDestination,
                string_table,
            )
        })?;
        if !canonical_destination.starts_with(&canonical_output_root) {
            return Err(file_error_with_rejection_reason(
                &relative_path,
                format!(
                    "Output destination '{}' resolves outside the validated output root '{}'.",
                    relative_path.display(),
                    output_root.display()
                ),
                OutputRejectionReason::EscapesOutputRoot,
                string_table,
            ));
        }
        if canonical_destination == canonical_output_root {
            return Err(file_error_with_rejection_reason(
                &relative_path,
                format!(
                    "Output destination '{}' resolves to the output root itself.",
                    relative_path.display()
                ),
                OutputRejectionReason::DestinationIsOutputRoot,
                string_table,
            ));
        }
        let canonical_relative_path = canonical_destination
            .strip_prefix(&canonical_output_root)
            .expect("preflight already validated output-root containment")
            .to_path_buf();
        if !is_lossless_portable_relative_path(&canonical_relative_path) {
            return Err(file_error_with_rejection_reason(
                &relative_path,
                format!(
                    "Output destination '{}' resolves to a filesystem path that cannot be represented safely in the build manifest.",
                    relative_path.display()
                ),
                OutputRejectionReason::NonLosslessCanonicalPath,
                string_table,
            ));
        }
        if path_starts_with_component_identity(
            &canonical_relative_path,
            Path::new(BUILD_MANIFEST_FILENAME),
        ) {
            return Err(file_error_with_rejection_reason(
                &relative_path,
                format!(
                    "Output destination '{}' resolves to a path reserved for the internal build manifest.",
                    relative_path.display()
                ),
                OutputRejectionReason::ReservedManifestDestinationCanonical,
                string_table,
            ));
        }

        let is_directory = matches!(output_file.file_kind(), FileKind::Directory);
        validate_existing_destination(
            &canonical_destination,
            &relative_path,
            is_directory,
            string_table,
        )?;
        if is_directory {
            explicit_directory_paths.insert(canonical_relative_path.clone());
        } else if project.cleanup_policy.manages_path(&relative_path)
            || matches!(output_file.file_kind(), FileKind::Bytes(_))
        {
            managed_artifact_paths.insert(canonical_relative_path.clone());
        }

        destinations.push(PreparedDestination {
            output_file_index,
            relative_path,
            canonical_relative_path,
            destination: canonical_destination,
            is_directory,
        });
    }

    reject_canonical_destination_conflicts(&destinations, string_table)?;
    let manifest_destination =
        prepare_manifest_destination(output_root, &canonical_output_root, string_table)?;

    Ok(PreparedOutputWrite {
        destinations,
        managed_artifact_paths,
        explicit_directory_paths,
        manifest_destination,
    })
}

fn prepare_manifest_destination(
    output_root: &Path,
    canonical_output_root: &Path,
    string_table: &StringTable,
) -> Result<PathBuf, CompilerMessages> {
    let authored_manifest_path = output_root.join(BUILD_MANIFEST_FILENAME);
    match fs::symlink_metadata(&authored_manifest_path) {
        Ok(metadata) if metadata.file_type().is_file() => {
            let has_multiple_hard_links = inspect_hard_link_count(
                &authored_manifest_path,
                &metadata,
            )
            .map_err(|error| {
                file_error_with_rejection_reason(
                    &authored_manifest_path,
                    format!(
                        "The existing build manifest cannot be inspected safely for hard-link aliases: {error}"
                    ),
                    OutputRejectionReason::ManifestHardLinkInspectionFailed,
                    string_table,
                )
            })?;
            if has_multiple_hard_links {
                return Err(file_error_with_rejection_reason(
                    &authored_manifest_path,
                    "The existing build manifest is hard-linked to another filesystem path and cannot be overwritten safely.",
                    OutputRejectionReason::ManifestHardLinked,
                    string_table,
                ));
            }
        }
        Ok(_) => {
            return Err(file_error_with_rejection_reason(
                &authored_manifest_path,
                "The existing build manifest destination must be a regular file; symlinks, directories, and special files are not allowed.",
                OutputRejectionReason::ManifestNotRegularFile,
                string_table,
            ));
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => {
            return Err(file_error_with_rejection_reason(
                &authored_manifest_path,
                format!("The build manifest destination cannot be inspected safely: {error}"),
                OutputRejectionReason::ManifestInspectionFailed,
                string_table,
            ));
        }
    }

    Ok(canonical_output_root.join(BUILD_MANIFEST_FILENAME))
}

fn reject_canonical_destination_conflicts(
    destinations: &[PreparedDestination],
    string_table: &StringTable,
) -> Result<(), CompilerMessages> {
    let mut destinations_by_identity: HashMap<Vec<String>, usize> = HashMap::new();
    for (destination_index, destination) in destinations.iter().enumerate() {
        let identity = output_path_component_identities(&destination.canonical_relative_path)
            .expect("preflight already validated canonical relative output path");
        if let Some(&existing_index) = destinations_by_identity.get(&identity) {
            let existing = &destinations[existing_index];
            return Err(file_error_with_rejection_reason(
                &destination.relative_path,
                format!(
                    "Output destinations '{}' and '{}' resolve to the same portable filesystem path and cannot coexist safely.",
                    existing.relative_path.display(),
                    destination.relative_path.display()
                ),
                OutputRejectionReason::CanonicalDestinationCollision,
                string_table,
            ));
        }
        destinations_by_identity.insert(identity, destination_index);
    }

    for destination in destinations {
        let identity = output_path_component_identities(&destination.canonical_relative_path)
            .expect("preflight already validated canonical relative output path");
        let mut ancestor_identity = Vec::with_capacity(identity.len().saturating_sub(1));
        for component in identity.iter().take(identity.len().saturating_sub(1)) {
            ancestor_identity.push(component.clone());
            let Some(&ancestor_index) = destinations_by_identity.get(&ancestor_identity) else {
                continue;
            };
            let ancestor = &destinations[ancestor_index];
            if !ancestor.is_directory {
                return Err(file_error_with_rejection_reason(
                    &destination.relative_path,
                    format!(
                        "Output destination '{}' resolves below file '{}'. Only an explicit directory output may contain child destinations.",
                        destination.relative_path.display(),
                        ancestor.relative_path.display()
                    ),
                    OutputRejectionReason::FileAncestorConflict,
                    string_table,
                ));
            }
        }
    }

    Ok(())
}

fn validate_existing_destination(
    destination: &Path,
    relative_path: &Path,
    expects_directory: bool,
    string_table: &StringTable,
) -> Result<(), CompilerMessages> {
    let metadata = match fs::metadata(destination) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(file_error_with_rejection_reason(
                relative_path,
                format!(
                    "Existing output destination '{}' cannot be inspected safely: {error}",
                    relative_path.display()
                ),
                OutputRejectionReason::DestinationInspectionFailed,
                string_table,
            ));
        }
    };

    if expects_directory && !metadata.is_dir() {
        return Err(file_error_with_rejection_reason(
            relative_path,
            format!(
                "Directory output destination '{}' already exists as a non-directory path.",
                relative_path.display()
            ),
            OutputRejectionReason::DirectoryDestinationExistsAsNonDirectory,
            string_table,
        ));
    }
    if !expects_directory && !metadata.is_file() {
        return Err(file_error_with_rejection_reason(
            relative_path,
            format!(
                "File output destination '{}' already exists as a non-regular path.",
                relative_path.display()
            ),
            OutputRejectionReason::FileDestinationExistsAsNonFile,
            string_table,
        ));
    }
    if metadata.is_file() {
        let has_multiple_hard_links = inspect_hard_link_count(destination, &metadata).map_err(
            |error| {
                file_error_with_rejection_reason(
                    relative_path,
                    format!(
                        "Existing output destination '{}' cannot be inspected safely for hard-link aliases: {error}",
                        relative_path.display()
                    ),
                    OutputRejectionReason::DestinationInspectionFailed,
                    string_table,
                )
            },
        )?;
        if has_multiple_hard_links {
            return Err(file_error_with_rejection_reason(
                relative_path,
                format!(
                    "Output destination '{}' is hard-linked to another filesystem path and cannot be overwritten safely.",
                    relative_path.display()
                ),
                OutputRejectionReason::HardLinkedDestination,
                string_table,
            ));
        }
    }

    Ok(())
}

fn inspect_hard_link_count(path: &Path, metadata: &fs::Metadata) -> std::io::Result<bool> {
    #[cfg(unix)]
    {
        let _ = path;
        use std::os::unix::fs::MetadataExt;

        Ok(metadata.nlink() > 1)
    }
    #[cfg(windows)]
    {
        let _ = metadata;
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
        };

        let file = fs::File::open(path)?;
        let mut information = BY_HANDLE_FILE_INFORMATION::default();
        let succeeded =
            unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut information) };
        if succeeded == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(information.nNumberOfLinks > 1)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        let _ = metadata;
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "hard-link identity inspection is unsupported on this target",
        ))
    }
}

#[cfg(test)]
#[path = "tests/writer_tests.rs"]
mod tests;

/// Emit only the destinations prepared by [`prepare_output_write`].
pub(crate) fn emit_prepared_output_files(
    project: &Project,
    prepared_write: &PreparedOutputWrite,
    write_mode: WriteMode,
    string_table: &StringTable,
) -> Result<(), CompilerMessages> {
    for prepared_destination in &prepared_write.destinations {
        let output_file = &project.output_files[prepared_destination.output_file_index];
        let destination = &prepared_destination.destination;

        match output_file.file_kind() {
            FileKind::NotBuilt => {}
            FileKind::Directory => fs::create_dir_all(destination).map_err(|error| {
                file_error_with_rejection_reason(
                    destination,
                    format!(
                        "Failed to create output directory '{}': {error}",
                        destination.display()
                    ),
                    OutputRejectionReason::DirectoryCreationFailed,
                    string_table,
                )
            })?,
            FileKind::Js(content) | FileKind::Html(content) => {
                write_string_output(destination, content, write_mode, string_table)?;
            }
            FileKind::Wasm(bytes) | FileKind::Bytes(bytes) => {
                write_bytes_output(destination, bytes, write_mode, string_table)?;
            }
        }
    }

    Ok(())
}

fn create_parent_dir_if_needed(
    path: &Path,
    string_table: &StringTable,
) -> Result<(), CompilerMessages> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };

    fs::create_dir_all(parent).map_err(|error| {
        file_error_with_rejection_reason(
            parent,
            format!(
                "Failed to create parent directory '{}': {error}",
                parent.display()
            ),
            OutputRejectionReason::ParentDirCreationFailed,
            string_table,
        )
    })
}

fn write_string_output(
    destination: &Path,
    content: &str,
    write_mode: WriteMode,
    string_table: &StringTable,
) -> Result<(), CompilerMessages> {
    write_bytes_output(destination, content.as_bytes(), write_mode, string_table)
}

fn write_bytes_output(
    destination: &Path,
    content: &[u8],
    write_mode: WriteMode,
    string_table: &StringTable,
) -> Result<(), CompilerMessages> {
    create_parent_dir_if_needed(destination, string_table)?;

    if should_skip_unchanged_write(destination, content, write_mode) {
        return Ok(());
    }

    fs::write(destination, content).map_err(|error| {
        file_error_with_rejection_reason(
            destination,
            format!(
                "Failed to write output file '{}': {error}",
                destination.display()
            ),
            OutputRejectionReason::FileWriteFailed,
            string_table,
        )
    })
}
