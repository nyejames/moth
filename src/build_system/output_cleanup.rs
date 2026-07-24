//! Conservative manifest-backed cleanup for build outputs.
//!
//! WHAT: owns cleanup policy, manifest parsing/writing, output-root safety validation, and stale
//! artifact removal.
//! WHY: build orchestration should stay focused on compilation and file emission while cleanup
//! policy remains isolated behind one safety-first module.

use crate::build_system::build::WriteMode;
use crate::build_system::utils::{file_error_messages, should_skip_unchanged_write};
use crate::compiler_frontend::compiler_errors::CompilerMessages;
use crate::compiler_frontend::symbols::string_interning::StringTable;

use saying::say;

use std::collections::{BTreeSet, HashSet};
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

/// Manifest file written to the output root to track which managed build artifacts exist.
pub(crate) const BUILD_MANIFEST_FILENAME: &str = ".moth_manifest";
const BUILD_MANIFEST_HEADER_V3: &str = "# moth-manifest v3";
const BUILD_MANIFEST_HEADER_PREFIX: &str = "# moth-manifest ";
const BUILD_MANIFEST_BUILDER_PREFIX: &str = "# builder: ";
const BUILD_MANIFEST_MANAGED_EXTENSIONS_PREFIX: &str = "# managed_extensions: ";

// -------------------------
//  Cleanup Policy Types
// -------------------------

/// Identifies which builder owns a cleanup policy and whether builder-specific cleanup applies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuilderKind {
    Generic,
    Html,
}

impl BuilderKind {
    fn manifest_name(&self) -> &'static str {
        match self {
            Self::Generic => "generic",
            Self::Html => "html",
        }
    }

    fn from_manifest_name(raw_value: &str) -> Option<Self> {
        match raw_value.trim() {
            "generic" => Some(Self::Generic),
            "html" => Some(Self::Html),
            _ => None,
        }
    }
}

/// Builder-owned cleanup contract describing which file types may be deleted automatically.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanupPolicy {
    /// Distinguishes generic manifest cleanup from builder-specific ownership policies.
    pub builder_kind: BuilderKind,
    /// Extensions this builder is allowed to delete through manifest-backed cleanup.
    pub managed_extensions: BTreeSet<String>,
}

impl CleanupPolicy {
    /// Constructs a cleanup policy for builders that only want manifest-scoped cleanup.
    ///
    /// WHAT: stores the managed file extensions the builder owns.
    /// WHY: cleanup must be explicit about ownership instead of inferring it from the filesystem.
    pub fn generic<I, S>(managed_extensions: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        Self::new(BuilderKind::Generic, managed_extensions)
    }

    /// Constructs the cleanup policy for HTML builds.
    ///
    /// WHAT: HTML builds manage HTML, JS, and Wasm artifacts.
    /// WHY: stale cleanup should be limited to builder-owned route artifacts in this pass.
    pub fn html() -> Self {
        Self::new(BuilderKind::Html, [".html", ".js", ".wasm"])
    }

    fn new<I, S>(builder_kind: BuilderKind, managed_extensions: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        Self {
            builder_kind,
            managed_extensions: collect_managed_extensions(managed_extensions),
        }
    }

    pub(crate) fn manages_path(&self, path: &Path) -> bool {
        relative_path_extension(path)
            .is_some_and(|extension| self.managed_extensions.contains(extension.as_str()))
    }

    fn manifest_extensions_csv(&self) -> String {
        join_extensions_csv(&self.managed_extensions)
    }
}

