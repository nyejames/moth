#[cfg(windows)]
use super::super::manifest::{
    WindowsSystemLocations, is_dangerous_system_path, is_dangerous_system_path_with_locations,
};
use super::super::manifest::{
    validate_directory_output_root_safety, validate_single_file_output_root_safety,
};
use super::super::output_path::{
    canonicalize_output_path, normalize_directory_output_relative_path, parse_native_relative_path,
    parse_relative_path,
};
use super::super::output_roots::{DEFAULT_DEV_OUTPUT, DEFAULT_RELEASE_OUTPUT};
use super::super::policy::{
    classify_output_folder, validate_directory_output_root_containment,
    validate_output_folder_containment,
};
use crate::build_system::output::OutputRejectionReason;
use crate::build_system::output::output_path::{
    is_lossless_portable_relative_path, normalize_relative_path, output_path_component_identities,
    percent_encode_url_segment,
};
use crate::build_system::output::output_path_identity;
use crate::compiler_frontend::compiler_errors::{
    InfrastructureDetail, InfrastructureFailure, InfrastructureFailureKind,
};
use crate::compiler_frontend::compiler_messages::{CompilerStage, InvalidOutputFolderReason};

use std::fs;
use std::path::{Path, PathBuf};

fn project_root() -> PathBuf {
    PathBuf::from("/project")
}

/// Assert one failure came from the shared output-root danger guard, not a later containment rule.
#[track_caller]
fn assert_dangerous_rejection(failure: &InfrastructureFailure, output_root: &Path) {
    assert_eq!(failure.kind, InfrastructureFailureKind::Filesystem);
    assert_eq!(failure.stage, CompilerStage::Output);
    assert_eq!(failure.host_path.as_deref(), Some(output_root));

    let expected_reason = InfrastructureDetail::OutputRejectionReason(
        OutputRejectionReason::OutputRootDangerousSystemPath
            .as_metadata_value()
            .into(),
    );
    assert_eq!(
        failure.details.as_ref(),
        [expected_reason].as_slice(),
        "the typed rejection reason must identify the dangerous system path"
    );
}

