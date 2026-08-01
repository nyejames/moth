use super::super::policy::classify_output_folder;
use crate::build_system::output::output_path_identity;
use crate::compiler_frontend::compiler_messages::InvalidOutputFolderReason;

use std::path::{Path, PathBuf};

fn project_root() -> PathBuf {
    PathBuf::from("/project")
}

fn entry_root() -> PathBuf {
    PathBuf::from("/project/src")
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
fn classifier_rejects_rooted_paths_on_every_host() {
    for path in [
        Path::new("/absolute"),
        Path::new("\\rooted"),
        Path::new("\\\\server\\share"),
        Path::new("C:output"),
        Path::new("C:\\output"),
    ] {
        let result = classify_output_folder(path, &project_root(), Some(&entry_root()));
        let reason = match result {
            Err(reason) => reason,
            Ok(valid) => panic!("{path:?} should be rejected, got {valid:?}"),
        };
        assert!(
            matches!(
                reason,
                InvalidOutputFolderReason::AbsolutePath | InvalidOutputFolderReason::RootOrPrefix
            ),
            "{path:?} should be rejected as absolute or root/prefix, got {reason:?}"
        );
    }
}

#[test]
fn classifier_rejects_plain_cur_dir() {
    assert_eq!(
        classify_output_folder(Path::new("."), &project_root(), Some(&entry_root())),
        Err(InvalidOutputFolderReason::CurrentDirectory)
    );
}

#[test]
fn classifier_rejects_parent_directory_segments() {
    for path in [
        Path::new(".."),
        Path::new("../output"),
        Path::new("nested/../output"),
    ] {
        assert_eq!(
            classify_output_folder(path, &project_root(), Some(&entry_root())),
            Err(InvalidOutputFolderReason::ParentDirectorySegment),
            "{path:?} should be rejected"
        );
    }
}

#[test]
fn classifier_rejects_cur_dir_segments_anywhere() {
    for path in [
        Path::new("./output"),
        Path::new("nested/./output"),
        Path::new("output/."),
    ] {
        assert_eq!(
            classify_output_folder(path, &project_root(), Some(&entry_root())),
            Err(InvalidOutputFolderReason::CurrentDirectory),
            "{path:?} should be rejected"
        );
    }
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
    let dev = output_path_identity(Path::new("dev")).expect("dev is a valid relative path");
    let dev_upper = output_path_identity(Path::new("DEV")).expect("DEV is a valid relative path");
    assert_eq!(
        dev, dev_upper,
        "case-only variants must be one output identity"
    );
}

#[test]
fn separator_variant_roots_share_an_output_identity() {
    let slash = output_path_identity(Path::new("nested/output"))
        .expect("nested/output is a valid relative path");
    let backslash = output_path_identity(Path::new("nested\\output"))
        .expect("nested\\output is a valid relative path");
    assert_eq!(
        slash, backslash,
        "separator variants must be one output identity"
    );
}

#[test]
fn distinct_valid_roots_have_distinct_output_identities() {
    let dev = output_path_identity(Path::new("dev")).expect("dev is a valid relative path");
    let release =
        output_path_identity(Path::new("release")).expect("release is a valid relative path");
    assert_ne!(dev, release);
}