// -------------------------
//  Manifest Load Types
// -------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PreparedOutputCleanup {
    manifest_load_result: Option<ManifestLoadResult>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ManifestLoadResult {
    Valid {
        paths: Vec<PathBuf>,
        builder_kind: BuilderKind,
    },
    LimitedSafeMode {
        reason: ManifestLimitedSafeModeReason,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ManifestLimitedSafeModeReason {
    Missing,
    Unreadable,
    UnsupportedVersion,
    InvalidMetadata,
    BuilderMismatch {
        manifest_builder_kind: BuilderKind,
        active_builder_kind: BuilderKind,
    },
    ManagedExtensionsMismatch {
        manifest_extensions: BTreeSet<String>,
        active_extensions: BTreeSet<String>,
    },
}

impl ManifestLimitedSafeModeReason {
    fn describe(&self) -> String {
        match self {
            Self::Missing => String::from("build manifest is missing"),
            Self::Unreadable => String::from("build manifest is unreadable"),
            Self::UnsupportedVersion => String::from("build manifest version is unsupported"),
            Self::InvalidMetadata => String::from("build manifest metadata is invalid"),

            Self::BuilderMismatch {
                manifest_builder_kind,
                active_builder_kind,
            } => format!(
                "build manifest builder '{}' does not match active builder '{}'",
                manifest_builder_kind.manifest_name(),
                active_builder_kind.manifest_name()
            ),

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
    removed_paths: Vec<PathBuf>,
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
    project_entry_dir: Option<&Path>,
    cleanup_policy: &CleanupPolicy,
    string_table: &StringTable,
) -> Result<PreparedOutputCleanup, CompilerMessages> {
    let manifest_load_result = if let Some(project_entry_dir) = project_entry_dir {
        validate_output_root_is_safe(output_root, project_entry_dir, string_table)?;
        Some(read_build_manifest(output_root, cleanup_policy))
    } else {
        None
    };

    Ok(PreparedOutputCleanup {
        manifest_load_result,
    })
}

/// Finalize cleanup after outputs are written.
///
/// WHAT: removes stale managed artifacts tracked by a valid manifest and writes the new manifest.
/// WHY: cleanup must be compare the previous manifest against the outputs that were actually emitted
/// without inferring ownership from legacy route shapes or unsupported manifest formats.
pub(crate) fn finalize_output_cleanup(
    cleanup_state: &PreparedOutputCleanup,
    output_root: &Path,
    current_managed_artifact_paths: &HashSet<PathBuf>,
    cleanup_policy: &CleanupPolicy,
    write_mode: WriteMode,
    string_table: &StringTable,
) -> Result<(), CompilerMessages> {
    let Some(manifest_load_result) = cleanup_state.manifest_load_result.as_ref() else {
        return Ok(());
    };

    match manifest_load_result {
        ManifestLoadResult::Valid { paths, .. } => {
            remove_manifest_tracked_stale_artifacts(
                output_root,
                current_managed_artifact_paths,
                paths,
            );
        }

        ManifestLoadResult::LimitedSafeMode { reason } => emit_limited_safe_mode_warning(reason),
    }

    write_build_manifest(
        output_root,
        current_managed_artifact_paths,
        cleanup_policy,
        write_mode,
        string_table,
    )
}

// -------------------------
//  Safety Validation
// -------------------------

/// Validate an output path before writing or deleting under the output root.
pub(crate) fn validate_relative_output_path(
    relative_output_path: &Path,
    string_table: &StringTable,
) -> Result<(), CompilerMessages> {
    if relative_output_path.as_os_str().is_empty() {
        return Err(file_error_messages(
            relative_output_path,
            "Output path cannot be empty for built artifacts.",
            string_table,
        ));
    }

    if relative_output_path.is_absolute() {
        return Err(file_error_messages(
            relative_output_path,
            "Output path must be relative, not absolute.",
            string_table,
        ));
    }

    for component in relative_output_path.components() {
        match component {
            Component::Normal(_) => {}

            Component::ParentDir => {
                return Err(file_error_messages(
                    relative_output_path,
                    "Output path cannot contain '..' traversal components.",
                    string_table,
                ));
            }

            Component::CurDir | Component::RootDir | Component::Prefix(_) => {
                return Err(file_error_messages(
                    relative_output_path,
                    "Output path must only contain normal path components.",
                    string_table,
                ));
            }
        }
    }

    Ok(())
}

/// Reject output roots that are dangerous system paths or suspiciously far from the project.
///
/// WHY: stale artifact cleanup deletes files, so the output root must be validated before any
/// removal to prevent accidental deletion on system-critical or unrelated paths.
pub(crate) fn validate_output_root_is_safe(
    output_root: &Path,
    project_entry_dir: &Path,
    string_table: &StringTable,
) -> Result<(), CompilerMessages> {
    // WHAT: Canonicalize the output root, falling back to the nearest existing ancestor.
    // WHY: Symlinks or relative segments could disguise a dangerous target path.
    let canonical_root = canonicalize_or_nearest_ancestor(output_root);

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

    // WHAT: Verify the output root is near the project directory.
    // WHY: An output root in a completely unrelated location is likely a misconfiguration.
    let canonical_project = canonicalize_or_nearest_ancestor(project_entry_dir);
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
                project_entry_dir.display()
            ),
            string_table,
        ));
    }

    Ok(())
}