/// Exercise every output lane that shares `reject_dangerous_output_root`.
///
/// WHY: both lanes canonicalise first, so an ordinary containment rejection could otherwise mask
/// a bypassed danger guard.
#[track_caller]
fn assert_dangerous_system_path_rejection(output_root: &Path, project_root: &Path) {
    let directory_failure = validate_directory_output_root_safety(output_root, project_root)
        .expect_err("a protected system path must never become a directory output root");
    assert_dangerous_rejection(&directory_failure, output_root);

    let single_file_failure = validate_single_file_output_root_safety(output_root, project_root)
        .expect_err("a protected system path must never become a single-file output root");
    assert_dangerous_rejection(&single_file_failure, output_root);
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
    use crate::compiler_frontend::utilities::basic::normalize_windows_verbatim_path;
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
    let disk_root = normalize_windows_verbatim_path(&verbatim_root);
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
//  Dangerous System Roots
// -------------------------

/// Canonical Unix spellings and trailing separators must reach the typed danger rejection.
///
/// `/bin` and `/sbin` are symlinks into `/usr` on usr-merged Linux; the guard compares both the
/// literal and canonical protected roots, so this remains true on either host layout.
#[cfg(unix)]
#[test]
fn unix_dangerous_system_output_roots_are_rejected_with_typed_reason() {
    let project_temp = tempfile::tempdir().expect("should create project temp dir");
    let project_root = project_temp.path();

    for root in [
        Path::new("/"),
        Path::new("/usr"),
        Path::new("/usr/"),
        Path::new("/bin"),
        Path::new("/sbin"),
        Path::new("/etc"),
        Path::new("/etc/"),
        Path::new("/var"),
        Path::new("/tmp"),
        Path::new("/tmp/"),
    ] {
        assert_dangerous_system_path_rejection(root, project_root);
    }
}

/// macOS canonicalises `/etc`, `/tmp` and `/var` through `/private`, so those aliases must name
/// the protected canonical roots rather than relying on the authored spelling.
#[cfg(target_os = "macos")]
#[test]
fn macos_canonical_system_root_aliases_are_rejected_with_typed_reason() {
    let project_temp = tempfile::tempdir().expect("should create project temp dir");
    let project_root = project_temp.path();

    for (alias, canonical) in [
        ("/etc", "/private/etc"),
        ("/tmp", "/private/tmp"),
        ("/var", "/private/var"),
        ("/private/etc", "/private/etc"),
        ("/private/tmp", "/private/tmp"),
        ("/private/var", "/private/var"),
    ] {
        assert_eq!(
            fs::canonicalize(alias).expect("macOS system alias should resolve"),
            Path::new(canonical),
            "'{alias}' must canonicalise to its protected root"
        );
        assert_dangerous_system_path_rejection(Path::new(alias), project_root);
    }
}

/// Canonical Windows verbatim spellings, drive roots and case/trailing-separator variants must
/// reach the typed danger rejection rather than falling through to containment.
#[cfg(windows)]
#[test]
fn windows_canonical_system_output_roots_are_rejected_with_typed_reason() {
    let project_temp = tempfile::tempdir().expect("should create project temp dir");
    let project_root = project_temp.path();

    for root in [
        r"\\?\C:\Windows",
        r"\\?\C:\WINDOWS",
        r"\\?\c:\windows",
        r"\\?\C:\Users",
        r"C:\Windows",
        r"C:\Windows\",
        r"c:\WINDOWS",
        r"C:\Users",
        r"C:\Users\",
        r"C:\Program Files",
        r"\\?\C:\",
        r"C:\",
    ] {
        assert_dangerous_system_path_rejection(Path::new(root), project_root);
    }
}

/// A UNC share root is a filesystem root, while a project directory inside the share is not.
///
/// WHY: the safety boundary canonicalises its input, which would contact `\\server\share` and
/// require network access. This asserts the component comparison directly instead.
#[cfg(windows)]
#[test]
fn windows_unc_share_root_is_dangerous_but_share_subdirectory_is_not() {
    for share_root in [
        r"\\server\share",
        r"\\server\share\",
        r"\\?\UNC\server\share",
        r"\\?\UNC\server\share\",
    ] {
        assert!(
            is_dangerous_system_path(Path::new(share_root)),
            "'{share_root}' names a share root and must be dangerous"
        );
    }

    for share_subdirectory in [
        r"\\server\share\project\_dev",
        r"\\?\UNC\server\share\project\_dev",
    ] {
        assert!(
            !is_dangerous_system_path(Path::new(share_subdirectory)),
            "'{share_subdirectory}' is not the protected share root"
        );
    }
}

/// The guard folds ASCII case by component, so only an exact protected root is rejected.
///
/// WHY: the safety boundary canonicalises first, which masks the fold whenever the host stores the
/// ordinary spelling. This asserts the comparison itself, including that descendants stay legal.
#[cfg(windows)]
#[test]
fn windows_system_root_guard_folds_case_and_requires_exact_components() {
    for protected in [
        r"C:\Windows",
        r"c:\windows",
        r"C:\WINDOWS",
        r"\\?\C:\Windows",
        r"\\?\c:\WINDOWS",
        r"C:\Windows\System32",
        r"c:\windows\system32",
        r"C:\Program Files (x86)",
        r"C:\Users",
    ] {
        assert!(
            is_dangerous_system_path(Path::new(protected)),
            "'{protected}' names a protected Windows root"
        );
    }

    for allowed in [
        r"C:\Windows\Temp",
        r"C:\Windows\System32\drivers",
        r"C:\Users\build",
        r"C:\Program Files\app",
        r"\\?\C:\Users\build\_dev",
    ] {
        assert!(
            !is_dangerous_system_path(Path::new(allowed)),
            "'{allowed}' is not an exact protected Windows root"
        );
    }
}

/// Named Windows protected roots follow the runtime system drive, not only `C:`.
///
/// WHY: `SystemDrive` and `SystemRoot` can name another installation drive, and a hard-coded
/// `C:` list leaves `D:\Windows`, `D:\Program Files` and `D:\Users` unprotected. Locations are
/// injected because edition 2024 `std::env::set_var` is unsafe and races under parallel tests.
#[cfg(windows)]
#[test]
fn windows_protected_roots_follow_the_system_drive() {
    for (system_root, system_drive) in [
        (r"D:\Windows", r"D:"),
        (r"d:\windows", r"d:"),
        (r"D:\Windows\", r"D:\"),
        ("D:/Windows", "D:/"),
    ] {
        let locations = WindowsSystemLocations::from_parts(Some(system_root), Some(system_drive));

        for protected_root in locations.protected_roots() {
            assert!(
                protected_root.has_root(),
                "SystemRoot '{system_root}' must not derive the drive-relative '{}'",
                protected_root.display()
            );
        }

        for protected in [
            r"\\?\D:\Windows",
            r"D:\Windows",
            r"D:\Windows\System32",
            r"D:\Users",
            r"D:\Program Files",
            r"D:\Program Files (x86)",
            r"C:\Windows",
            r"C:\Windows\System32",
            r"C:\Users",
            r"C:\Program Files",
        ] {
            assert!(
                is_dangerous_system_path_with_locations(Path::new(protected), &locations),
                "'{protected}' names a protected Windows root for SystemRoot '{system_root}'"
            );
        }

        // For this installation below `D:\Windows`, drive-root System32 spellings are ordinary
        // directories rather than protected roots.
        for allowed in [
            r"D:\Windows\Temp",
            r"D:\Windows\System32\drivers",
            r"D:\System32",
            r"C:\System32",
            r"D:\Users\build",
            r"D:\Program Files\app",
            r"D:\dev\_dev",
        ] {
            assert!(
                !is_dangerous_system_path_with_locations(Path::new(allowed), &locations),
                "'{allowed}' is not an exact protected Windows root for SystemRoot '{system_root}'"
            );
        }
    }
}

/// A drive-root `SystemRoot` places `System32` directly on the drive.
///
/// WHY: Windows permits an installation at `D:\`, where the system directory is `D:\System32`.
/// Rejecting that `SystemRoot` as malformed would fall back to `D:\Windows` and stop protecting
/// the real system directory. The drive root itself is already rejected by the structural
/// filesystem-root rule, so including it in this layout adds no protection beyond that rule.
#[cfg(windows)]
#[test]
fn windows_system_root_at_a_drive_root_protects_its_system32() {
    for (system_root, system_drive) in [(r"D:\", Some(r"D:")), ("D:/", Some("D:/")), (r"D:\", None)]
    {
        let locations = WindowsSystemLocations::from_parts(Some(system_root), system_drive);

        for protected in [
            r"D:\",
            r"D:\System32",
            r"D:\Users",
            r"D:\Program Files",
            r"D:\Program Files (x86)",
        ] {
            assert!(
                is_dangerous_system_path_with_locations(Path::new(protected), &locations),
                "'{protected}' names a protected Windows root for SystemRoot '{system_root}'"
            );
        }

        for allowed in [
            r"D:\System32\drivers",
            r"D:\Windows",
            r"D:\Windows\System32",
            r"D:\Users\build",
        ] {
            assert!(
                !is_dangerous_system_path_with_locations(Path::new(allowed), &locations),
                "'{allowed}' is not an exact protected Windows root for SystemRoot '{system_root}'"
            );
        }
    }
}

/// With no usable `SystemDrive`, a nested `SystemRoot` still contributes its disk drive root.
///
/// WHY: `D:\OS\Windows` sits below the drive root. Deriving the root from SystemRoot's parent
/// would protect `D:\OS\Users` and leave the real `D:\Users` and `D:\Program Files` unprotected.
#[cfg(windows)]
#[test]
fn windows_protected_roots_keep_a_nested_system_root_on_its_drive() {
    for system_drive in [None, Some(r"D:\Windows")] {
        let locations = WindowsSystemLocations::from_parts(Some(r"D:\OS\Windows"), system_drive);
        let system_drive_label = system_drive.unwrap_or("unset");

        for protected in [
            r"D:\OS\Windows",
            r"D:\OS\Windows\System32",
            r"D:\Users",
            r"D:\Program Files",
            r"D:\Program Files (x86)",
        ] {
            assert!(
                is_dangerous_system_path_with_locations(Path::new(protected), &locations),
                "'{protected}' names a protected Windows root for SystemDrive '{system_drive_label}'"
            );
        }

        for allowed in [r"D:\OS\Users", r"D:\OS\Program Files"] {
            assert!(
                !is_dangerous_system_path_with_locations(Path::new(allowed), &locations),
                "'{allowed}' is below SystemRoot's drive root and is not protected"
            );
        }
    }
}

/// Malformed system locations are ignored instead of deriving partial protected roots.
///
/// WHY: a drive-relative `D:Windows`, a relative directory name or a UNC value cannot name a
/// system location. Deriving from one would either compare against paths the system never
/// protects or silently drop the fallback drive's real roots.
#[cfg(windows)]
#[test]
fn windows_protected_roots_ignore_malformed_system_locations() {
    for malformed_system_root in [r"D:Windows", r"Windows", r"\\server\share\Windows"] {
        let locations = WindowsSystemLocations::from_parts(Some(malformed_system_root), Some("D:"));
        assert!(
            is_dangerous_system_path_with_locations(Path::new(r"D:\Windows"), &locations),
            "malformed SystemRoot '{malformed_system_root}' must fall back to the system drive"
        );
        assert!(
            is_dangerous_system_path_with_locations(Path::new(r"D:\Windows\System32"), &locations),
            "malformed SystemRoot '{malformed_system_root}' must keep System32 under the fallback"
        );
    }

    for malformed_system_drive in [r"D:\Windows", r"Windows", r"\\server\share"] {
        let locations =
            WindowsSystemLocations::from_parts(Some(r"D:\Windows"), Some(malformed_system_drive));
        assert!(
            is_dangerous_system_path_with_locations(Path::new(r"D:\Program Files"), &locations),
            "malformed SystemDrive '{malformed_system_drive}' must fall back to SystemRoot's drive"
        );
    }

    let missing_locations = WindowsSystemLocations::from_parts(None, None);
    assert!(
        is_dangerous_system_path_with_locations(Path::new(r"C:\Windows"), &missing_locations),
        "missing system locations must keep the C: fallback protected"
    );
    assert!(
        is_dangerous_system_path_with_locations(
            Path::new(r"C:\Windows\System32"),
            &missing_locations
        ),
        "missing system locations must keep the literal System32 root protected"
    );
    assert!(
        !is_dangerous_system_path_with_locations(Path::new(r"D:\Users"), &missing_locations),
        "a drive named only by malformed or missing values is not protected"
    );
}

/// The authoritative Windows directory wins over a `Windows` name on the system drive.
#[cfg(windows)]
#[test]
fn windows_protected_roots_use_the_authoritative_system_root() {
    let locations = WindowsSystemLocations::from_parts(Some(r"D:\WinNT"), Some(r"D:"));

    assert!(
        is_dangerous_system_path_with_locations(Path::new(r"D:\WinNT"), &locations),
        "SystemRoot names the installation directory even when it is not called Windows"
    );
    assert!(
        is_dangerous_system_path_with_locations(Path::new(r"D:\WinNT\System32"), &locations),
        "System32 lives under the authoritative installation directory"
    );
    assert!(
        is_dangerous_system_path_with_locations(Path::new(r"D:\Users"), &locations),
        "named directories on the system drive stay protected beside a renamed installation"
    );
    assert!(
        !is_dangerous_system_path_with_locations(Path::new(r"D:\Windows"), &locations),
        "an unused Windows directory on the system drive is not the installation directory"
    );
}

/// A project-local hidden output root must remain legal, including Windows temporary projects
/// that live under `C:\Users\<user>` but are not the protected `C:\Users` root itself.
#[test]
fn project_local_hidden_output_root_is_allowed() {
    let project_temp = tempfile::tempdir().expect("should create project temp dir");
    let project_root = fs::canonicalize(project_temp.path()).expect("project root should resolve");
    fs::create_dir(project_root.join("_dev")).expect("should create hidden output root");

    validate_directory_output_root_safety(&project_root.join("_dev"), &project_root)
        .expect("an ordinary project-local hidden output root is not a protected system path");
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
fn authored_separator_variants_parse_to_one_portable_path() {
    let slash =
        parse_relative_path("nested/output").expect("authored slash spelling is a valid path");
    let backslash =
        parse_relative_path("nested\\output").expect("authored backslash spelling is a valid path");
    assert_eq!(
        slash, backslash,
        "authored strings treat both separators as structural"
    );
    assert_eq!(
        slash.identity(),
        backslash.identity(),
        "authored separator variants must be one output identity"
    );
}

#[test]
fn native_paths_expose_slash_joined_portable_spelling() {
    let native = Path::new("nested").join("page.html");
    let portable =
        parse_native_relative_path(&native).expect("joined path is a valid relative path");
    assert_eq!(portable.portable_spelling(), "nested/page.html");

    let authored = parse_relative_path("nested\\page.html").expect("authored backslash is valid");
    assert_eq!(
        authored.portable_spelling(),
        "nested/page.html",
        "both parsers feed the same validated spelling"
    );
}

#[cfg(unix)]
#[test]
fn native_component_with_literal_backslash_is_rejected_not_re_split() {
    // WHAT: a Unix filename may contain `\`, but it is one directory-entry component.
    // WHY: re-parsing a native path as an authored string would alias `a\b` with `a/b`.
    let native = Path::new("nested\\output");

    assert_eq!(
        output_path_identity(native),
        Err(InvalidOutputFolderReason::InvalidPathComponent),
        "a literal backslash must not become a separator in the native identity"
    );
    assert_eq!(
        output_path_component_identities(native),
        Err(InvalidOutputFolderReason::InvalidPathComponent)
    );
    assert_eq!(
        normalize_relative_path(native),
        Err(InvalidOutputFolderReason::InvalidPathComponent)
    );
    assert!(
        !is_lossless_portable_relative_path(native),
        "a native path that cannot round-trip is never lossless"
    );
    assert!(
        output_path_identity(Path::new("nested/output")).is_ok(),
        "the distinct nested file remains a valid output identity"
    );
}

#[test]
fn native_leading_separator_stays_absolute_before_component_scan() {
    // WHAT: a native spelling that begins with a separator is rooted, even though its remaining
    //       text also contains `\` characters.
    // WHY: the component scan rejects a literal `\` inside a Unix filename, so it must run after
    //      the rooted check or these paths would be misclassified as invalid components.
    assert_eq!(
        output_path_identity(Path::new("\\\\server\\share")),
        Err(InvalidOutputFolderReason::AbsolutePath),
        "a native UNC-like spelling is rooted on every host"
    );
    assert_eq!(
        normalize_relative_path(Path::new("\\rooted")),
        Err(InvalidOutputFolderReason::AbsolutePath),
        "a single leading backslash is a root, not a directory-entry name"
    );
}

#[cfg(windows)]
#[test]
fn windows_native_backslash_separates_components_without_rejection() {
    let slash = output_path_identity(Path::new("nested/output"))
        .expect("nested/output is a valid relative path");
    let backslash = output_path_identity(Path::new("nested\\output"))
        .expect("Windows native separators are structural path components");
    assert_eq!(
        slash, backslash,
        "Windows separator spelling must keep sharing one output identity"
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
