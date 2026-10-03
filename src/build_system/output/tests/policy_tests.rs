use super::super::output_path::{canonicalize_output_path, parse_relative_path};
use super::super::policy::{
    classify_output_folder, validate_directory_output_root_containment,
    validate_output_folder_containment,
};
use crate::build_system::output::output_path::{
    is_lossless_portable_relative_path, normalize_relative_path, output_path_component_identities,
    percent_encode_url_segment,
};
use crate::build_system::output::output_path_identity;
use crate::compiler_frontend::compiler_messages::InvalidOutputFolderReason;

use std::fs;
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
fn portable_parser_rejects_windows_ambiguous_components_on_every_host() {
    for path in [
        "page.js.",
        "page.js ",
        "CON",
        "con.txt",
        "NUL.dat",
        "COM1",
        "LPT9",
        "COM¹",
        "com¹",
        "CoM¹.txt",
        "LPT²",
        "lpt².txt",
        "LpT³.bin",
        "bad<name",
        "bad|name",
        "bad?name",
        "bad*name",
        "bad\"name",
        "bad\u{0}name",
    ] {
        assert_eq!(
            parse_relative_path(path),
            Err(InvalidOutputFolderReason::InvalidPathComponent),
            "{path:?} must be rejected before filesystem emission"
        );
    }
}

