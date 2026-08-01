//! Build-system output policy: profiles, builder identity, owners and the pure
//! output-folder classifier.
//!
//! WHAT: owns the single build-system `BuildProfile`, the stable `BuilderKind`
//! identity, the `OutputOwner` pair, and the pure classifier used by both config
//! diagnostics and output-plan construction.
//! WHY: profile and owner identity must exist once so output ownership never
//! drifts between CLI, the dev server and manifest persistence.

use crate::compiler_frontend::compiler_messages::InvalidOutputFolderReason;

use std::path::{Component, Path, PathBuf};

// -------------------------
//  Build Profile
// -------------------------

/// One build-system profile used for command policy.
///
/// WHAT: distinguishes development and release builds for output policy and the
/// HTML builder. Derived once from command flags through [`BuildProfile::from_flags`].
/// WHY: output roots, manifest ownership and builder codegen must agree on the
/// selected profile without each layer re-deriving it from `Flag::Release`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildProfile {
    Dev,
    Release,
}

impl BuildProfile {
    /// Select the build profile from the command flag slice.
    ///
    /// WHAT: maps the presence of `Flag::Release` to [`BuildProfile::Release`].
    /// WHY: this is the single profile-selection helper shared by output resolution,
    /// the HTML builder and manifest ownership.
    pub fn from_flags(flags: &[crate::compiler_frontend::Flag]) -> Self {
        if flags.contains(&crate::compiler_frontend::Flag::Release) {
            Self::Release
        } else {
            Self::Dev
        }
    }
}

// -------------------------
//  Output-Folder Classifier
// -------------------------

/// The outcome of classifying one directory output setting.
///
/// WHAT: reports either a validated project-relative folder with its resolved path,
/// or a concrete invalid reason.
/// WHY: config diagnostics and output-plan construction both consume this pure
/// result without re-running path checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutputFolderClassification {
    Valid {
        relative_path: PathBuf,
        resolved_path: PathBuf,
    },
    Invalid(InvalidOutputFolderReason),
}

