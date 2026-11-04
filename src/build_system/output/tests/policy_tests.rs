use super::super::output_path::{
    canonicalize_output_path, normalize_directory_output_relative_path, parse_relative_path,
};
use super::super::output_roots::{DEFAULT_DEV_OUTPUT, DEFAULT_RELEASE_OUTPUT};
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

// -------------------------
//  Rejected Output Folders
// -------------------------

#[test]
fn classifier_rejects_empty_output_folder() {
    assert_eq!(
        classify_output_folder(Path::new(""), &project_root()),
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
        let result = classify_output_folder(path, &project_root());
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
        classify_output_folder(Path::new("."), &project_root()),
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
            classify_output_folder(path, &project_root()),
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
            classify_output_folder(path, &project_root()),
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
fn multi_component_output_without_hidden_directory_is_rejected() {
    for path in ["generated/site", "generated/next/site", "dist/site"] {
        assert_eq!(
            classify_output_folder(Path::new(path), &project_root()),
            Err(InvalidOutputFolderReason::VisibleDirectoryOutputPath),
            "{path:?} would be visible to source discovery"
        );
    }
}

// -------------------------
//  Hidden Output Normalization
// -------------------------

#[test]
fn single_component_output_names_receive_exactly_one_underscore() {
    for (authored, expected) in [
        ("dev", "_dev"),
        ("_dev", "_dev"),
        ("dist", "_dist"),
        ("artifacts", "_artifacts"),
        ("_cache", "_cache"),
        ("DEV", "_DEV"),
    ] {
        assert_eq!(
            normalize_directory_output_relative_path(authored),
            Ok(PathBuf::from(expected)),
            "'{authored}' must normalize to one hidden component"
        );
    }
}

#[test]
fn hidden_output_normalization_is_idempotent() {
    for authored in ["dev", "dist", "_cache"] {
        let first = normalize_directory_output_relative_path(authored)
            .expect("single-component output should normalize");
        let second = normalize_directory_output_relative_path(
            first.to_str().expect("normalized output stays UTF-8"),
        )
        .expect("normalized output should stay valid");
        assert_eq!(
            first, second,
            "'{authored}' normalization must not add prefixes"
        );
    }
}

#[test]
fn multi_component_output_keeps_spelling_when_hidden_component_exists() {
    for path in ["_generated/site", "generated/_site", "_a/_b"] {
        assert_eq!(
            normalize_directory_output_relative_path(path),
            Ok(PathBuf::from(path)),
            "'{path}' already contains a hidden directory component"
        );
    }
}

#[test]
fn default_output_roots_are_hidden() {
    assert_eq!(DEFAULT_DEV_OUTPUT, "_dev");
    assert_eq!(DEFAULT_RELEASE_OUTPUT, "_release");
    for default in [DEFAULT_DEV_OUTPUT, DEFAULT_RELEASE_OUTPUT] {
        assert_eq!(
            normalize_directory_output_relative_path(default),
            Ok(PathBuf::from(default))
        );
    }
}

#[test]
fn classifier_resolves_normalized_hidden_output_roots() {
    let dev = classify_output_folder(Path::new("dev"), &project_root())
        .expect("dev should normalize to a valid hidden root");
    assert_eq!(dev.relative_path, PathBuf::from("_dev"));
    assert_eq!(dev.resolved_path, PathBuf::from("/project/_dev"));

    let release = classify_output_folder(Path::new("release"), &project_root())
        .expect("release should normalize to a valid hidden root");
    assert_eq!(release.relative_path, PathBuf::from("_release"));
    assert_eq!(release.resolved_path, PathBuf::from("/project/_release"));
}

// -------------------------
//  Filesystem Containment
// -------------------------

/// Config bootstrap validates outputs against an `fs::canonicalize` project root, which carries
/// the verbatim `\\?\` drive prefix on Windows. Hidden project-local outputs must resolve inside
/// that root whether they are absent, nested or already present.
#[test]
fn canonical_project_root_accepts_hidden_local_output_roots() {
    let _project_temp = tempfile::tempdir().expect("should create project temp dir");
    let project_root = fs::canonicalize(_project_temp.path()).expect("project root should resolve");
    fs::create_dir(project_root.join("_existing")).expect("should create existing output");

    assert_eq!(
        canonicalize_output_path(&project_root.join("_dev")),
        Ok(project_root.join("_dev")),
        "an absent child of a canonical root must keep that root"
    );

    for relative in ["dev", "_generated/site", "generated/_site", "_existing"] {
        let folder = classify_output_folder(Path::new(relative), &project_root)
            .expect("lexical output folder should classify");
        assert_eq!(
            validate_output_folder_containment(&folder, &project_root),
            Ok(()),
            "'{relative}' should resolve strictly inside the canonical project root"
        );
    }
}

#[test]
fn canonical_project_root_accepts_hidden_output_inside_source_root() {
    let _project_temp = tempfile::tempdir().expect("should create project temp dir");
    let project_root = fs::canonicalize(_project_temp.path()).expect("project root should resolve");
    fs::create_dir(project_root.join("src")).expect("should create source root");

    let folder = classify_output_folder(Path::new("src/_dev"), &project_root)
        .expect("hidden output inside the source root should classify");
    assert_eq!(
        validate_output_folder_containment(&folder, &project_root),
        Ok(()),
        "hidden output containment inside the selected source root is legal"
    );
}

#[test]
fn canonical_project_root_rejects_root_outside_and_visible_outputs() {
    let _project_temp = tempfile::tempdir().expect("should create project temp dir");
    let project_root = fs::canonicalize(_project_temp.path()).expect("project root should resolve");
    let _outside_temp = tempfile::tempdir().expect("should create outside temp dir");
    let outside_root = fs::canonicalize(_outside_temp.path()).expect("outside root should resolve");

    for output_root in [project_root.clone(), outside_root.join("_dev")] {
        assert_eq!(
            validate_directory_output_root_containment(&output_root, &project_root),
            Err(InvalidOutputFolderReason::ResolvesOutsideProjectRoot),
            "'{}' is not strictly inside the project root",
            output_root.display()
        );
    }

    fs::create_dir(project_root.join("src")).expect("should create source root");
    fs::create_dir(project_root.join("src").join("dev")).expect("should create visible output");
    assert_eq!(
        validate_directory_output_root_containment(
            &project_root.join("src").join("dev"),
            &project_root
        ),
        Err(InvalidOutputFolderReason::VisibleDirectoryOutputPath),
        "a visible source-child output root must be rejected"
    );
}

#[cfg(unix)]
#[test]
fn visible_symlink_alias_to_outside_root_is_rejected() {
    use crate::build_system::output::manifest::validate_directory_output_root_safety;
    use crate::compiler_frontend::compiler_errors::{CompilationOutcome, InfrastructureErrors};
    use crate::compiler_frontend::compiler_messages::diagnostics::boundary_accumulator::BoundaryAccumulator;
    use crate::compiler_frontend::compiler_messages::diagnostics::freeze::InstallMode;
    use crate::compiler_tests::test_diagnostics::assert_output_rejection;
    use std::os::unix::fs::symlink;

    let _project_temp = tempfile::tempdir().expect("should create project temp dir");
    let project_root = _project_temp.path().to_path_buf();
    let _outside_temp = tempfile::tempdir().expect("should create outside temp dir");
    let outside_root = _outside_temp.path().to_path_buf();
    fs::create_dir_all(&outside_root).expect("should create outside root");
    symlink(&outside_root, project_root.join("_out")).expect("should create output symlink");

    let folder = classify_output_folder(Path::new("_out"), &project_root)
        .expect("lexical output folder should classify before symlink validation");
    let expected = Err(InvalidOutputFolderReason::ResolvesOutsideProjectRoot);
    assert_eq!(
        validate_output_folder_containment(&folder, &project_root),
        expected
    );
    assert_eq!(
        validate_directory_output_root_containment(&folder.resolved_path, &project_root),
        expected
    );
    let error = validate_directory_output_root_safety(&folder.resolved_path, &project_root)
        .expect_err("symlink output root should be rejected");
    let mut accumulator = BoundaryAccumulator::new();
    accumulator.append_infrastructure(Box::new(InfrastructureErrors::new(error)));
    let failure = accumulator
        .finish(InstallMode::Frozen)
        .map(CompilationOutcome::Diagnosed);
    assert_output_rejection(&failure, "output-root-not-inside-project");
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
    symlink(project_root.join("missing"), project_root.join("_out"))
        .expect("should create dangling output alias");
    let output_root = project_root.join("_out").join("_dev");

    assert_eq!(canonicalize_output_path(&output_root), Err(DanglingSymlink));
    assert_eq!(
        validate_directory_output_root_containment(&output_root, &project_root),
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
            validate_directory_output_root_containment(&output_parent.join("_dev"), project_root),
            Ok(()),
            "'{}' should be inside '{}'",
            output_parent.join("_dev").display(),
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