// -------------------------
//  Manifest Persistence
// -------------------------

/// Read the build manifest from the output root and classify whether full cleanup is safe.
///
/// Missing, unreadable, or metadata-invalid manifests fall back to limited safe mode. Path lines
/// are still revalidated individually so corrupt entries are skipped without broadening cleanup.
pub(crate) fn read_build_manifest(
    output_root: &Path,
    active_policy: &CleanupPolicy,
) -> ManifestLoadResult {
    let manifest_path = output_root.join(BUILD_MANIFEST_FILENAME);
    let content = match fs::read_to_string(&manifest_path) {
        Ok(content) => content,

        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return ManifestLoadResult::LimitedSafeMode {
                reason: ManifestLimitedSafeModeReason::Missing,
            };
        }

        Err(_) => {
            return ManifestLoadResult::LimitedSafeMode {
                reason: ManifestLimitedSafeModeReason::Unreadable,
            };
        }
    };

    let mut non_empty_lines = content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty());

    let Some(first_line) = non_empty_lines.next() else {
        return invalid_manifest_metadata();
    };

    if !first_line.starts_with('#') {
        return ManifestLoadResult::LimitedSafeMode {
            reason: ManifestLimitedSafeModeReason::UnsupportedVersion,
        };
    }

    if first_line != BUILD_MANIFEST_HEADER_V3 {
        let reason = if first_line.starts_with(BUILD_MANIFEST_HEADER_PREFIX) {
            ManifestLimitedSafeModeReason::UnsupportedVersion
        } else {
            ManifestLimitedSafeModeReason::InvalidMetadata
        };
        return ManifestLoadResult::LimitedSafeMode { reason };
    }

    read_v3_build_manifest(non_empty_lines, active_policy)
}

