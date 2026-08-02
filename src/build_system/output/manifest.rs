//! Conservative manifest-backed cleanup for build outputs.
//!
//! WHAT: owns manifest parsing, ownership recovery, persistence, output-root safety validation,
//! and stale artifact removal.
//! WHY: build orchestration should stay focused on compilation and file emission while manifest
//! lifecycle and cleanup remain isolated behind one safety-first module.

use crate::build_system::build_profile::BuildProfile;
use crate::build_system::output::{BuilderKind, CleanupPolicy, OutputOwner};
use crate::build_system::utils::{file_error_messages, should_skip_unchanged_write};
use crate::compiler_frontend::compiler_errors::{CompilerMessages, SourceLocation};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidConfigReason, InvalidOutputFolderReason,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;

use saying::say;

use std::collections::{BTreeSet, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::WriteMode;
use super::output_path::{
    canonicalize_output_path, is_lossless_portable_relative_path, output_path_identity,
    path_starts_with_component_identity, relative_path_contains_symlink_component,
};
use super::policy::{join_extensions_csv, normalize_managed_extension};

/// Manifest file written to the output root to track which managed build artifacts exist.
pub(crate) const BUILD_MANIFEST_FILENAME: &str = ".moth_manifest";
const BUILD_MANIFEST_HEADER_V4: &str = "# moth-manifest v4";
const BUILD_MANIFEST_HEADER_V3: &str = "# moth-manifest v3";
const BUILD_MANIFEST_HEADER_PREFIX: &str = "# moth-manifest ";
const BUILD_MANIFEST_BUILDER_PREFIX: &str = "# builder: ";
const BUILD_MANIFEST_PROFILE_PREFIX: &str = "# profile: ";
const BUILD_MANIFEST_MANAGED_EXTENSIONS_PREFIX: &str = "# managed_extensions: ";

fn build_profile_manifest_name(profile: BuildProfile) -> &'static str {
    match profile {
        BuildProfile::Dev => "dev",
        BuildProfile::Release => "release",
    }
}

fn build_profile_from_manifest_name(raw_value: &str) -> Option<BuildProfile> {
    match raw_value.trim() {
        "dev" => Some(BuildProfile::Dev),
        "release" => Some(BuildProfile::Release),
        _ => None,
    }
}

// -------------------------
//  Manifest Load Types
// -------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PreparedOutputCleanup {
    manifest_read_result: Option<ManifestReadResult>,
}

