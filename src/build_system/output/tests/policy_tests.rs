use super::super::policy::classify_output_folder;
use crate::build_system::BuildProfile;
use crate::build_system::output::output_path_identity;
use crate::compiler_frontend::Flag;
use crate::compiler_frontend::compiler_messages::InvalidOutputFolderReason;

use std::path::{Path, PathBuf};

fn project_root() -> PathBuf {
    PathBuf::from("/project")
}

fn entry_root() -> PathBuf {
    PathBuf::from("/project/src")
}

// -------------------------
//  Profile Selection
// -------------------------

#[test]
fn from_flags_maps_release_once() {
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
fn is_release_reports_the_selected_profile() {
    assert!(!BuildProfile::Dev.is_release());
    assert!(BuildProfile::Release.is_release());
}

// -------------------------
//  Rejected Output Folders
// -------------------------

#[test]
fn classifier_rejects_empty_output_folder() {
    assert_eq!(
        classify_output_folder(Path::new(""), &project_root(), Some(&entry_root())),
        Err(InvalidOutputFolderReason::Empty)
    );
}

#[test]
fn classifier_rejects_absolute_paths() {
    assert_eq!(
        classify_output_folder(Path::new("/absolute"), &project_root(), Some(&entry_root())),
        Err(InvalidOutputFolderReason::AbsolutePath)
    );
}

#[cfg(windows)]
#[test]
fn classifier_rejects_windows_drive_relative_prefix() {
    // A drive-relative prefix such as `C:output` is neither absolute nor a plain relative path.
    // It surfaces as a `Prefix` component and must be rejected as a platform-prefix path.
    assert_eq!(
        classify_output_folder(Path::new("C:output"), &project_root(), Some(&entry_root())),
        Err(InvalidOutputFolderReason::RootOrPrefix)
    );
}

#[test]
fn classifier_rejects_parent_directory_segment() {
    assert_eq!(
        classify_output_folder(Path::new("../out"), &project_root(), Some(&entry_root())),
        Err(InvalidOutputFolderReason::ParentDirectorySegment)
    );
}

#[test]
fn classifier_rejects_leading_cur_dir_segment() {
    assert_eq!(
        classify_output_folder(Path::new("./out"), &project_root(), Some(&entry_root())),
        Err(InvalidOutputFolderReason::CurrentDirectory)
    );
}

#[test]
fn classifier_rejects_nested_cur_dir_segment() {
    assert_eq!(
        classify_output_folder(
            Path::new("nested/./out"),
            &project_root(),
            Some(&entry_root())
        ),
        Err(InvalidOutputFolderReason::CurrentDirectory)
    );
}

#[test]
fn classifier_rejects_trailing_cur_dir_segment() {
    assert_eq!(
        classify_output_folder(Path::new("out/."), &project_root(), Some(&entry_root())),
        Err(InvalidOutputFolderReason::CurrentDirectory)
    );
}

#[test]
fn classifier_rejects_output_equal_to_explicit_entry_root() {
    assert_eq!(
        classify_output_folder(Path::new("src"), &project_root(), Some(&entry_root())),
        Err(InvalidOutputFolderReason::InsideOrEqualToEntryRoot)
    );
}

#[test]
fn classifier_rejects_output_inside_explicit_entry_root() {
    assert_eq!(
        classify_output_folder(Path::new("src/deep"), &project_root(), Some(&entry_root())),
        Err(InvalidOutputFolderReason::InsideOrEqualToEntryRoot)
    );
}

// -------------------------
//  Accepted Output Folders
// -------------------------

#[test]
fn classifier_accepts_distinct_valid_output_folders() {
    let dev = classify_output_folder(Path::new("dev"), &project_root(), Some(&entry_root()))
        .expect("dev should be valid");
    assert_eq!(dev.relative_path, PathBuf::from("dev"));
    assert_eq!(dev.resolved_path, PathBuf::from("/project/dev"));

    let release =
        classify_output_folder(Path::new("release"), &project_root(), Some(&entry_root()))
            .expect("release should be valid");
    assert_eq!(release.relative_path, PathBuf::from("release"));
    assert_eq!(release.resolved_path, PathBuf::from("/project/release"));
}

#[test]
fn classifier_skips_entry_root_containment_in_transitional_root_form() {
    // Empty or "." entry root means the entry root covers the whole project, so a project-relative
    // output folder is validated only against the project root.
    let dev = classify_output_folder(Path::new("dev"), &project_root(), None)
        .expect("dev should be valid in transitional root form");
    assert_eq!(dev.relative_path, PathBuf::from("dev"));
    assert_eq!(dev.resolved_path, PathBuf::from("/project/dev"));
}

// -------------------------
//  Output-Path Identity Pairs
// -------------------------

#[test]
fn case_only_variant_roots_share_an_output_identity() {
    let dev = output_path_identity(Path::new("/project/dev"));
    let dev_upper = output_path_identity(Path::new("/project/DEV"));
    assert_eq!(
        dev, dev_upper,
        "case-only variants must be one output identity"
    );
}

#[test]
fn nested_and_trailing_cur_dir_roots_share_an_output_identity() {
    let plain = output_path_identity(Path::new("/project/out"));
    let with_cur_dirs = output_path_identity(Path::new("/project/./out/."));
    assert_eq!(plain, with_cur_dirs);
}

#[test]
fn distinct_valid_roots_have_distinct_output_identities() {
    let dev = output_path_identity(Path::new("/project/dev"));
    let release = output_path_identity(Path::new("/project/release"));
    assert_ne!(dev, release);
}
