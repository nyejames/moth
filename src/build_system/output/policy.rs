//! Build-system output policy: builder/profile ownership, cleanup scope, and output-folder plans.
//!
//! WHAT: owns the output owner, managed-extension policy, validated folder plans, and the pure
//! classifier shared by config diagnostics and output planning.
//! WHY: output identity, cleanup scope, and directory roots must be decided once so config
//! diagnostics, build orchestration, and manifest persistence agree on the same result.

use crate::build_system::build_profile::BuildProfile;
use crate::build_system::output::output_path::{canonicalize_output_path, normalize_relative_path};
use crate::compiler_frontend::compiler_errors::SourceLocation;
use crate::compiler_frontend::compiler_messages::InvalidOutputFolderReason;
use crate::compiler_frontend::utilities::basic::normalize_path;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

// -------------------------
//  Output Ownership
// -------------------------

/// Identifies the production artefact builder that owns an output manifest.
///
/// WHAT: keeps manifest ownership closed over the builders that can produce project artefacts.
/// WHY: a generic production fallback would let an unrelated builder claim an existing manifest
/// without an explicit identity change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuilderKind {
    Html,
    #[cfg(test)]
    Test,
}

impl BuilderKind {
    pub(crate) fn manifest_name(self) -> &'static str {
        match self {
            Self::Html => "html",
            #[cfg(test)]
            Self::Test => "test",
        }
    }

    pub(crate) fn from_manifest_name(raw_value: &str) -> Option<Self> {
        match raw_value.trim() {
            "html" => Some(Self::Html),
            #[cfg(test)]
            "test" => Some(Self::Test),
            _ => None,
        }
    }
}

/// The one builder/profile pair that owns a directory output root.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputOwner {
    pub builder: BuilderKind,
    pub profile: BuildProfile,
}

/// Builder-owned deletion scope for manifest-backed stale cleanup.
///
/// WHAT: records only the managed extensions that cleanup may remove.
/// WHY: builder identity and build profile belong to [`OutputOwner`] and must not be copied into
/// a second policy object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanupPolicy {
    pub(crate) managed_extensions: BTreeSet<String>,
}

impl CleanupPolicy {
    /// Constructs a cleanup policy for a test or non-HTML output record owner.
    pub fn generic<I, S>(managed_extensions: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        Self::new(managed_extensions)
    }

    /// Constructs the cleanup policy for HTML builds.
    pub fn html() -> Self {
        Self::new([".html", ".js", ".wasm"])
    }

    fn new<I, S>(managed_extensions: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        Self {
            managed_extensions: collect_managed_extensions(managed_extensions),
        }
    }

    pub(crate) fn manages_path(&self, path: &Path) -> bool {
        relative_path_extension(path)
            .is_some_and(|extension| self.managed_extensions.contains(extension.as_str()))
    }

    pub(crate) fn manifest_extensions_csv(&self) -> String {
        join_extensions_csv(&self.managed_extensions)
    }
}

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
    pub location: SourceLocation,
}

/// Validated development and release output settings produced during bootstrap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedDirectoryOutputSettings {
    pub dev: ValidatedOutputFolder,
    pub release: ValidatedOutputFolder,
}

impl ValidatedDirectoryOutputSettings {
    /// Select the output root and ownership facts for one command profile.
    pub(crate) fn select(
        &self,
        project_root: PathBuf,
        entry_root: PathBuf,
        owner: OutputOwner,
    ) -> ValidatedOutputPlan {
        let folder = match owner.profile {
            BuildProfile::Dev => &self.dev,
            BuildProfile::Release => &self.release,
        };

        ValidatedOutputPlan {
            output_root: folder.resolved_path.clone(),
            project_root,
            entry_root,
            owner,
            setting_location: folder.location.clone(),
        }
    }
}

/// Complete output plan for a validated directory project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedOutputPlan {
    pub output_root: PathBuf,
    pub project_root: PathBuf,
    pub entry_root: PathBuf,
    pub owner: OutputOwner,
    pub setting_location: SourceLocation,
}

/// Explicit output plan for a single-file command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SingleFileOutputPlan {
    pub output_root: PathBuf,
    pub project_root: Option<PathBuf>,
    pub owner: OutputOwner,
    pub setting_location: SourceLocation,
}

/// The output plan consumed by the writer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutputPlan {
    Directory(ValidatedOutputPlan),
    SingleFile(SingleFileOutputPlan),
}

impl OutputPlan {
    pub(crate) fn output_root(&self) -> &Path {
        match self {
            Self::Directory(plan) => &plan.output_root,
            Self::SingleFile(plan) => &plan.output_root,
        }
    }