#[test]
fn output_path_percent_encoding_preserves_unreserved_bytes_and_escapes_utf8() {
    assert_eq!(
        percent_encode_url_segment("AZaz09-._~"),
        "AZaz09-._~",
        "RFC 3986 unreserved bytes remain readable"
    );
    assert_eq!(
        percent_encode_url_segment("space/%é"),
        "space%2F%25%C3%A9",
        "reserved and non-ASCII bytes use uppercase percent escapes"
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

#[cfg(unix)]
#[test]
fn config_and_write_time_containment_share_canonical_classification() {
    use std::os::unix::fs::symlink;

    use crate::build_system::output::manifest::validate_output_root_is_safe;
    use crate::compiler_frontend::symbols::string_interning::StringTable;
    use crate::compiler_tests::test_diagnostics::assert_output_rejection;

    let _project_temp = tempfile::tempdir().expect("should create project temp dir");
    let project_root = _project_temp.path().to_path_buf();
    let _outside_temp = tempfile::tempdir().expect("should create outside temp dir");
    let outside_root = _outside_temp.path().to_path_buf();
    fs::create_dir_all(&project_root).expect("should create project root");
    fs::create_dir_all(&outside_root).expect("should create outside root");
    symlink(&outside_root, project_root.join("out")).expect("should create output symlink");

    let folder = classify_output_folder(Path::new("out"), &project_root, None)
        .expect("lexical output folder should classify before symlink validation");
    let expected = Err(InvalidOutputFolderReason::ResolvesOutsideProjectRoot);
    assert_eq!(
        validate_output_folder_containment(&folder, &project_root, None),
        expected
    );
    assert_eq!(
        validate_directory_output_root_containment(&folder.resolved_path, &project_root, None,),
        expected
    );
    let messages = validate_output_root_is_safe(
        &folder.resolved_path,
        &project_root,
        Some(&project_root.join("src")),
        &StringTable::new(),
    )
    .expect_err("symlink output root should be rejected");
    assert_output_rejection(&messages, "output-root-not-inside-project");
}

// -------------------------
//  Filesystem Containment
// -------------------------

/// Config bootstrap validates outputs against an `fs::canonicalize` project root, which carries
/// the verbatim `\\?\` drive prefix on Windows. Ordinary project-local outputs must resolve inside
/// that root whether they are absent, nested or already present.
#[test]
fn canonical_project_root_accepts_local_output_roots() {
    let _project_temp = tempfile::tempdir().expect("should create project temp dir");
    let project_root = fs::canonicalize(_project_temp.path()).expect("project root should resolve");
    let entry_root = project_root.join("src");
    fs::create_dir(&entry_root).expect("should create entry root");
    fs::create_dir(project_root.join("existing")).expect("should create existing output");

    assert_eq!(
        canonicalize_output_path(&project_root.join("dev")),
        Ok(project_root.join("dev")),
        "an absent child of a canonical root must keep that root"
    );

    for relative in ["dev", "build/release", "existing"] {
        let folder = classify_output_folder(Path::new(relative), &project_root, Some(&entry_root))
            .expect("lexical output folder should classify");
        assert_eq!(
            validate_output_folder_containment(&folder, &project_root, Some(&entry_root)),
            Ok(()),
            "'{relative}' should resolve strictly inside the canonical project root"
        );
    }
}

#[test]
fn canonical_project_root_rejects_root_outside_and_entry_root_outputs() {
    let _project_temp = tempfile::tempdir().expect("should create project temp dir");
    let project_root = fs::canonicalize(_project_temp.path()).expect("project root should resolve");
    let entry_root = project_root.join("src");
    fs::create_dir(&entry_root).expect("should create entry root");
    let _outside_temp = tempfile::tempdir().expect("should create outside temp dir");
    let outside_root = fs::canonicalize(_outside_temp.path()).expect("outside root should resolve");

    for output_root in [project_root.clone(), outside_root.join("dev")] {
        assert_eq!(
            validate_directory_output_root_containment(&output_root, &project_root, None),
            Err(InvalidOutputFolderReason::ResolvesOutsideProjectRoot),
            "'{}' is not strictly inside the project root",
            output_root.display()
        );
    }

    for output_root in [entry_root.clone(), entry_root.join("dev")] {
        assert_eq!(
            validate_directory_output_root_containment(
                &output_root,
                &project_root,
                Some(&entry_root)
            ),
            Err(InvalidOutputFolderReason::InsideOrEqualToEntryRoot),
            "'{}' collides with the source entry root",
            output_root.display()
        );
    }
}

/// A dangling alias could become resolvable after an earlier output is emitted, so it must never
/// be mistaken for an ordinary absent output suffix.
#[cfg(unix)]
#[test]
fn dangling_output_root_component_is_rejected() {
    use super::super::output_path::DanglingSymlink;
    use std::os::unix::fs::symlink;

    let _project_temp = tempfile::tempdir().expect("should create project temp dir");
    let project_root = fs::canonicalize(_project_temp.path()).expect("project root should resolve");
    symlink(project_root.join("missing"), project_root.join("out"))
        .expect("should create dangling output alias");
    let output_root = project_root.join("out").join("dev");

    assert_eq!(canonicalize_output_path(&output_root), Err(DanglingSymlink));
    assert_eq!(
        validate_directory_output_root_containment(&output_root, &project_root, None),
        Err(InvalidOutputFolderReason::ResolvesOutsideProjectRoot)
    );
}

/// Windows spells one directory as either an ordinary or a verbatim drive path. Containment must
/// compare those spellings by filesystem identity in every combination.
#[cfg(windows)]
#[test]
fn windows_disk_and_verbatim_disk_roots_share_containment() {
    use crate::compiler_frontend::utilities::basic::normalize_path;
    use std::path::{Component, Prefix};

    fn prefix_kind(path: &Path) -> Option<Prefix<'_>> {
        match path.components().next() {
            Some(Component::Prefix(prefix)) => Some(prefix.kind()),
            _ => None,
        }
    }

    let _project_temp = tempfile::tempdir().expect("should create project temp dir");
    let verbatim_root =
        fs::canonicalize(_project_temp.path()).expect("project root should resolve");
    let disk_root = normalize_path(&verbatim_root);
    assert!(matches!(
        prefix_kind(&verbatim_root),
        Some(Prefix::VerbatimDisk(_))
    ));
    assert!(matches!(prefix_kind(&disk_root), Some(Prefix::Disk(_))));

    for (output_parent, project_root) in [
        (&verbatim_root, &verbatim_root),
        (&verbatim_root, &disk_root),
        (&disk_root, &verbatim_root),
        (&disk_root, &disk_root),
    ] {
        assert_eq!(
            validate_directory_output_root_containment(
                &output_parent.join("dev"),
                project_root,
                None
            ),
            Ok(()),
            "'{}' should be inside '{}'",
            output_parent.join("dev").display(),
            project_root.display()
        );
    }
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

#[cfg(any(unix, windows))]
#[test]
fn non_utf8_relative_paths_return_non_utf8_from_all_conversions() {
    use std::ffi::OsString;

    #[cfg(unix)]
    use std::os::unix::ffi::OsStringExt;
    #[cfg(windows)]
    use std::os::windows::ffi::OsStringExt;

    #[cfg(unix)]
    let path = PathBuf::from(OsString::from_vec(b"safe-\xFF-file.js".to_vec()));
    #[cfg(windows)]
    let path = PathBuf::from(OsString::from_wide(&[0xD800]));

    assert!(!is_lossless_portable_relative_path(&path));
    assert_eq!(
        output_path_identity(&path),
        Err(InvalidOutputFolderReason::NonUtf8)
    );
    assert_eq!(
        output_path_component_identities(&path),
        Err(InvalidOutputFolderReason::NonUtf8)
    );
    assert_eq!(
        normalize_relative_path(&path),
        Err(InvalidOutputFolderReason::NonUtf8)
    );
}