pub(crate) struct OutputCleanupFinalization<'a> {
    pub(crate) output_root: &'a Path,
    pub(crate) manifest_destination: &'a Path,
    pub(crate) current_managed_artifact_paths: &'a HashSet<PathBuf>,
    pub(crate) current_explicit_directory_paths: &'a HashSet<PathBuf>,
    pub(crate) owner: OutputOwner,
    pub(crate) cleanup_policy: &'a CleanupPolicy,
    pub(crate) write_mode: WriteMode,
    pub(crate) string_table: &'a StringTable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ManifestReadResult {
    Uninitialised,
    Recoverable {
        reason: ManifestRecoveryReason,
    },
    RecoverableWithOwner {
        reason: ManifestRecoveryReason,
        owner: OutputOwner,
    },
    Valid(BuildManifest),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BuildManifest {
    pub(crate) owner: OutputOwner,
    pub(crate) managed_extensions: BTreeSet<String>,
    pub(crate) paths: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ManifestRecoveryReason {
    Missing,
    Unreadable,
    UnsupportedVersion,
    InvalidMetadata,
    ManagedExtensionsMismatch {
        manifest_extensions: BTreeSet<String>,
        active_extensions: BTreeSet<String>,
    },
}

impl ManifestRecoveryReason {
    fn describe(&self) -> String {
        match self {
            Self::Missing => String::from("build manifest is missing"),
            Self::Unreadable => String::from("build manifest is unreadable"),
            Self::UnsupportedVersion => String::from("build manifest version is unsupported"),
            Self::InvalidMetadata => String::from("build manifest metadata is invalid"),

            Self::ManagedExtensionsMismatch {
                manifest_extensions,
                active_extensions,
            } => format!(
                "build manifest managed extensions {} do not match active extensions {}",
                describe_extension_set(manifest_extensions),
                describe_extension_set(active_extensions)
            ),
        }
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct ManifestCleanupReport {
    pub(crate) removed_paths: Vec<PathBuf>,
    pub(crate) retained_paths: Vec<PathBuf>,
    pub(crate) ignored_paths: Vec<PathBuf>,
}

// -------------------------
//  Core Cleanup Workflow
// -------------------------

/// Prepare cleanup state before outputs are written.
///
/// WHAT: validates the output root and loads the previous manifest when cleanup is enabled.
/// WHY: cleanup decisions must be based on the pre-write state and never run on unsafe roots.
pub(crate) fn prepare_output_cleanup(
    output_root: &Path,
    project_root: Option<&Path>,
    entry_root: Option<&Path>,
    owner: OutputOwner,
    setting_location: &SourceLocation,
    cleanup_policy: &CleanupPolicy,
    string_table: &StringTable,
) -> Result<PreparedOutputCleanup, CompilerMessages> {
    let manifest_read_result = if let Some(project_root) = project_root {
        validate_output_root_is_safe(output_root, project_root, entry_root, string_table)?;
        let manifest_read_result = read_build_manifest(output_root, cleanup_policy);
        let existing_owner = match &manifest_read_result {
            ManifestReadResult::Valid(manifest) => Some(manifest.owner),
            ManifestReadResult::RecoverableWithOwner { owner, .. } => Some(*owner),
            ManifestReadResult::Uninitialised | ManifestReadResult::Recoverable { .. } => None,
        };
        if let Some(existing_owner) = existing_owner
            && existing_owner != owner
        {
            return Err(manifest_owner_conflict_messages(
                output_root,
                existing_owner,
                owner,
                setting_location,
                string_table,
            ));
        }
        Some(manifest_read_result)
    } else {
        None
    };

    Ok(PreparedOutputCleanup {
        manifest_read_result,
    })
}

/// Finalize cleanup after outputs are written.
///
/// WHAT: removes stale managed artifacts tracked by a valid manifest and writes the new manifest.
/// WHY: cleanup must compare the previous manifest against the outputs that were actually emitted
/// without inferring ownership from legacy route shapes or unsupported manifest formats.
pub(crate) fn finalize_output_cleanup(
    cleanup_state: &PreparedOutputCleanup,
    finalization: &OutputCleanupFinalization<'_>,
) -> Result<(), CompilerMessages> {
    let Some(manifest_read_result) = cleanup_state.manifest_read_result.as_ref() else {
        return Ok(());
    };

    let mut manifest_paths_to_write = finalization.current_managed_artifact_paths.clone();
    match manifest_read_result {
        ManifestReadResult::Uninitialised => {}
        ManifestReadResult::Valid(manifest) => {
            let cleanup_report = remove_manifest_tracked_stale_artifacts(
                finalization.output_root,
                finalization.current_managed_artifact_paths,
                finalization.current_explicit_directory_paths,
                &manifest.paths,
            );
            manifest_paths_to_write.extend(cleanup_report.retained_paths);
        }

        ManifestReadResult::Recoverable { reason }
        | ManifestReadResult::RecoverableWithOwner { reason, .. } => {
            emit_recoverable_manifest_warning(reason)
        }
    }

    write_build_manifest(
        finalization.manifest_destination,
        &manifest_paths_to_write,
        finalization.owner,
        finalization.cleanup_policy,
        finalization.write_mode,
        finalization.string_table,
    )
}

// -------------------------
//  Safety Validation
// -------------------------

/// Validate an output path before writing or deleting under the output root.
pub(crate) fn validate_relative_output_path(
    relative_output_path: &Path,
    string_table: &StringTable,
) -> Result<PathBuf, CompilerMessages> {
    if relative_output_path
        .to_str()
        .is_some_and(|path| path.contains(['\r', '\n']))
    {
        return Err(file_error_messages(
            relative_output_path,
            "Output paths cannot contain line-break characters.",
            string_table,
        ));
    }

    super::output_path::normalize_relative_path(relative_output_path).map_err(|reason| {
        let message = match reason {
            InvalidOutputFolderReason::Empty => "Output path cannot be empty for built artifacts.",
            InvalidOutputFolderReason::NonUtf8 => {
                "Output path must use valid UTF-8 portable path components."
            }
            InvalidOutputFolderReason::AbsolutePath => {
                "Output path must be relative, not absolute."
            }
            InvalidOutputFolderReason::ParentDirectorySegment => {
                "Output path cannot contain '..' traversal components."
            }
            InvalidOutputFolderReason::CurrentDirectory
            | InvalidOutputFolderReason::InvalidPathComponent
            | InvalidOutputFolderReason::RootOrPrefix
            | InvalidOutputFolderReason::InsideOrEqualToEntryRoot
            | InvalidOutputFolderReason::ResolvesOutsideProjectRoot => {
                "Output path must only contain normal portable path components."
            }
        };
        file_error_messages(relative_output_path, message, string_table)
    })
}

/// Reject output roots that are dangerous system paths or suspiciously far from the project.
///
/// WHY: stale artifact cleanup deletes files, so the output root must be validated before any
/// removal to prevent accidental deletion on system-critical or unrelated paths.
pub(crate) fn validate_output_root_is_safe(
    output_root: &Path,
    project_root: &Path,
    entry_root: Option<&Path>,
    string_table: &StringTable,
) -> Result<(), CompilerMessages> {
    // WHAT: Canonicalize the output root, falling back to the nearest existing ancestor.
    // WHY: Symlinks or relative segments could disguise a dangerous target path.
    let canonical_root = canonicalize_output_path(output_root).map_err(|_| {
        file_error_messages(
            output_root,
            "Build output root contains a dangling symlink component and cannot be used safely.",
            string_table,
        )
    })?;

    if is_dangerous_system_path(&canonical_root) {
        return Err(file_error_messages(
            output_root,
            format!(
                "Refusing to use '{}' as the build output root because it is a protected system path. \
                 Configure a project-relative output folder in config.moth.",
                output_root.display()
            ),
            string_table,
        ));
    }

    // Directory-project output roots use the same canonical containment classification as config
    // bootstrap. Single-file output retains the older adjacent-project boundary below because
    // its output root is chosen by the command rather than config.
    if let Some(entry_root) = entry_root {
        if let Err(reason) = super::policy::validate_directory_output_root_containment(
            output_root,
            project_root,
            Some(entry_root),
        ) {
            let message = match reason {
                InvalidOutputFolderReason::InsideOrEqualToEntryRoot => format!(
                    "Directory build output root '{}' must not resolve inside the source entry root '{}'.",
                    output_root.display(),
                    entry_root.display()
                ),
                InvalidOutputFolderReason::ResolvesOutsideProjectRoot => format!(
                    "Directory build output root '{}' must resolve strictly inside the project directory '{}'.",
                    output_root.display(),
                    project_root.display()
                ),
                _ => "Directory build output root failed canonical containment validation."
                    .to_owned(),
            };
            return Err(file_error_messages(output_root, message, string_table));
        }

        return Ok(());
    }

    // WHAT: Verify a single-file output root is near the project directory.
    // WHY: An output root in a completely unrelated location is likely a misconfiguration.
    let canonical_project = canonicalize_output_path(project_root).map_err(|_| {
        file_error_messages(
            project_root,
            "Project root contains a dangling symlink component and cannot validate output safety.",
            string_table,
        )
    })?;
    let project_parent = canonical_project.parent().unwrap_or(&canonical_project);

    let is_inside_project = canonical_root.starts_with(&canonical_project);
    let is_sibling_of_project = canonical_root.starts_with(project_parent);

    if !is_inside_project && !is_sibling_of_project {
        return Err(file_error_messages(
            output_root,
            format!(
                "Build output root '{}' is not inside or adjacent to the project directory '{}'. \
                 Stale artifact cleanup requires the output root to be near the project to prevent \
                 accidental file deletion.",
                output_root.display(),
                project_root.display()
            ),
            string_table,
        ));
    }

    Ok(())
}

// -------------------------
//  Manifest Persistence
// -------------------------

/// Read the build manifest from the output root without comparing its owner to the active build.
///
/// Missing, unreadable, or metadata-invalid manifests enter recoverable state. Path lines are
/// still revalidated individually so corrupt entries are skipped without broadening cleanup.
///
/// v4 manifests carry builder identity, profile and managed extensions. v3 manifests lack profile
/// identity, so they enter recoverable state and the v4 format is written after a successful build.
pub(crate) fn read_build_manifest(
    output_root: &Path,
    active_policy: &CleanupPolicy,
) -> ManifestReadResult {
    let manifest_path = output_root.join(BUILD_MANIFEST_FILENAME);
    let content = match fs::read(&manifest_path) {
        Ok(content) => content,

        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return if output_root.exists()
                && fs::read_dir(output_root).is_ok_and(|mut entries| entries.next().is_some())
            {
                ManifestReadResult::Recoverable {
                    reason: ManifestRecoveryReason::Missing,
                }
            } else {
                ManifestReadResult::Uninitialised
            };
        }

        Err(_) => {
            return ManifestReadResult::Recoverable {
                reason: ManifestRecoveryReason::Unreadable,
            };
        }
    };

    let mut manifest_lines = content
        .split(|byte| *byte == b'\n')
        .map(|line| std::str::from_utf8(line).map_err(|_| ()));

    let Some(first_line) = manifest_lines.next() else {
        return invalid_manifest_metadata();
    };
    let Ok(first_line) = first_line else {
        return ManifestReadResult::Recoverable {
            reason: ManifestRecoveryReason::Unreadable,
        };
    };
    let first_line = first_line.trim();

    if !first_line.starts_with('#') {
        return ManifestReadResult::Recoverable {
            reason: ManifestRecoveryReason::UnsupportedVersion,
        };
    }

    if first_line == BUILD_MANIFEST_HEADER_V4 {
        return read_v4_build_manifest(manifest_lines, active_policy);
    }

    if first_line == BUILD_MANIFEST_HEADER_V3 {
        // v3 manifests lack profile identity, so they enter recoverable state. The v4 format is
        // written after a successful build upgrades the manifest.
        return ManifestReadResult::Recoverable {
            reason: ManifestRecoveryReason::UnsupportedVersion,
        };
    }

    let reason = if first_line.starts_with(BUILD_MANIFEST_HEADER_PREFIX) {
        ManifestRecoveryReason::UnsupportedVersion
    } else {
        ManifestRecoveryReason::InvalidMetadata
    };
    ManifestReadResult::Recoverable { reason }
}

/// Write the build manifest listing all current managed artifact paths.
///
/// The v4 manifest records builder identity, build profile and managed extensions so future
/// cleanup can reject mismatched builder, profile or extension ownership safely.
pub(crate) fn write_build_manifest(
    manifest_destination: &Path,
    current_paths: &HashSet<PathBuf>,
    owner: OutputOwner,
    cleanup_policy: &CleanupPolicy,
    write_mode: WriteMode,
    string_table: &StringTable,
) -> Result<(), CompilerMessages> {
    if let Some(path) = current_paths
        .iter()
        .find(|path| !is_lossless_portable_relative_path(path))
    {
        return Err(file_error_messages(
            path,
            format!(
                "Managed output path '{}' cannot be represented safely in the build manifest.",
                path.display()
            ),
            string_table,
        ));
    }

    let mut sorted_paths: Vec<String> = current_paths
        .iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect();
    sorted_paths.sort();

    let mut manifest_lines = vec![
        String::from(BUILD_MANIFEST_HEADER_V4),
        format!(
            "{BUILD_MANIFEST_BUILDER_PREFIX}{}",
            owner.builder.manifest_name()
        ),
        format!(
            "{BUILD_MANIFEST_PROFILE_PREFIX}{}",
            build_profile_manifest_name(owner.profile)
        ),
        format!(
            "{BUILD_MANIFEST_MANAGED_EXTENSIONS_PREFIX}{}",
            cleanup_policy.manifest_extensions_csv()
        ),
    ];
    manifest_lines.extend(sorted_paths);

    let content = manifest_lines.join("\n");
    if should_skip_unchanged_write(manifest_destination, content.as_bytes(), write_mode) {
        return Ok(());
    }

    fs::write(manifest_destination, content).map_err(|error| {
        file_error_messages(
            manifest_destination,
            format!(
                "Failed to write build manifest '{}': {error}",
                manifest_destination.display()
            ),
            string_table,
        )
    })
}

// -------------------------
//  Stale Removal Logic
// -------------------------

/// Remove stale managed files tracked by the previous manifest.
///
/// WHAT: deletes stale manifest-tracked files after revalidating each relative path for safety.
/// WHY: v4 manifests are the supported ownership contract, so stale removal trusts the manifest's
/// emitted-path list instead of trying to infer ownership from extensions or route shapes.
pub(crate) fn remove_manifest_tracked_stale_artifacts(
    output_root: &Path,
    current_managed_artifact_paths: &HashSet<PathBuf>,
    current_explicit_directory_paths: &HashSet<PathBuf>,
    previous_manifest_paths: &[PathBuf],
) -> ManifestCleanupReport {
    let mut report = ManifestCleanupReport::default();
    let Ok(canonical_output_root) = canonicalize_output_path(output_root) else {
        return report;
    };

    for stale_relative in previous_manifest_paths {
        if current_managed_artifact_paths.contains(stale_relative) {
            continue;
        }

        if !is_lossless_portable_relative_path(stale_relative) {
            report.ignored_paths.push(stale_relative.clone());
            continue;
        }

        // Re-validate each manifest entry before deletion as defense against corrupted manifests.
        let Ok(normalized_stale_relative) =
            super::output_path::normalize_relative_path(stale_relative)
        else {
            report.ignored_paths.push(stale_relative.clone());
            continue;
        };

        if path_starts_with_component_identity(
            &normalized_stale_relative,
            Path::new(BUILD_MANIFEST_FILENAME),
        ) {
            report.ignored_paths.push(stale_relative.clone());
            continue;
        }

        if current_managed_artifact_paths.contains(&normalized_stale_relative) {
            continue;
        }

        let absolute_path = output_root.join(&normalized_stale_relative);

        // WHAT: Resolve the target before deletion.
        // WHY: stale cleanup must never follow a symlink outside the validated output root.
        let Ok(canonical_target) = canonicalize_output_path(&absolute_path) else {
            report.ignored_paths.push(stale_relative.clone());
            continue;
        };
        if !canonical_target.starts_with(&canonical_output_root) {
            report.ignored_paths.push(stale_relative.clone());
            continue;
        }

        let Ok(canonical_relative) = canonical_target.strip_prefix(&canonical_output_root) else {
            report.ignored_paths.push(stale_relative.clone());
            continue;
        };
        if current_managed_artifact_paths.contains(canonical_relative) {
            continue;
        }

        // A stale manifest path containing a symlink may have been retargeted since it was
        // emitted. Retaining it is safer than deleting a new target through the changed alias.
        let Ok(contains_symlink) =
            relative_path_contains_symlink_component(output_root, &normalized_stale_relative)
        else {
            report.ignored_paths.push(stale_relative.clone());
            continue;
        };
        if contains_symlink {
            report.ignored_paths.push(stale_relative.clone());
            continue;
        }

        let Ok(metadata) = fs::symlink_metadata(&absolute_path) else {
            continue;
        };
        if !metadata.file_type().is_file() {
            report.retained_paths.push(normalized_stale_relative);
            continue;
        }

        if let Err(error) = fs::remove_file(&absolute_path) {
            say!(
                Yellow "Warning: failed to remove stale artifact '",
                Yellow absolute_path.display(),
                Yellow "': ",
                Yellow error.to_string()
            );
            report
                .retained_paths
                .push(normalized_stale_relative.clone());
            continue;
        }

        remove_empty_parent_dirs(
            output_root,
            &absolute_path,
            current_explicit_directory_paths,
        );
        report.removed_paths.push(normalized_stale_relative);
    }

    report
}

// -------------------------
//  Internal Helpers
// -------------------------

fn parse_manifest_paths<'a, I>(lines: I) -> Result<Vec<PathBuf>, ()>
where
    I: IntoIterator<Item = Result<&'a str, ()>>,
{
    let mut paths = Vec::new();
    let mut path_identities = HashSet::new();
    for line in lines {
        let Ok(line) = line else {
            continue;
        };
        if line.is_empty() || line.contains(['\r', '\n']) {
            continue;
        }
        let path = PathBuf::from(line);
        if !is_lossless_portable_relative_path(&path) {
            continue;
        }
        let Ok(identity) = output_path_identity(&path) else {
            continue;
        };
        if let Ok(normalized_path) = super::output_path::normalize_relative_path(&path)
            && !path_starts_with_component_identity(
                &normalized_path,
                Path::new(BUILD_MANIFEST_FILENAME),
            )
        {
            if !path_identities.insert(identity) {
                return Err(());
            }
            paths.push(normalized_path);
        }
    }
    Ok(paths)
}

fn read_v4_build_manifest<'a, I>(
    mut manifest_lines: I,
    active_policy: &CleanupPolicy,
) -> ManifestReadResult
where
    I: Iterator<Item = Result<&'a str, ()>>,
{
    // 1. Parse builder kind.
    let Some(builder_line) = manifest_lines.next() else {
        return invalid_manifest_metadata();
    };
    let Ok(builder_line) = builder_line else {
        return ManifestReadResult::Recoverable {
            reason: ManifestRecoveryReason::Unreadable,
        };
    };
    let builder_line = builder_line.trim_end_matches('\r');
    let Some(raw_builder_kind) = builder_line.strip_prefix(BUILD_MANIFEST_BUILDER_PREFIX) else {
        return invalid_manifest_metadata();
    };
    let Some(manifest_builder_kind) = BuilderKind::from_manifest_name(raw_builder_kind) else {
        return invalid_manifest_metadata();
    };

    // 2. Parse build profile.
    let Some(profile_line) = manifest_lines.next() else {
        return invalid_manifest_metadata();
    };
    let Ok(profile_line) = profile_line else {
        return ManifestReadResult::Recoverable {
            reason: ManifestRecoveryReason::Unreadable,
        };
    };
    let profile_line = profile_line.trim_end_matches('\r');
    let Some(raw_profile) = profile_line.strip_prefix(BUILD_MANIFEST_PROFILE_PREFIX) else {
        return invalid_manifest_metadata();
    };
    let Some(manifest_profile) = build_profile_from_manifest_name(raw_profile) else {
        return invalid_manifest_metadata();
    };

    let owner = OutputOwner {
        builder: manifest_builder_kind,
        profile: manifest_profile,
    };

    // 3. Parse managed extensions (normalized so order, leading dot and ASCII case do not matter).
    let Some(managed_extensions_line) = manifest_lines.next() else {
        return invalid_manifest_metadata_with_owner(owner);
    };
    let Ok(managed_extensions_line) = managed_extensions_line else {
        return invalid_manifest_metadata_with_owner(owner);
    };
    let managed_extensions_line = managed_extensions_line.trim_end_matches('\r');
    let Some(raw_managed_extensions) =
        managed_extensions_line.strip_prefix(BUILD_MANIFEST_MANAGED_EXTENSIONS_PREFIX)
    else {
        return invalid_manifest_metadata_with_owner(owner);
    };
    let Some(manifest_managed_extensions) =
        parse_manifest_managed_extensions(raw_managed_extensions)
    else {
        return invalid_manifest_metadata_with_owner(owner);
    };

    // Require exact managed-extension ownership. Any set difference enters recoverable mode
    //    so stale files are preserved instead of being deleted under a mismatched ownership set.
    if manifest_managed_extensions != active_policy.managed_extensions {
        return ManifestReadResult::RecoverableWithOwner {
            reason: ManifestRecoveryReason::ManagedExtensionsMismatch {
                manifest_extensions: manifest_managed_extensions,
                active_extensions: active_policy.managed_extensions.clone(),
            },
            owner,
        };
    }

    let paths = match parse_manifest_paths(manifest_lines) {
        Ok(paths) => paths,
        Err(()) => return invalid_manifest_metadata_with_owner(owner),
    };

    ManifestReadResult::Valid(BuildManifest {
        owner,
        managed_extensions: active_policy.managed_extensions.clone(),
        paths,
    })
}

fn invalid_manifest_metadata() -> ManifestReadResult {
    ManifestReadResult::Recoverable {
        reason: ManifestRecoveryReason::InvalidMetadata,
    }
}

fn invalid_manifest_metadata_with_owner(owner: OutputOwner) -> ManifestReadResult {
    ManifestReadResult::RecoverableWithOwner {
        reason: ManifestRecoveryReason::InvalidMetadata,
        owner,
    }
}

fn parse_manifest_managed_extensions(raw_value: &str) -> Option<BTreeSet<String>> {
    if raw_value.trim().is_empty() {
        return None;
    }

    let mut managed_extensions = BTreeSet::new();
    for raw_extension in raw_value.split(',') {
        let trimmed_extension = raw_extension.trim();
        if trimmed_extension.is_empty() {
            return None;
        }
        managed_extensions.insert(normalize_managed_extension(trimmed_extension));
    }

    Some(managed_extensions)
}

/// Render a managed-extension set for a diagnostic description, using `(none)` for the empty set.
fn describe_extension_set(extensions: &BTreeSet<String>) -> String {
    if extensions.is_empty() {
        String::from("(none)")
    } else {
        join_extensions_csv(extensions)
    }
}

fn manifest_owner_conflict_messages(
    output_root: &Path,
    manifest_owner: OutputOwner,
    active_owner: OutputOwner,
    setting_location: &SourceLocation,
    string_table: &StringTable,
) -> CompilerMessages {
    let mut diagnostic_table = string_table.clone();
    let reason = InvalidConfigReason::OutputManifestOwnerConflict {
        output_root: diagnostic_table.intern(&output_root.to_string_lossy()),
        existing_builder: diagnostic_table.intern(manifest_owner.builder.manifest_name()),
        existing_profile: diagnostic_table
            .intern(build_profile_manifest_name(manifest_owner.profile)),
        active_builder: diagnostic_table.intern(active_owner.builder.manifest_name()),
        active_profile: diagnostic_table.intern(build_profile_manifest_name(active_owner.profile)),
    };
    let diagnostic =
        CompilerDiagnostic::invalid_config_reason(None, reason, setting_location.clone());
    CompilerMessages::from_diagnostic(diagnostic, diagnostic_table)
}

fn emit_recoverable_manifest_warning(reason: &ManifestRecoveryReason) {
    say!(Yellow format!(
        "Warning: full manifest-based stale cleanup was unavailable because {}. Cleanup entered recoverable mode; stale artifacts were preserved intentionally until a valid manifest is available.",
        reason.describe()
    ));
}

/// Walk from a removed file's parent directory upward toward the output root, removing each
/// directory if it is empty. Stops as soon as a removal fails (directory not empty) or the
/// output root is reached.
fn remove_empty_parent_dirs(
    output_root: &Path,
    removed_file: &Path,
    current_explicit_directory_paths: &HashSet<PathBuf>,
) {
    let mut current = match removed_file.parent() {
        Some(parent) => parent.to_path_buf(),
        None => return,
    };

    let Ok(output_root_canonical) = canonicalize_output_path(output_root) else {
        return;
    };

    while current != output_root {
        let Ok(current_canonical) = canonicalize_output_path(&current) else {
            break;
        };
        if current_canonical == output_root_canonical
            || !current_canonical.starts_with(&output_root_canonical)
        {
            break;
        }

        let Ok(current_relative) = current_canonical.strip_prefix(&output_root_canonical) else {
            break;
        };
        if current_explicit_directory_paths.contains(current_relative) {
            break;
        }

        if remove_empty_dir_if_safe(&current).is_err() {
            break;
        }

        current = match current.parent() {
            Some(parent) => parent.to_path_buf(),
            None => break,
        };
    }
}

/// Check whether a path matches a known dangerous system directory.
///
/// WHY: cleanup removes files, so it must never operate on OS-critical directories like `/usr`
/// or their platform equivalents.
fn is_dangerous_system_path(path: &Path) -> bool {
    let component_count = path.components().count();

    if component_count < 2 {
        return true;
    }

    #[cfg(unix)]
    {
        let path_str = path.to_string_lossy();
        let dangerous_unix_paths: &[&str] = &[
            "/usr", "/bin", "/sbin", "/etc", "/var", "/lib", "/boot", "/sys", "/proc", "/dev",
            "/home", "/tmp", "/opt", "/root", "/run", "/snap", "/srv",
        ];
        for dangerous in dangerous_unix_paths {
            if path_str == *dangerous || path_str.as_ref() == format!("{dangerous}/") {
                return true;
            }
        }
    }

    #[cfg(windows)]
    {
        let path_str = path.to_string_lossy().to_lowercase();
        let dangerous_windows_paths: &[&str] = &[
            r"c:\",
            r"c:\windows",
            r"c:\program files",
            r"c:\program files (x86)",
            r"c:\users",
            r"c:\system32",
        ];
        for dangerous in dangerous_windows_paths {
            if path_str == *dangerous || path_str == dangerous.trim_end_matches('\\') {
                return true;
            }
        }
    }

    false
}

/// Attempt to remove a directory only if it is empty. Returns `Ok(())` if removed, `Err`
/// otherwise.
fn remove_empty_dir_if_safe(path: &Path) -> io::Result<()> {
    fs::remove_dir(path)
}