    pub(crate) fn project_root(&self) -> Option<&Path> {
        match self {
            Self::Directory(plan) => Some(&plan.project_root),
            Self::SingleFile(plan) => plan.project_root.as_deref(),
        }
    }

    pub(crate) fn entry_root(&self) -> Option<&Path> {
        match self {
            Self::Directory(plan) => Some(&plan.entry_root),
            Self::SingleFile(_) => None,
        }
    }

    pub(crate) fn owner(&self) -> OutputOwner {
        match self {
            Self::Directory(plan) => plan.owner,
            Self::SingleFile(plan) => plan.owner,
        }
    }

    pub(crate) fn setting_location(&self) -> &SourceLocation {
        match self {
            Self::Directory(plan) => &plan.setting_location,
            Self::SingleFile(plan) => &plan.setting_location,
        }
    }
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
    let relative_path = normalize_relative_path(relative)?;

    let resolved_path = project_root.join(&relative_path);
    if let Some(entry_root) = resolved_entry_root
        && (resolved_path == entry_root || resolved_path.starts_with(entry_root))
    {
        return Err(InvalidOutputFolderReason::InsideOrEqualToEntryRoot);
    }

    Ok(ValidatedOutputFolder {
        relative_path,
        resolved_path,
        location: SourceLocation::default(),
    })
}

/// Validate the filesystem target of a lexically valid directory output folder.
///
/// WHAT: follows existing symlink components conservatively and checks the resolved output root
/// against the project and strict source-entry boundaries.
/// WHY: lexical config validation cannot see a symlink that redirects an apparently project-local
/// folder. Bootstrap, `check`, the dev server and the writer must reject that target consistently.
pub(crate) fn validate_output_folder_containment(
    folder: &ValidatedOutputFolder,
    project_root: &Path,
    resolved_entry_root: Option<&Path>,
) -> Result<(), InvalidOutputFolderReason> {
    validate_directory_output_root_containment(
        &folder.resolved_path,
        project_root,
        resolved_entry_root,
    )
}

/// Resolve a validated directory output root for physical-identity comparisons.
///
/// WHAT: returns the canonical filesystem target used to compare development and release roots.
/// WHY: lexical portable identities cannot detect two configured symlink aliases that target one
/// physical output directory. Containment and distinctness must share this output-policy owner.
pub(crate) fn canonical_output_root_for_identity(
    output_root: &Path,
) -> Result<PathBuf, InvalidOutputFolderReason> {
    canonicalize_output_path(output_root)
        .map(|canonical| normalize_path(&canonical))
        .map_err(|_| InvalidOutputFolderReason::ResolvesOutsideProjectRoot)
}

/// Validate a directory output root after resolving its filesystem target.
///
/// WHAT: owns the canonical project and source-entry containment rule shared by config bootstrap
/// and write-time output-root revalidation.
/// WHY: symlink resolution must produce one safety classification for every directory-output lane.
pub(crate) fn validate_directory_output_root_containment(
    output_root: &Path,
    project_root: &Path,
    resolved_entry_root: Option<&Path>,
) -> Result<(), InvalidOutputFolderReason> {
    let canonical_output_root = canonical_output_root_for_identity(output_root)?;
    let canonical_project_root = canonicalize_output_path(project_root)
        .map(|canonical| normalize_path(&canonical))
        .map_err(|_| InvalidOutputFolderReason::ResolvesOutsideProjectRoot)?;

    if canonical_output_root == canonical_project_root
        || !canonical_output_root.starts_with(&canonical_project_root)
    {
        return Err(InvalidOutputFolderReason::ResolvesOutsideProjectRoot);
    }

    if let Some(entry_root) = resolved_entry_root {
        let canonical_entry_root = canonicalize_output_path(entry_root)
            .map(|canonical| normalize_path(&canonical))
            .map_err(|_| InvalidOutputFolderReason::ResolvesOutsideProjectRoot)?;
        if canonical_entry_root != canonical_project_root
            && (canonical_output_root == canonical_entry_root
                || canonical_output_root.starts_with(&canonical_entry_root))
        {
            return Err(InvalidOutputFolderReason::InsideOrEqualToEntryRoot);
        }
    }

    Ok(())
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

pub(crate) fn normalize_managed_extension(raw_extension: &str) -> String {
    let trimmed_extension = raw_extension.trim();
    let dotted_extension = if trimmed_extension.starts_with('.') {
        trimmed_extension.to_owned()
    } else {
        format!(".{trimmed_extension}")
    };

    dotted_extension.to_ascii_lowercase()
}

fn relative_path_extension(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| format!(".{}", extension.to_ascii_lowercase()))
}

pub(crate) fn join_extensions_csv(extensions: &BTreeSet<String>) -> String {
    extensions
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(",")
}