/// Classify one directory output setting against the project boundary.
///
/// WHAT: validates that `relative` is a non-empty, relative, normal path that
/// resolves to a folder strictly inside the project root but outside the source
/// entry root.
/// WHY: directory output roots must be safe and distinct before any output writing
/// or cleanup runs. This pure classifier is shared by config diagnostics and plan
/// construction.
///
/// `resolved_entry_root` is `None` for the transitional empty or `.` entry root form,
/// where entry-root containment is not enforced and the output is validated against
/// the project root only.
pub(crate) fn classify_output_folder(
    relative: &Path,
    project_root: &Path,
    resolved_entry_root: Option<&Path>,
) -> OutputFolderClassification {
    if relative.as_os_str().is_empty() {
        return OutputFolderClassification::Invalid(InvalidOutputFolderReason::Empty);
    }

    let raw = relative.to_string_lossy();
    if relative.is_absolute() || raw.starts_with('/') {
        return OutputFolderClassification::Invalid(InvalidOutputFolderReason::AbsolutePath);
    }

    for component in relative.components() {
        match component {
            Component::Normal(_) => {}
            Component::ParentDir => {
                return OutputFolderClassification::Invalid(
                    InvalidOutputFolderReason::ParentDirectorySegment,
                );
            }
            Component::CurDir => {
                return OutputFolderClassification::Invalid(
                    InvalidOutputFolderReason::CurrentDirectory,
                );
            }
            Component::RootDir | Component::Prefix(_) => {
                return OutputFolderClassification::Invalid(
                    InvalidOutputFolderReason::RootOrPrefix,
                );
            }
        }
    }

    let resolved_path = project_root.join(relative);
    if resolved_path == project_root {
        return OutputFolderClassification::Invalid(InvalidOutputFolderReason::EqualsProjectRoot);
    }

    if let Some(entry_root) = resolved_entry_root
        && (resolved_path == entry_root || resolved_path.starts_with(entry_root))
    {
        return OutputFolderClassification::Invalid(
            InvalidOutputFolderReason::InsideOrEqualToEntryRoot,
        );
    }

    OutputFolderClassification::Valid {
        relative_path: relative.to_path_buf(),
        resolved_path,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler_frontend::compiler_messages::InvalidOutputFolderReason;

    fn project_root() -> PathBuf {
        PathBuf::from("/project")
    }

    fn entry_root() -> PathBuf {
        PathBuf::from("/project/src")
    }

    #[test]
    fn profile_selection_maps_release_flag_once() {
        use crate::compiler_frontend::Flag;
        assert_eq!(BuildProfile::from_flags(&[]), BuildProfile::Dev);
        assert_eq!(
            BuildProfile::from_flags(&[Flag::Release]),
            BuildProfile::Release
        );
        // Unrelated flags must not change profile selection.
        assert_eq!(
            BuildProfile::from_flags(&[Flag::HtmlWasm]),
            BuildProfile::Dev
        );
        assert_eq!(
            BuildProfile::from_flags(&[Flag::HtmlWasm, Flag::Release]),
            BuildProfile::Release
        );
    }

    #[test]
    fn classifier_rejects_empty_output_folder() {
        assert_eq!(
            classify_output_folder(Path::new(""), &project_root(), Some(&entry_root())),
            OutputFolderClassification::Invalid(InvalidOutputFolderReason::Empty)
        );
    }

    #[test]
    fn classifier_rejects_absolute_and_root_prefix_paths() {
        for path in [Path::new("/absolute"), Path::new("/")] {
            assert_eq!(
                classify_output_folder(path, &project_root(), Some(&entry_root())),
                OutputFolderClassification::Invalid(InvalidOutputFolderReason::AbsolutePath)
            );
        }
    }

    #[test]
    fn classifier_rejects_parent_and_current_directory_components() {
        assert_eq!(
            classify_output_folder(Path::new("../out"), &project_root(), Some(&entry_root())),
            OutputFolderClassification::Invalid(InvalidOutputFolderReason::ParentDirectorySegment)
        );
        assert_eq!(
            classify_output_folder(Path::new("./out"), &project_root(), Some(&entry_root())),
            OutputFolderClassification::Invalid(InvalidOutputFolderReason::CurrentDirectory)
        );
    }

    #[test]
    fn classifier_rejects_output_equal_to_explicit_entry_root() {
        assert_eq!(
            classify_output_folder(Path::new("src"), &project_root(), Some(&entry_root())),
            OutputFolderClassification::Invalid(
                InvalidOutputFolderReason::InsideOrEqualToEntryRoot
            )
        );
    }

    #[test]
    fn classifier_rejects_output_inside_explicit_entry_root() {
        assert_eq!(
            classify_output_folder(Path::new("src/deep"), &project_root(), Some(&entry_root())),
            OutputFolderClassification::Invalid(
                InvalidOutputFolderReason::InsideOrEqualToEntryRoot
            )
        );
    }

    #[test]
    fn classifier_accepts_distinct_valid_output_folders() {
        assert_eq!(
            classify_output_folder(Path::new("dev"), &project_root(), Some(&entry_root())),
            OutputFolderClassification::Valid {
                relative_path: PathBuf::from("dev"),
                resolved_path: PathBuf::from("/project/dev"),
            }
        );
        assert_eq!(
            classify_output_folder(Path::new("release"), &project_root(), Some(&entry_root())),
            OutputFolderClassification::Valid {
                relative_path: PathBuf::from("release"),
                resolved_path: PathBuf::from("/project/release"),
            }
        );
    }

    #[test]
    fn classifier_skips_entry_root_containment_in_transitional_root_form() {
        // Empty or "." entry root means the entry root covers the whole project, so a
        // project-relative output folder is validated only against the project root.
        assert_eq!(
            classify_output_folder(Path::new("dev"), &project_root(), None),
            OutputFolderClassification::Valid {
                relative_path: PathBuf::from("dev"),
                resolved_path: PathBuf::from("/project/dev"),
            }
        );
    }

    #[test]
    fn dev_and_release_roots_remain_distinct_after_normalisation() {
        let dev =
            match classify_output_folder(Path::new("dev"), &project_root(), Some(&entry_root())) {
                OutputFolderClassification::Valid { resolved_path, .. } => resolved_path,
                other => panic!("dev should be valid, got {other:?}"),
            };
        let release = match classify_output_folder(
            Path::new("release"),
            &project_root(),
            Some(&entry_root()),
        ) {
            OutputFolderClassification::Valid { resolved_path, .. } => resolved_path,
            other => panic!("release should be valid, got {other:?}"),
        };
        assert_ne!(dev, release);
    }
}