/// Write the build manifest listing all current managed artifact paths.
///
/// The manifest records only managed file artifacts, with builder metadata kept in lightweight
/// headers so future cleanup can reject mismatched or unsupported manifests safely.
pub(crate) fn write_build_manifest(
    output_root: &Path,
    current_paths: &HashSet<PathBuf>,
    cleanup_policy: &CleanupPolicy,
    write_mode: WriteMode,
    string_table: &StringTable,
) -> Result<(), CompilerMessages> {
    let manifest_path = output_root.join(BUILD_MANIFEST_FILENAME);

    let mut sorted_paths: Vec<String> = current_paths
        .iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect();
    sorted_paths.sort();

    let mut manifest_lines = vec![
        String::from(BUILD_MANIFEST_HEADER_V3),
        format!(
            "{BUILD_MANIFEST_BUILDER_PREFIX}{}",
            cleanup_policy.builder_kind.manifest_name()
        ),
        format!(
            "{BUILD_MANIFEST_MANAGED_EXTENSIONS_PREFIX}{}",
            cleanup_policy.manifest_extensions_csv()
        ),
    ];
    manifest_lines.extend(sorted_paths);

    let content = manifest_lines.join("\n");
    if should_skip_unchanged_write(&manifest_path, content.as_bytes(), write_mode) {
        return Ok(());
    }

    fs::write(&manifest_path, content).map_err(|error| {
        file_error_messages(
            &manifest_path,
            format!(
                "Failed to write build manifest '{}': {error}",
                manifest_path.display()
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
/// WHY: v3 manifests are the supported ownership contract, so stale removal trusts the manifest's
/// emitted-path list instead of trying to infer ownership from extensions or route shapes.
pub(crate) fn remove_manifest_tracked_stale_artifacts(
    output_root: &Path,
    current_managed_artifact_paths: &HashSet<PathBuf>,
    previous_manifest_paths: &[PathBuf],
) -> ManifestCleanupReport {
    let canonical_output_root = canonicalize_or_nearest_ancestor(output_root);
    let mut report = ManifestCleanupReport::default();

    for stale_relative in previous_manifest_paths {
        if current_managed_artifact_paths.contains(stale_relative) {
            continue;
        }

        // Re-validate each manifest entry before deletion as defense against corrupted manifests.
        if !is_safe_relative_output_path(stale_relative) {
            continue;
        }

        let absolute_path = output_root.join(stale_relative);

        // WHAT: Resolve the target before deletion.
        // WHY: stale cleanup must never follow a symlink outside the validated output root.
        let canonical_target = canonicalize_or_nearest_ancestor(&absolute_path);
        if !canonical_target.starts_with(&canonical_output_root) {
            continue;
        }

        if absolute_path.is_file() {
            if let Err(error) = fs::remove_file(&absolute_path) {
                say!(
                    Yellow "Warning: failed to remove stale artifact '",
                    Yellow absolute_path.display(),
                    Yellow "': ",
                    Yellow error.to_string()
                );
                continue;
            }

            remove_empty_parent_dirs(output_root, &absolute_path);
            report.removed_paths.push(stale_relative.clone());
        }
    }

    report
}

// -------------------------
//  Internal Helpers
// -------------------------

fn parse_manifest_paths<'a, I>(lines: I) -> Vec<PathBuf>
where
    I: IntoIterator<Item = &'a str>,
{
    let mut paths = Vec::new();
    for line in lines {
        let path = PathBuf::from(line);
        if is_safe_relative_output_path(&path) {
            paths.push(path);
        }
    }
    paths
}

fn read_v3_build_manifest<'a, I>(
    mut manifest_lines: I,
    active_policy: &CleanupPolicy,
) -> ManifestLoadResult
where
    I: Iterator<Item = &'a str>,
{
    // 1. Parse builder kind
    let Some(builder_line) = manifest_lines.next() else {
        return invalid_manifest_metadata();
    };
    let Some(raw_builder_kind) = builder_line.strip_prefix(BUILD_MANIFEST_BUILDER_PREFIX) else {
        return invalid_manifest_metadata();
    };
    let Some(manifest_builder_kind) = BuilderKind::from_manifest_name(raw_builder_kind) else {
        return invalid_manifest_metadata();
    };

    // 2. Parse managed extensions (normalized so order, leading dot and ASCII case do not matter).
    let Some(managed_extensions_line) = manifest_lines.next() else {
        return invalid_manifest_metadata();
    };
    let Some(raw_managed_extensions) =
        managed_extensions_line.strip_prefix(BUILD_MANIFEST_MANAGED_EXTENSIONS_PREFIX)
    else {
        return invalid_manifest_metadata();
    };
    let Some(manifest_managed_extensions) =
        parse_manifest_managed_extensions(raw_managed_extensions)
    else {
        return invalid_manifest_metadata();
    };

    // 3. Verify builder ownership before extension ownership.
    if manifest_builder_kind != active_policy.builder_kind {
        return ManifestLoadResult::LimitedSafeMode {
            reason: ManifestLimitedSafeModeReason::BuilderMismatch {
                manifest_builder_kind,
                active_builder_kind: active_policy.builder_kind.clone(),
            },
        };
    }

    // 4. Require exact managed-extension ownership. Any set difference enters limited safe mode
    //    so stale files are preserved instead of being deleted under a mismatched ownership set.
    if manifest_managed_extensions != active_policy.managed_extensions {
        return ManifestLoadResult::LimitedSafeMode {
            reason: ManifestLimitedSafeModeReason::ManagedExtensionsMismatch {
                manifest_extensions: manifest_managed_extensions,
                active_extensions: active_policy.managed_extensions.clone(),
            },
        };
    }

    ManifestLoadResult::Valid {
        paths: parse_manifest_paths(manifest_lines),
        builder_kind: manifest_builder_kind,
    }
}

fn invalid_manifest_metadata() -> ManifestLoadResult {
    ManifestLoadResult::LimitedSafeMode {
        reason: ManifestLimitedSafeModeReason::InvalidMetadata,
    }
}

fn parse_manifest_managed_extensions(raw_value: &str) -> Option<BTreeSet<String>> {
    if raw_value.trim().is_empty() {
        return Some(BTreeSet::new());
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

fn collect_managed_extensions<I, S>(managed_extensions: I) -> BTreeSet<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    managed_extensions
        .into_iter()
        .map(|extension| normalize_managed_extension(extension.as_ref()))
        .collect()
}

fn normalize_managed_extension(raw_extension: &str) -> String {
    let trimmed_extension = raw_extension.trim();
    let dotted_extension = if trimmed_extension.starts_with('.') {
        trimmed_extension.to_owned()
    } else {
        format!(".{trimmed_extension}")
    };

    dotted_extension.to_ascii_lowercase()
}

/// Join a normalized managed-extension set into a deterministic CSV string.
///
/// WHAT: renders a `BTreeSet<String>` of extensions as a comma-separated list.
/// WHY: the manifest file format and the limited-safe-mode diagnostic both need a stable
/// rendering of an extension set, so the shared join lives here to avoid duplicating the logic.
fn join_extensions_csv(extensions: &BTreeSet<String>) -> String {
    extensions
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(",")
}

/// Render a managed-extension set for a diagnostic description, using `(none)` for the empty set.
fn describe_extension_set(extensions: &BTreeSet<String>) -> String {
    if extensions.is_empty() {
        String::from("(none)")
    } else {
        join_extensions_csv(extensions)
    }
}

fn relative_path_extension(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| format!(".{}", extension.to_ascii_lowercase()))
}

fn emit_limited_safe_mode_warning(reason: &ManifestLimitedSafeModeReason) {
    say!(Yellow format!(
        "Warning: full manifest-based stale cleanup was unavailable because {}. Cleanup ran in limited safe mode; stale artifacts were preserved intentionally until a valid manifest is available.",
        reason.describe()
    ));
}

fn is_safe_relative_output_path(relative_output_path: &Path) -> bool {
    if relative_output_path.as_os_str().is_empty() || relative_output_path.is_absolute() {
        return false;
    }

    relative_output_path
        .components()
        .all(|component| matches!(component, Component::Normal(_)))
}

/// Walk from a removed file's parent directory upward toward the output root, removing each
/// directory if it is empty. Stops as soon as a removal fails (directory not empty) or the
/// output root is reached.
fn remove_empty_parent_dirs(output_root: &Path, removed_file: &Path) {
    let mut current = match removed_file.parent() {
        Some(parent) => parent.to_path_buf(),
        None => return,
    };

    let output_root_canonical = canonicalize_or_nearest_ancestor(output_root);

    while current != output_root
        && canonicalize_or_nearest_ancestor(&current) != output_root_canonical
    {
        if remove_empty_dir_if_safe(&current).is_err() {
            break;
        }

        current = match current.parent() {
            Some(parent) => parent.to_path_buf(),
            None => break,
        };
    }
}

/// Canonicalize a path, falling back to the nearest existing ancestor if the path does not exist.
fn canonicalize_or_nearest_ancestor(path: &Path) -> PathBuf {
    if let Ok(canonical) = fs::canonicalize(path) {
        return canonical;
    }

    let mut ancestor = path.to_path_buf();
    while let Some(parent) = ancestor.parent() {
        if let Ok(canonical) = fs::canonicalize(parent) {
            let suffix = path.strip_prefix(parent).unwrap_or(Path::new(""));
            return canonical.join(suffix);
        }
        ancestor = parent.to_path_buf();
    }

    path.to_path_buf()
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
