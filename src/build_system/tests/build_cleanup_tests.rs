//! Tests for the core build orchestration and output writer APIs.
// NOTE: temp file creation processes have to be explicitly dropped
// Or these tests will fail on Windows due to attempts to delete non-empty temp directories while files are still open.

use super::*;
use crate::build_system::BuildProfile;
use crate::build_system::build::{FileKind, OutputFile};
use crate::build_system::output::manifest::{
    BuildManifest, ManifestReadResult, ManifestRecoveryReason,
};
use crate::build_system::output::{BuilderKind, OutputOwner, WriteMode, WriteOptions};
use crate::compiler_frontend::compiler_messages::display_messages::format_terse_compiler_messages;
use crate::compiler_frontend::compiler_messages::render::resolve_source_file_path;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use std::fs;
use std::path::{Path, PathBuf};

use crate::build_system::output::manifest::{
    BUILD_MANIFEST_FILENAME, read_build_manifest, remove_manifest_tracked_stale_artifacts,
    validate_output_root_is_safe, write_build_manifest,
};
use crate::compiler_tests::test_support::temp_dir;
use std::collections::{BTreeSet, HashSet};

fn html_owner(profile: BuildProfile) -> OutputOwner {
    OutputOwner {
        builder: BuilderKind::Html,
        profile,
    }
}

fn read_html_manifest(root: &Path) -> ManifestReadResult {
    read_build_manifest(root, &html_cleanup_policy())
}

fn valid_manifest(paths: Vec<PathBuf>, profile: BuildProfile) -> ManifestReadResult {
    ManifestReadResult::Valid(BuildManifest {
        paths,
        owner: html_owner(profile),
        managed_extensions: html_active_extensions(),
    })
}

#[test]
fn cleanup_manifest_diff_removes_stale_managed_files() {
    let root = temp_dir("cleanup_stale");
    fs::create_dir_all(&root).expect("should create temp root");
    let project_dir = root.join("project");
    fs::create_dir_all(&project_dir).expect("should create project dir");
    let output_root = project_dir.join("dev");

    // Build A: index.html + about/index.html
    let project_a = html_project(
        vec![
            OutputFile::new(
                PathBuf::from("index.html"),
                FileKind::Html(String::from("<html>Home</html>")),
            ),
            OutputFile::new(
                PathBuf::from("about/index.html"),
                FileKind::Html(String::from("<html>About</html>")),
            ),
            OutputFile::new(
                PathBuf::from("scripts/page.js"),
                FileKind::Js(String::from("console.log('about');")),
            ),
        ],
        Some(PathBuf::from("index.html")),
    );
    write_project_outputs(
        &project_a,
        &always_write_options(output_root.clone(), Some(project_dir.clone())),
    )
    .expect("build A should succeed");

    assert!(output_root.join("index.html").exists());
    assert!(output_root.join("about/index.html").exists());
    assert!(output_root.join("scripts/page.js").exists());
    assert!(output_root.join(BUILD_MANIFEST_FILENAME).exists());

    // Build B: only index.html
    let project_b = html_project(
        vec![OutputFile::new(
            PathBuf::from("index.html"),
            FileKind::Html(String::from("<html>Home v2</html>")),
        )],
        Some(PathBuf::from("index.html")),
    );
    write_project_outputs(
        &project_b,
        &always_write_options(output_root.clone(), Some(project_dir.clone())),
    )
    .expect("build B should succeed");

    assert!(output_root.join("index.html").exists());
    assert!(
        !output_root.join("about/index.html").exists(),
        "stale about/index.html should have been removed"
    );
    assert!(
        !output_root.join("scripts/page.js").exists(),
        "stale scripts/page.js should have been removed"
    );
    assert!(
        !output_root.join("about").exists(),
        "empty about/ directory should have been removed"
    );

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[test]
fn cleanup_manifest_diff_removes_stale_tracked_byte_assets_from_manifest() {
    let root = temp_dir("cleanup_stale_bytes");
    fs::create_dir_all(&root).expect("should create temp root");
    let project_dir = root.join("project");
    fs::create_dir_all(&project_dir).expect("should create project dir");
    let output_root = project_dir.join("dev");

    let project_a = html_project(
        vec![
            OutputFile::new(
                PathBuf::from("index.html"),
                FileKind::Html(String::from("<html>Home</html>")),
            ),
            OutputFile::new(
                PathBuf::from("assets/logo.png"),
                FileKind::Bytes(vec![1, 2, 3, 4]),
            ),
        ],
        Some(PathBuf::from("index.html")),
    );
    write_project_outputs(
        &project_a,
        &always_write_options(output_root.clone(), Some(project_dir.clone())),
    )
    .expect("build A should succeed");

    assert_eq!(
        read_html_manifest(&output_root),
        valid_manifest(
            vec![
                PathBuf::from("assets/logo.png"),
                PathBuf::from("index.html")
            ],
            BuildProfile::Dev,
        )
    );
    assert!(output_root.join("assets/logo.png").exists());

    let project_b = html_project(
        vec![OutputFile::new(
            PathBuf::from("index.html"),
            FileKind::Html(String::from("<html>Home v2</html>")),
        )],
        Some(PathBuf::from("index.html")),
    );
    write_project_outputs(
        &project_b,
        &always_write_options(output_root.clone(), Some(project_dir.clone())),
    )
    .expect("build B should succeed");

    assert!(
        !output_root.join("assets/logo.png").exists(),
        "stale tracked byte asset should have been removed"
    );

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[test]
fn cleanup_missing_manifest_preserves_stale_html_route_alias() {
    let root = temp_dir("cleanup_missing_manifest_alias");
    fs::create_dir_all(&root).expect("should create temp root");
    let project_dir = root.join("project");
    fs::create_dir_all(&project_dir).expect("should create project dir");
    let output_root = project_dir.join("dev");
    fs::create_dir_all(output_root.join("docs")).expect("should create docs output dir");
    fs::write(
        output_root.join("docs/basics.html"),
        "<html>stale flat route</html>",
    )
    .expect("should write stale alias");

    let manifest = read_html_manifest(&output_root);
    assert_eq!(
        manifest,
        ManifestReadResult::Recoverable {
            reason: ManifestRecoveryReason::Missing,
        }
    );

    let project = html_project(
        vec![OutputFile::new(
            PathBuf::from("docs/basics/index.html"),
            FileKind::Html(String::from("<html>Docs</html>")),
        )],
        Some(PathBuf::from("docs/basics/index.html")),
    );
    write_project_outputs(
        &project,
        &always_write_options(output_root.clone(), Some(project_dir.clone())),
    )
    .expect("build should succeed");

    assert!(
        output_root.join("docs/basics.html").exists(),
        "missing manifests must preserve stale aliases until a valid manifest is available"
    );
    assert!(output_root.join("docs/basics/index.html").exists());

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[test]
fn cleanup_first_build_writes_manifest_without_removing() {
    let root = temp_dir("cleanup_first_build");
    fs::create_dir_all(&root).expect("should create temp root");
    let project_dir = root.join("project");
    fs::create_dir_all(&project_dir).expect("should create project dir");
    let output_root = project_dir.join("dev");

    assert!(!output_root.join(BUILD_MANIFEST_FILENAME).exists());

    let project = html_project(
        vec![OutputFile::new(
            PathBuf::from("index.html"),
            FileKind::Html(String::from("<html>Home</html>")),
        )],
        Some(PathBuf::from("index.html")),
    );
    write_project_outputs(
        &project,
        &always_write_options(output_root.clone(), Some(project_dir.clone())),
    )
    .expect("first build should succeed");

    assert!(output_root.join("index.html").exists());
    assert!(
        output_root.join(BUILD_MANIFEST_FILENAME).exists(),
        "manifest should be written on first build"
    );

    assert_eq!(
        read_html_manifest(&output_root),
        valid_manifest(vec![PathBuf::from("index.html")], BuildProfile::Dev)
    );

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[test]
fn cleanup_removes_empty_parent_directories_after_deleting_managed_files() {
    let root = temp_dir("cleanup_empty_parents");
    fs::create_dir_all(&root).expect("should create temp root");
    let project_dir = root.join("project");
    fs::create_dir_all(&project_dir).expect("should create project dir");
    let output_root = project_dir.join("dev");

    let project_a = html_project(
        vec![OutputFile::new(
            PathBuf::from("a/b/c/file.js"),
            FileKind::Js(String::from("console.log('deep');")),
        )],
        None,
    );
    write_project_outputs(
        &project_a,
        &always_write_options(output_root.clone(), Some(project_dir.clone())),
    )
    .expect("build A should succeed");
    assert!(output_root.join("a/b/c/file.js").exists());

    let project_b = html_project(
        vec![OutputFile::new(
            PathBuf::from("index.html"),
            FileKind::Html(String::from("<html></html>")),
        )],
        Some(PathBuf::from("index.html")),
    );
    write_project_outputs(
        &project_b,
        &always_write_options(output_root.clone(), Some(project_dir.clone())),
    )
    .expect("build B should succeed");

    assert!(
        !output_root.join("a").exists(),
        "empty parent directories should be removed after safe file deletion"
    );

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[test]
fn cleanup_preserves_current_explicit_directory_after_removing_stale_child() {
    let root = temp_dir("cleanup_explicit_directory");
    fs::create_dir_all(&root).expect("should create temp root");
    let project_dir = root.join("project");
    fs::create_dir_all(&project_dir).expect("should create project dir");
    let output_root = project_dir.join("dev");
    fs::create_dir_all(output_root.join("assets")).expect("should create assets directory");
    fs::write(output_root.join("assets/stale.js"), "stale").expect("should write stale asset");

    let manifest_paths: HashSet<PathBuf> = [PathBuf::from("assets/stale.js")].into_iter().collect();
    write_build_manifest(
        &output_root.join(BUILD_MANIFEST_FILENAME),
        &manifest_paths,
        html_owner(BuildProfile::Dev),
        &html_cleanup_policy(),
        WriteMode::AlwaysWrite,
        &StringTable::new(),
    )
    .expect("should write valid prior manifest");

    let project = html_project(
        vec![OutputFile::new(
            PathBuf::from("assets"),
            FileKind::Directory,
        )],
        None,
    );
    write_project_outputs(
        &project,
        &always_write_options(output_root.clone(), Some(project_dir.clone())),
    )
    .expect("directory-only build should succeed");

    assert!(
        output_root.join("assets").is_dir(),
        "a current explicit directory must survive stale child cleanup"
    );
    assert!(!output_root.join("assets/stale.js").exists());

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[cfg(unix)]
#[test]
fn stale_cleanup_preserves_non_regular_nodes() {
    use std::os::unix::net::UnixListener;

    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time should be after epoch")
        .as_nanos();
    let root = PathBuf::from(format!(
        "/tmp/moth_socket_{}_{}",
        std::process::id(),
        unique
    ));
    fs::create_dir_all(&root).expect("should create temp root");
    let socket_path = root.join("stale.sock");
    let listener = UnixListener::bind(&socket_path).expect("should create stale socket");
    let stale_path = PathBuf::from("stale.sock");

    let report = remove_manifest_tracked_stale_artifacts(
        &root,
        &HashSet::new(),
        &HashSet::new(),
        std::slice::from_ref(&stale_path),
    );

    assert!(
        socket_path.exists(),
        "stale special nodes must not be unlinked"
    );
    assert_eq!(report.removed_paths, Vec::<PathBuf>::new());
    assert_eq!(report.retained_paths, vec![stale_path]);

    drop(listener);
    fs::remove_file(&socket_path).expect("should remove test socket");
    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn cleanup_preserves_parent_directories_when_non_managed_files_remain() {
    let root = temp_dir("cleanup_preserves_parent_dirs");
    fs::create_dir_all(&root).expect("should create temp root");
    let project_dir = root.join("project");
    fs::create_dir_all(&project_dir).expect("should create project dir");
    let output_root = project_dir.join("dev");
    fs::create_dir_all(output_root.join("docs/basics")).expect("should create docs output dir");
    fs::write(
        output_root.join("docs/basics/index.html"),
        "<html>stale nested route</html>",
    )
    .expect("should write stale html file");
    fs::write(output_root.join("docs/basics/notes.txt"), "keep me")
        .expect("should write preserved notes file");
    let manifest_paths: HashSet<PathBuf> = [PathBuf::from("docs/basics/index.html")]
        .into_iter()
        .collect();
    write_build_manifest(
        &output_root.join(BUILD_MANIFEST_FILENAME),
        &manifest_paths,
        html_owner(BuildProfile::Dev),
        &html_cleanup_policy(),
        WriteMode::AlwaysWrite,
        &StringTable::new(),
    )
    .expect("should write v4 manifest");

    let project = html_project(
        vec![OutputFile::new(
            PathBuf::from("index.html"),
            FileKind::Html(String::from("<html>Home</html>")),
        )],
        Some(PathBuf::from("index.html")),
    );
    write_project_outputs(
        &project,
        &always_write_options(output_root.clone(), Some(project_dir.clone())),
    )
    .expect("build should succeed");

    assert!(
        output_root.join("docs/basics").exists(),
        "directories containing preserved files should not be pruned"
    );
    assert!(output_root.join("docs/basics/notes.txt").exists());

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[test]
fn validate_output_root_rejects_dangerous_paths() {
    let project_dir = PathBuf::from("/tmp/test_project");

    let dangerous_paths = vec![
        PathBuf::from("/"),
        PathBuf::from("/usr"),
        PathBuf::from("/etc"),
        PathBuf::from("/bin"),
        PathBuf::from("/var"),
    ];

    for dangerous in dangerous_paths {
        let result =
            validate_output_root_is_safe(&dangerous, &project_dir, None, &StringTable::new());
        assert!(
            result.is_err(),
            "should reject dangerous path: {}",
            dangerous.display()
        );
    }
}

#[test]
fn validate_output_root_accepts_project_subdirectory() {
    let root = temp_dir("validate_accept");
    fs::create_dir_all(root.join("dev")).expect("should create output dir");

    let result = validate_output_root_is_safe(&root.join("dev"), &root, None, &StringTable::new());
    assert!(
        result.is_ok(),
        "should accept output root inside project directory"
    );

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[test]
fn cleanup_unreadable_manifest_enters_recoverable_mode_and_preserves_existing_files() {
    let root = temp_dir("cleanup_garbage_manifest");
    fs::create_dir_all(&root).expect("should create temp root");
    let project_dir = root.join("project");
    fs::create_dir_all(&project_dir).expect("should create project dir");
    let output_root = project_dir.join("dev");
    fs::create_dir_all(output_root.join("docs")).expect("should create docs output dir");
    fs::create_dir_all(output_root.join("custom")).expect("should create custom output dir");

    fs::write(
        output_root.join(BUILD_MANIFEST_FILENAME),
        b"\0\0\x01\x02 binary garbage \xFF\xFE",
    )
    .expect("should write garbage manifest");
    fs::write(
        output_root.join("docs/basics.html"),
        "<html>stale alias</html>",
    )
    .expect("should write stale alias");
    fs::write(
        output_root.join("custom/landing.html"),
        "<html>preserve me</html>",
    )
    .expect("should write unrelated html file");

    assert_eq!(
        read_html_manifest(&output_root),
        ManifestReadResult::Recoverable {
            reason: ManifestRecoveryReason::Unreadable,
        }
    );

    let project = html_project(
        vec![OutputFile::new(
            PathBuf::from("docs/basics/index.html"),
            FileKind::Html(String::from("<html>Docs</html>")),
        )],
        Some(PathBuf::from("docs/basics/index.html")),
    );
    write_project_outputs(
        &project,
        &always_write_options(output_root.clone(), Some(project_dir.clone())),
    )
    .expect("build should succeed despite unreadable manifest");

    assert!(
        output_root.join("docs/basics.html").exists(),
        "unreadable manifests must preserve stale aliases until a valid manifest is available"
    );
    assert!(
        output_root.join("custom/landing.html").exists(),
        "unknown managed-looking files should be preserved when full manifest cleanup is unavailable"
    );

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[test]
fn cleanup_disabled_skips_manifest_cleanup() {
    let root = temp_dir("cleanup_disabled");
    fs::create_dir_all(root.join("docs")).expect("should create temp root");
    fs::write(root.join("docs/basics.html"), "<html>stale alias</html>")
        .expect("should write stale alias");

    let project = html_project(
        vec![OutputFile::new(
            PathBuf::from("docs/basics/index.html"),
            FileKind::Html(String::from("<html></html>")),
        )],
        Some(PathBuf::from("docs/basics/index.html")),
    );
    write_project_outputs(&project, &always_write_options(root.clone(), None))
        .expect("build should succeed");

    assert!(root.join("docs/basics/index.html").exists());
    assert!(
        root.join("docs/basics.html").exists(),
        "cleanup-disabled builds should not remove stale files"
    );
    assert!(
        !root.join(BUILD_MANIFEST_FILENAME).exists(),
        "manifest should not be written when cleanup is disabled"
    );

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[test]
fn unsupported_manifest_preserves_existing_files_until_next_cleanup() {
    let root = temp_dir("cleanup_legacy_manifest");
    fs::create_dir_all(&root).expect("should create temp root");
    let project_dir = root.join("project");
    fs::create_dir_all(&project_dir).expect("should create project dir");
    let output_root = project_dir.join("dev");
    fs::create_dir_all(output_root.join("about")).expect("should create about output dir");
    fs::create_dir_all(output_root.join("scripts")).expect("should create scripts output dir");

    fs::write(
        output_root.join("about/index.html"),
        "<html>stale about</html>",
    )
    .expect("should write stale html file");
    fs::write(output_root.join("scripts/page.js"), "console.log('stale');")
        .expect("should write stale js file");
    fs::write(output_root.join("notes.txt"), "keep me").expect("should write notes file");
    fs::write(
        output_root.join(BUILD_MANIFEST_FILENAME),
        "about/index.html\nscripts/page.js\nnotes.txt\n",
    )
    .expect("should write unsupported manifest");

    assert_eq!(
        read_html_manifest(&output_root),
        ManifestReadResult::Recoverable {
            reason: ManifestRecoveryReason::UnsupportedVersion,
        },
    );

    let project = html_project(
        vec![OutputFile::new(
            PathBuf::from("index.html"),
            FileKind::Html(String::from("<html>Home</html>")),
        )],
        Some(PathBuf::from("index.html")),
    );
    write_project_outputs(
        &project,
        &always_write_options(output_root.clone(), Some(project_dir.clone())),
    )
    .expect("build should succeed");

    assert!(
        output_root.join("about/index.html").exists(),
        "unsupported manifests must not drive stale html cleanup"
    );
    assert!(
        output_root.join("scripts/page.js").exists(),
        "unsupported manifests must not drive stale js cleanup"
    );
    assert!(
        output_root.join("notes.txt").exists(),
        "recoverable mode must preserve non-managed file types"
    );

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

/// Write a v4 manifest directly so extension-mismatch cases can vary metadata without reusing the
/// active policy's writer.
fn write_v4_manifest_text(
    root: &Path,
    builder: &str,
    profile: &str,
    extensions_csv: &str,
    paths: &[&str],
) {
    let mut lines = vec![
        String::from("# moth-manifest v4"),
        format!("# builder: {builder}"),
        format!("# profile: {profile}"),
        format!("# managed_extensions: {extensions_csv}"),
    ];
    for path in paths {
        lines.push((*path).to_string());
    }
    fs::write(root.join(BUILD_MANIFEST_FILENAME), lines.join("\n"))
        .expect("should write v4 manifest");
}

fn html_active_extensions() -> BTreeSet<String> {
    [".html", ".js", ".wasm"]
        .iter()
        .map(|extension| (*extension).to_string())
        .collect()
}

#[test]
fn read_build_manifest_accepts_equivalent_managed_extensions_in_different_order() {
    let root = temp_dir("cleanup_ext_order");
    fs::create_dir_all(&root).expect("should create temp root");

    write_v4_manifest_text(&root, "html", "dev", ".wasm,.html,.js", &["index.html"]);

    assert_eq!(
        read_html_manifest(&root),
        valid_manifest(vec![PathBuf::from("index.html")], BuildProfile::Dev)
    );

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[test]
fn read_build_manifest_normalizes_managed_extension_case_and_leading_dot() {
    let root = temp_dir("cleanup_ext_normalize");
    fs::create_dir_all(&root).expect("should create temp root");

    // Uppercase and dotless forms must normalize to the active lowercased dotted set.
    write_v4_manifest_text(&root, "html", "dev", "HTML,js,.WASM", &["index.html"]);

    assert_eq!(
        read_html_manifest(&root),
        valid_manifest(vec![PathBuf::from("index.html")], BuildProfile::Dev)
    );

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[test]
fn read_build_manifest_rejects_missing_managed_extension() {
    let root = temp_dir("cleanup_ext_missing");
    fs::create_dir_all(&root).expect("should create temp root");

    let manifest_extensions: BTreeSet<String> = [".html", ".js"]
        .iter()
        .map(|ext| (*ext).to_string())
        .collect();

    write_v4_manifest_text(&root, "html", "dev", ".html,.js", &["index.html"]);

    assert_eq!(
        read_html_manifest(&root),
        ManifestReadResult::RecoverableWithOwner {
            reason: ManifestRecoveryReason::ManagedExtensionsMismatch {
                manifest_extensions,
                active_extensions: html_active_extensions(),
            },
            owner: html_owner(BuildProfile::Dev),
        }
    );

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[test]
fn read_build_manifest_rejects_extra_managed_extension() {
    let root = temp_dir("cleanup_ext_extra");
    fs::create_dir_all(&root).expect("should create temp root");

    let manifest_extensions: BTreeSet<String> = [".css", ".html", ".js", ".wasm"]
        .iter()
        .map(|ext| (*ext).to_string())
        .collect();

    write_v4_manifest_text(
        &root,
        "html",
        "dev",
        ".html,.js,.wasm,.css",
        &["index.html"],
    );

    assert_eq!(
        read_html_manifest(&root),
        ManifestReadResult::RecoverableWithOwner {
            reason: ManifestRecoveryReason::ManagedExtensionsMismatch {
                manifest_extensions,
                active_extensions: html_active_extensions(),
            },
            owner: html_owner(BuildProfile::Dev),
        }
    );

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[test]
fn read_v4_recovery_retains_known_owner_after_extension_metadata_damage() {
    let cases = [
        (
            "missing",
            "# moth-manifest v4\n# builder: html\n# profile: release\nindex.html\n",
            ManifestRecoveryReason::InvalidMetadata,
        ),
        (
            "malformed",
            "# moth-manifest v4\n# builder: html\n# profile: release\n# managed_extensions: .html,,.js\nindex.html\n",
            ManifestRecoveryReason::InvalidMetadata,
        ),
        (
            "blank",
            "# moth-manifest v4\n# builder: html\n# profile: release\n# managed_extensions:\nindex.html\n",
            ManifestRecoveryReason::InvalidMetadata,
        ),
    ];

    for (case_name, manifest_text, reason) in cases {
        let root = temp_dir(&format!("cleanup_owner_recovery_{case_name}"));
        fs::create_dir_all(&root).expect("should create temp root");
        fs::write(root.join(BUILD_MANIFEST_FILENAME), manifest_text)
            .expect("should write damaged v4 manifest");

        assert_eq!(
            read_html_manifest(&root),
            ManifestReadResult::RecoverableWithOwner {
                reason,
                owner: html_owner(BuildProfile::Release),
            },
            "known builder/profile ownership must survive recoverable metadata damage"
        );

        fs::remove_dir_all(&root).expect("should remove temp dir");
    }
}

#[cfg(unix)]
#[test]
fn stale_cleanup_retains_paths_with_dangling_symlink_components() {
    use std::os::unix::fs::symlink;

    let root = temp_dir("cleanup_dangling_symlink");
    fs::create_dir_all(&root).expect("should create output root");
    symlink(root.join("real"), root.join("alias")).expect("should create dangling alias");

    let stale_path = PathBuf::from("alias/stale.html");
    let report = remove_manifest_tracked_stale_artifacts(
        &root,
        &HashSet::new(),
        &HashSet::new(),
        std::slice::from_ref(&stale_path),
    );

    assert!(report.removed_paths.is_empty());
    assert_eq!(report.ignored_paths, vec![stale_path]);
    assert!(
        fs::symlink_metadata(root.join("alias"))
            .expect("dangling alias should remain")
            .file_type()
            .is_symlink()
    );
    assert!(!root.join("real").exists());

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn foreign_profile_in_recoverable_v4_manifest_fails_without_mutation() {
    let manifest_variants = [
        (
            "missing",
            "# moth-manifest v4\n# builder: html\n# profile: release\nindex.html\n",
        ),
        (
            "malformed",
            "# moth-manifest v4\n# builder: html\n# profile: release\n# managed_extensions: .html,,.js\nindex.html\n",
        ),
        (
            "blank",
            "# moth-manifest v4\n# builder: html\n# profile: release\n# managed_extensions:\nindex.html\n",
        ),
    ];

    for (case_name, manifest_text) in manifest_variants {
        let root = temp_dir(&format!("cleanup_foreign_recovery_{case_name}"));
        let project_dir = root.join("project");
        let output_root = project_dir.join("dev");
        fs::create_dir_all(&output_root).expect("should create output root");
        fs::write(output_root.join("index.html"), "existing")
            .expect("should write existing output");
        fs::write(output_root.join(BUILD_MANIFEST_FILENAME), manifest_text)
            .expect("should write damaged v4 manifest");
        let previous_output = fs::read(output_root.join("index.html")).expect("output exists");
        let previous_manifest =
            fs::read(output_root.join(BUILD_MANIFEST_FILENAME)).expect("manifest exists");

        let project = html_project(
            vec![OutputFile::new(
                PathBuf::from("index.html"),
                FileKind::Html(String::from("<html>dev</html>")),
            )],
            Some(PathBuf::from("index.html")),
        );
        let messages = write_project_outputs(
            &project,
            &always_write_options(output_root.clone(), Some(project_dir.clone())),
        )
        .expect_err("foreign profile ownership must fail before mutation");
        assert!(matches!(
            &messages
                .error_diagnostics()
                .next()
                .expect("owner conflict diagnostic should exist")
                .payload,
            DiagnosticPayload::InvalidConfig {
                reason: InvalidConfigReason::OutputManifestOwnerConflict { .. },
                ..
            }
        ));
        assert_eq!(
            fs::read(output_root.join("index.html")).expect("output should remain"),
            previous_output
        );
        assert_eq!(
            fs::read(output_root.join(BUILD_MANIFEST_FILENAME)).expect("manifest should remain"),
            previous_manifest
        );

        fs::remove_dir_all(&root).expect("should remove temp dir");
    }
}

#[test]
fn foreign_profile_with_invalid_utf8_path_record_fails_without_mutation() {
    let root = temp_dir("cleanup_foreign_invalid_utf8_path");
    let project_dir = root.join("project");
    let output_root = project_dir.join("dev");
    fs::create_dir_all(&output_root).expect("should create output root");
    fs::write(output_root.join("index.html"), "existing").expect("should write existing output");
    let manifest_bytes = b"# moth-manifest v4\n# builder: html\n# profile: release\n# managed_extensions: .html,.js,.wasm\nindex.html\ninvalid-\xFF-path.js\n";
    fs::write(output_root.join(BUILD_MANIFEST_FILENAME), manifest_bytes)
        .expect("should write invalid-utf8 manifest");
    let previous_output = fs::read(output_root.join("index.html")).expect("output exists");
    let previous_manifest =
        fs::read(output_root.join(BUILD_MANIFEST_FILENAME)).expect("manifest exists");

    let project = html_project(
        vec![OutputFile::new(
            PathBuf::from("index.html"),
            FileKind::Html(String::from("<html>dev</html>")),
        )],
        Some(PathBuf::from("index.html")),
    );
    let messages = write_project_outputs(
        &project,
        &always_write_options(output_root.clone(), Some(project_dir.clone())),
    )
    .expect_err("foreign owner must fail even when a path record is not UTF-8");
    assert!(matches!(
        &messages
            .error_diagnostics()
            .next()
            .expect("owner conflict diagnostic should exist")
            .payload,
        DiagnosticPayload::InvalidConfig {
            reason: InvalidConfigReason::OutputManifestOwnerConflict { .. },
            ..
        }
    ));
    assert_eq!(
        fs::read(output_root.join("index.html")).expect("output should remain"),
        previous_output
    );
    assert_eq!(
        fs::read(output_root.join(BUILD_MANIFEST_FILENAME)).expect("manifest should remain"),
        previous_manifest
    );

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn case_variant_manifest_paths_enter_recoverable_mode_with_owner() {
    let root = temp_dir("cleanup_case_variant_manifest_reader");
    fs::create_dir_all(&root).expect("should create temp root");

    write_v4_manifest_text(
        &root,
        "html",
        "dev",
        ".html,.js,.wasm",
        &["Pages/app.js", "pages/app.js"],
    );

    assert_eq!(
        read_html_manifest(&root),
        ManifestReadResult::RecoverableWithOwner {
            reason: ManifestRecoveryReason::InvalidMetadata,
            owner: html_owner(BuildProfile::Dev),
        }
    );

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn matching_owner_with_case_variant_manifest_paths_preserves_stale_files() {
    let root = temp_dir("cleanup_case_variant_manifest_matching_owner");
    let project_dir = root.join("project");
    let output_root = project_dir.join("dev");
    fs::create_dir_all(output_root.join("Pages")).expect("should create output directory");
    fs::write(output_root.join("Pages/app.js"), "keep me").expect("should write stale output");
    write_v4_manifest_text(
        &output_root,
        "html",
        "dev",
        ".html,.js,.wasm",
        &["Pages/app.js", "pages/app.js"],
    );

    let project = html_project(
        vec![OutputFile::new(
            PathBuf::from("index.html"),
            FileKind::Html(String::from("<html>Home</html>")),
        )],
        Some(PathBuf::from("index.html")),
    );
    write_project_outputs(
        &project,
        &always_write_options(output_root.clone(), Some(project_dir.clone())),
    )
    .expect("matching owner should rebuild without deleting ambiguous stale paths");

    assert_eq!(
        fs::read(output_root.join("Pages/app.js")).expect("ambiguous stale output should remain"),
        b"keep me"
    );
    assert_eq!(
        read_html_manifest(&output_root),
        valid_manifest(vec![PathBuf::from("index.html")], BuildProfile::Dev)
    );

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn foreign_owner_with_case_variant_manifest_paths_fails_without_mutation() {
    let root = temp_dir("cleanup_case_variant_manifest_foreign_owner");
    let project_dir = root.join("project");
    let output_root = project_dir.join("dev");
    fs::create_dir_all(output_root.join("Pages")).expect("should create output directory");
    fs::write(output_root.join("Pages/app.js"), "keep me").expect("should write stale output");
    write_v4_manifest_text(
        &output_root,
        "html",
        "release",
        ".html,.js,.wasm",
        &["Pages/app.js", "pages/app.js"],
    );
    let previous_output = fs::read(output_root.join("Pages/app.js")).expect("output should exist");
    let previous_manifest =
        fs::read(output_root.join(BUILD_MANIFEST_FILENAME)).expect("manifest should exist");

    let project = html_project(
        vec![OutputFile::new(
            PathBuf::from("index.html"),
            FileKind::Html(String::from("<html>Home</html>")),
        )],
        Some(PathBuf::from("index.html")),
    );
    let messages = write_project_outputs(
        &project,
        &always_write_options(output_root.clone(), Some(project_dir.clone())),
    )
    .expect_err("foreign owner must fail before touching ambiguous manifest outputs");
    assert!(matches!(
        &messages
            .error_diagnostics()
            .next()
            .expect("owner conflict diagnostic should exist")
            .payload,
        DiagnosticPayload::InvalidConfig {
            reason: InvalidConfigReason::OutputManifestOwnerConflict { .. },
            ..
        }
    ));
    assert_eq!(
        fs::read(output_root.join("Pages/app.js")).expect("output should remain"),
        previous_output
    );
    assert_eq!(
        fs::read(output_root.join(BUILD_MANIFEST_FILENAME)).expect("manifest should remain"),
        previous_manifest
    );

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn read_build_manifest_rejects_malformed_v4_metadata() {
    let root = temp_dir("cleanup_ext_malformed_metadata");
    fs::create_dir_all(&root).expect("should create temp root");

    // An explicit v4 owner with an unknown builder is foreign ownership, not ownerless recovery.
    write_v4_manifest_text(&root, "unknown", "dev", ".html,.js,.wasm", &["index.html"]);

    assert_eq!(
        read_html_manifest(&root),
        ManifestReadResult::ForeignOwner {
            reason: ManifestRecoveryReason::InvalidMetadata,
            builder: "unknown".to_string(),
            profile: "dev".to_string(),
        }
    );

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

fn assert_foreign_manifest_owner_fails_without_mutation(
    test_name: &str,
    builder: &str,
    profile: &str,
) {
    let root = temp_dir(test_name);
    let project_dir = root.join("project");
    let output_root = project_dir.join("dev");
    fs::create_dir_all(&output_root).expect("should create output root");
    fs::write(output_root.join("index.html"), "foreign index")
        .expect("should write foreign output");
    fs::write(output_root.join("foreign.js"), "foreign script")
        .expect("should write foreign managed output");
    write_v4_manifest_text(
        &output_root,
        builder,
        profile,
        ".html,.js,.wasm",
        &["foreign.js", "index.html"],
    );

    let previous_index = fs::read(output_root.join("index.html")).expect("index should exist");
    let previous_script = fs::read(output_root.join("foreign.js")).expect("script should exist");
    let previous_manifest =
        fs::read(output_root.join(BUILD_MANIFEST_FILENAME)).expect("manifest should exist");

    let project = html_project(
        vec![OutputFile::new(
            PathBuf::from("index.html"),
            FileKind::Html(String::from("<html>Home</html>")),
        )],
        Some(PathBuf::from("index.html")),
    );
    let messages = write_project_outputs(
        &project,
        &always_write_options(output_root.clone(), Some(project_dir.clone())),
    )
    .expect_err("foreign manifest ownership must fail before output mutation");
    assert!(matches!(
        &messages
            .error_diagnostics()
            .next()
            .expect("owner conflict diagnostic should exist")
            .payload,
        DiagnosticPayload::InvalidConfig {
            reason: InvalidConfigReason::OutputManifestOwnerConflict { .. },
            ..
        }
    ));
    assert_eq!(
        fs::read(output_root.join("index.html")).expect("index should remain"),
        previous_index
    );
    assert_eq!(
        fs::read(output_root.join("foreign.js")).expect("script should remain"),
        previous_script
    );
    assert_eq!(
        fs::read(output_root.join(BUILD_MANIFEST_FILENAME)).expect("manifest should remain"),
        previous_manifest
    );

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[test]
fn unknown_v4_builder_fails_without_mutation() {
    assert_foreign_manifest_owner_fails_without_mutation(
        "cleanup_unknown_v4_builder_no_mutation",
        "foreign-builder",
        "dev",
    );
}

#[test]
fn unknown_v4_profile_fails_without_mutation() {
    assert_foreign_manifest_owner_fails_without_mutation(
        "cleanup_unknown_v4_profile_no_mutation",
        "html",
        "future-profile",
    );
}

#[test]
fn cleanup_extension_mismatch_preserves_stale_files_and_rewrites_manifest() {
    let root = temp_dir("cleanup_ext_mismatch_stale");
    fs::create_dir_all(&root).expect("should create temp root");
    let project_dir = root.join("project");
    fs::create_dir_all(&project_dir).expect("should create project dir");
    let output_root = project_dir.join("dev");
    fs::create_dir_all(output_root.join("about")).expect("should create about output dir");
    fs::create_dir_all(output_root.join("scripts")).expect("should create scripts output dir");

    fs::write(
        output_root.join("about/index.html"),
        "<html>stale about</html>",
    )
    .expect("should write stale html file");
    fs::write(output_root.join("scripts/page.js"), "console.log('stale');")
        .expect("should write stale js file");
    fs::write(output_root.join("notes.txt"), "keep me").expect("should write notes file");

    // The manifest claims a different managed-extension set, so cleanup must enter recoverable
    // mode rather than delete files under a mismatched ownership contract.
    write_v4_manifest_text(
        &output_root,
        "html",
        "dev",
        ".html,.js",
        &[
            "about/index.html",
            "scripts/page.js",
            "notes.txt",
            "index.html",
        ],
    );

    assert_eq!(
        read_html_manifest(&output_root),
        ManifestReadResult::RecoverableWithOwner {
            reason: ManifestRecoveryReason::ManagedExtensionsMismatch {
                manifest_extensions: [".html", ".js"]
                    .iter()
                    .map(|ext| (*ext).to_string())
                    .collect(),
                active_extensions: html_active_extensions(),
            },
            owner: html_owner(BuildProfile::Dev),
        }
    );

    let project = html_project(
        vec![OutputFile::new(
            PathBuf::from("index.html"),
            FileKind::Html(String::from("<html>Home</html>")),
        )],
        Some(PathBuf::from("index.html")),
    );
    write_project_outputs(
        &project,
        &always_write_options(output_root.clone(), Some(project_dir.clone())),
    )
    .expect("build should succeed");

    // Recoverable mode preserves stale files while the extension ownership differs.
    assert!(
        output_root.join("about/index.html").exists(),
        "extension mismatch must preserve stale html files"
    );
    assert!(
        output_root.join("scripts/page.js").exists(),
        "extension mismatch must preserve stale js files"
    );
    assert!(
        output_root.join("notes.txt").exists(),
        "recoverable mode must preserve non-managed file types"
    );

    // The finalize path rewrites the manifest using the active policy, so the next read is valid.
    assert_eq!(
        read_html_manifest(&output_root),
        valid_manifest(vec![PathBuf::from("index.html")], BuildProfile::Dev)
    );

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[test]
fn write_build_manifest_produces_sorted_output() {
    let root = temp_dir("manifest_sorted");
    fs::create_dir_all(&root).expect("should create temp root");

    let paths: HashSet<PathBuf> = [
        PathBuf::from("z/page.js"),
        PathBuf::from("index.html"),
        PathBuf::from("about/index.html"),
    ]
    .into_iter()
    .collect();

    write_build_manifest(
        &root.join(BUILD_MANIFEST_FILENAME),
        &paths,
        html_owner(BuildProfile::Dev),
        &html_cleanup_policy(),
        WriteMode::AlwaysWrite,
        &StringTable::new(),
    )
    .expect("should write manifest");

    let content =
        fs::read_to_string(root.join(BUILD_MANIFEST_FILENAME)).expect("should read manifest file");
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(
        lines,
        vec![
            "# moth-manifest v4",
            "# builder: html",
            "# profile: dev",
            "# managed_extensions: .html,.js,.wasm",
            "about/index.html",
            "index.html",
            "z/page.js",
        ]
    );

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[test]
fn manifest_paths_round_trip_leading_and_interior_spaces() {
    let root = temp_dir("manifest_space_paths");
    fs::create_dir_all(&root).expect("should create temp root");

    let paths: HashSet<PathBuf> = [
        PathBuf::from(" leading.html"),
        PathBuf::from("interior name.html"),
    ]
    .into_iter()
    .collect();
    write_build_manifest(
        &root.join(BUILD_MANIFEST_FILENAME),
        &paths,
        html_owner(BuildProfile::Dev),
        &html_cleanup_policy(),
        WriteMode::AlwaysWrite,
        &StringTable::new(),
    )
    .expect("should write space-preserving manifest");

    assert_eq!(
        read_html_manifest(&root),
        valid_manifest(
            vec![
                PathBuf::from(" leading.html"),
                PathBuf::from("interior name.html"),
            ],
            BuildProfile::Dev,
        )
    );

    fs::write(root.join(" leading.html"), "leading").expect("should write leading-space file");
    fs::write(root.join("interior name.html"), "interior")
        .expect("should write interior-space file");
    let report = remove_manifest_tracked_stale_artifacts(
        &root,
        &HashSet::new(),
        &HashSet::new(),
        &[
            PathBuf::from(" leading.html"),
            PathBuf::from("interior name.html"),
        ],
    );
    assert_eq!(report.removed_paths.len(), 2);
    assert!(!root.join(" leading.html").exists());
    assert!(!root.join("interior name.html").exists());

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn stale_cleanup_ignores_reserved_manifest_paths() {
    let cases = [
        (
            "exact",
            PathBuf::from(".moth_manifest"),
            PathBuf::from(".moth_manifest"),
        ),
        (
            "descendant",
            PathBuf::from(".moth_manifest/child.js"),
            PathBuf::from(".moth_manifest/child.js"),
        ),
        (
            "case_separator",
            PathBuf::from(r".MOTH_MANIFEST\child.js"),
            PathBuf::from(".MOTH_MANIFEST/child.js"),
        ),
    ];

    for (case_name, stale_path, filesystem_path) in cases {
        let root = temp_dir(&format!("cleanup_reserved_manifest_{case_name}"));
        fs::create_dir_all(&root).expect("should create temp root");
        if let Some(parent) = filesystem_path.parent()
            && parent != Path::new("")
        {
            fs::create_dir_all(root.join(parent)).expect("should create reserved parent");
        }
        fs::write(root.join(&filesystem_path), "must remain")
            .expect("should create reserved stale path");

        let report = remove_manifest_tracked_stale_artifacts(
            &root,
            &HashSet::new(),
            &HashSet::new(),
            std::slice::from_ref(&stale_path),
        );
        assert_eq!(report.ignored_paths, vec![stale_path]);
        assert_eq!(
            fs::read(root.join(filesystem_path)).expect("reserved path should remain"),
            b"must remain"
        );

        fs::remove_dir_all(&root).expect("should remove temp root");
    }
}

// -------------------------
//  Output ownership and preflight tests
// -------------------------

#[test]
fn matching_v4_owner_performs_stale_cleanup() {
    let root = temp_dir("v4_matching_cleanup");
    fs::create_dir_all(&root).expect("should create temp root");
    let project_dir = root.join("project");
    fs::create_dir_all(&project_dir).expect("should create project dir");
    let output_root = project_dir.join("dev");

    let project_a = html_project(
        vec![
            OutputFile::new(
                PathBuf::from("index.html"),
                FileKind::Html(String::from("<html>Home</html>")),
            ),
            OutputFile::new(
                PathBuf::from("about/index.html"),
                FileKind::Html(String::from("<html>About</html>")),
            ),
        ],
        Some(PathBuf::from("index.html")),
    );
    write_project_outputs(
        &project_a,
        &always_write_options(output_root.clone(), Some(project_dir.clone())),
    )
    .expect("build A should succeed");

    assert!(output_root.join("about/index.html").exists());

    let project_b = html_project(
        vec![OutputFile::new(
            PathBuf::from("index.html"),
            FileKind::Html(String::from("<html>Home v2</html>")),
        )],
        Some(PathBuf::from("index.html")),
    );
    write_project_outputs(
        &project_b,
        &always_write_options(output_root.clone(), Some(project_dir.clone())),
    )
    .expect("build B should succeed");

    assert!(
        !output_root.join("about/index.html").exists(),
        "matching v4 owner should remove stale files"
    );

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[cfg(unix)]
#[test]
fn stale_cleanup_tracks_canonical_final_file_symlink_destination() {
    use std::os::unix::fs::symlink;

    let root = temp_dir("canonical_file_symlink_cleanup");
    let project_dir = root.join("project");
    let output_root = project_dir.join("dev");
    fs::create_dir_all(&output_root).expect("should create output root");
    fs::write(output_root.join("real.html"), "old target").expect("should create real file target");
    symlink(
        output_root.join("real.html"),
        output_root.join("alias.html"),
    )
    .expect("should create final-file alias");

    let first_project = html_project(
        vec![OutputFile::new(
            PathBuf::from("alias.html"),
            FileKind::Html(String::from("<html>first</html>")),
        )],
        Some(PathBuf::from("alias.html")),
    );
    write_project_outputs(
        &first_project,
        &always_write_options(output_root.clone(), Some(project_dir.clone())),
    )
    .expect("first symlinked build should succeed");

    let second_project = html_project(
        vec![OutputFile::new(
            PathBuf::from("index.html"),
            FileKind::Html(String::from("<html>second</html>")),
        )],
        Some(PathBuf::from("index.html")),
    );
    write_project_outputs(
        &second_project,
        &always_write_options(output_root.clone(), Some(project_dir.clone())),
    )
    .expect("second symlinked build should succeed");

    assert!(!output_root.join("real.html").exists());
    assert!(
        fs::symlink_metadata(output_root.join("alias.html"))
            .expect("alias should remain as a symlink")
            .file_type()
            .is_symlink()
    );
    assert!(output_root.join("index.html").exists());

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[cfg(unix)]
#[test]
fn stale_cleanup_does_not_follow_retargeted_directory_aliases() {
    use std::os::unix::fs::symlink;

    let root = temp_dir("canonical_directory_symlink_cleanup");
    let project_dir = root.join("project");
    let output_root = project_dir.join("dev");
    let old_target = output_root.join("old_target");
    let new_target = output_root.join("new_target");
    fs::create_dir_all(&old_target).expect("should create old target");
    fs::create_dir_all(&new_target).expect("should create new target");
    symlink(&old_target, output_root.join("alias")).expect("should create directory alias");

    let first_project = html_project(
        vec![OutputFile::new(
            PathBuf::from("alias/old.html"),
            FileKind::Html(String::from("<html>first</html>")),
        )],
        Some(PathBuf::from("alias/old.html")),
    );
    write_project_outputs(
        &first_project,
        &always_write_options(output_root.clone(), Some(project_dir.clone())),
    )
    .expect("first directory-alias build should succeed");

    fs::remove_file(output_root.join("alias")).expect("should remove old directory alias");
    symlink(&new_target, output_root.join("alias")).expect("should retarget directory alias");
    fs::write(new_target.join("old.html"), "new target").expect("should create new target file");

    let second_project = html_project(
        vec![OutputFile::new(
            PathBuf::from("index.html"),
            FileKind::Html(String::from("<html>second</html>")),
        )],
        Some(PathBuf::from("index.html")),
    );
    write_project_outputs(
        &second_project,
        &always_write_options(output_root.clone(), Some(project_dir.clone())),
    )
    .expect("second directory-alias build should succeed");

    assert!(!old_target.join("old.html").exists());
    assert_eq!(
        fs::read(new_target.join("old.html")).expect("new target should remain"),
        b"new target"
    );

    fs::remove_dir_all(&root).expect("should remove temp root");
}

#[test]
fn dev_then_release_against_same_v4_root_fails_without_mutation() {
    let root = temp_dir("v4_profile_mismatch");
    fs::create_dir_all(&root).expect("should create temp root");
    let project_dir = root.join("project");
    fs::create_dir_all(&project_dir).expect("should create project dir");
    let output_root = project_dir.join("dev");

    let project = html_project(
        vec![OutputFile::new(
            PathBuf::from("index.html"),
            FileKind::Html(String::from("<html>Dev</html>")),
        )],
        Some(PathBuf::from("index.html")),
    );
    write_project_outputs(
        &project,
        &always_write_options(output_root.clone(), Some(project_dir.clone())),
    )
    .expect("dev build should succeed");
    let previous_output = fs::read(output_root.join("index.html")).expect("output should exist");
    let previous_manifest =
        fs::read(output_root.join(BUILD_MANIFEST_FILENAME)).expect("manifest should exist");

    let release_project = Project {
        output_files: vec![OutputFile::new(
            PathBuf::from("index.html"),
            FileKind::Html(String::from("<html>Release</html>")),
        )],
        entry_page_rel: Some(PathBuf::from("index.html")),
        cleanup_policy: CleanupPolicy::html(),
        warnings: vec![],
    };
    let result = write_project_outputs(
        &release_project,
        &always_write_options_for_profile(
            output_root.clone(),
            Some(project_dir.clone()),
            BuildProfile::Release,
        ),
    );

    let messages = result.expect_err("profile ownership conflict should fail the write");
    assert_eq!(messages.error_count(), 1);
    assert!(matches!(
        &messages
            .error_diagnostics()
            .next()
            .expect("conflict diagnostic should exist")
            .payload,
        DiagnosticPayload::InvalidConfig {
            reason: InvalidConfigReason::OutputManifestOwnerConflict { .. },
            ..
        }
    ));
    assert_eq!(
        fs::read(output_root.join("index.html"))
            .expect("output should remain")
            .as_slice(),
        previous_output.as_slice()
    );
    assert_eq!(
        fs::read(output_root.join(BUILD_MANIFEST_FILENAME))
            .expect("manifest should remain")
            .as_slice(),
        previous_manifest.as_slice()
    );

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[test]
fn builder_owner_conflict_fails_without_mutation() {
    let root = temp_dir("v4_builder_owner_conflict");
    fs::create_dir_all(&root).expect("should create temp root");
    let project_dir = root.join("project");
    fs::create_dir_all(&project_dir).expect("should create project dir");
    let output_root = project_dir.join("dev");

    let project = html_project(
        vec![OutputFile::new(
            PathBuf::from("index.html"),
            FileKind::Html(String::from("<html>HTML</html>")),
        )],
        Some(PathBuf::from("index.html")),
    );
    write_project_outputs(
        &project,
        &always_write_options(output_root.clone(), Some(project_dir.clone())),
    )
    .expect("initial HTML build should succeed");
    let previous_output = fs::read(output_root.join("index.html")).expect("output should exist");
    let previous_manifest =
        fs::read(output_root.join(BUILD_MANIFEST_FILENAME)).expect("manifest should exist");

    let mut string_table = StringTable::new();
    let setting_location = SourceLocation::from_path(Path::new("config.moth"), &mut string_table);
    let conflicting_options = WriteOptions {
        output_plan: OutputPlan::SingleFile(SingleFileOutputPlan {
            output_root: output_root.clone(),
            project_root: Some(project_dir.clone()),
            owner: OutputOwner {
                builder: BuilderKind::Test,
                profile: BuildProfile::Dev,
            },
            setting_location: setting_location.clone(),
        }),
        write_mode: WriteMode::AlwaysWrite,
    };
    let messages = crate::build_system::output::write_project_outputs(
        &project,
        &conflicting_options,
        &string_table,
    )
    .expect_err("a foreign builder must not claim the output root");
    let diagnostic = messages
        .error_diagnostics()
        .next()
        .expect("owner conflict diagnostic should exist");
    assert!(matches!(
        &diagnostic.payload,
        DiagnosticPayload::InvalidConfig {
            reason: InvalidConfigReason::OutputManifestOwnerConflict { .. },
            ..
        }
    ));
    assert_eq!(
        diagnostic.primary_location,
        conflicting_options.output_plan.setting_location().clone()
    );
    let resolved_scope =
        resolve_source_file_path(&diagnostic.primary_location.scope, &messages.string_table);
    assert_eq!(resolved_scope, Path::new("config.moth"));
    assert!(
        format_terse_compiler_messages(&messages)
            .iter()
            .any(|line| line.contains("config.moth")),
        "owner-conflict diagnostics should render their config source scope"
    );
    assert_eq!(
        fs::read(output_root.join("index.html")).expect("output should remain"),
        previous_output
    );
    assert_eq!(
        fs::read(output_root.join(BUILD_MANIFEST_FILENAME)).expect("manifest should remain"),
        previous_manifest
    );

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[test]
fn manifest_reader_returns_manifest_owner_without_comparing_active_owner() {
    let root = temp_dir("v4_builder_mismatch");
    fs::create_dir_all(&root).expect("should create temp root");

    let generic_policy = CleanupPolicy::generic([".html"]);
    let paths: HashSet<PathBuf> = [PathBuf::from("index.html")].into_iter().collect();
    write_build_manifest(
        &root.join(BUILD_MANIFEST_FILENAME),
        &paths,
        OutputOwner {
            builder: BuilderKind::Test,
            profile: BuildProfile::Dev,
        },
        &generic_policy,
        WriteMode::AlwaysWrite,
        &StringTable::new(),
    )
    .expect("should write manifest");

    let manifest = read_build_manifest(&root, &generic_policy);
    assert_eq!(
        manifest,
        ManifestReadResult::Valid(BuildManifest {
            owner: OutputOwner {
                builder: BuilderKind::Test,
                profile: BuildProfile::Dev,
            },
            managed_extensions: [".html"].into_iter().map(String::from).collect(),
            paths: vec![PathBuf::from("index.html")],
        })
    );

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[test]
fn v3_manifest_enters_recoverable_mode() {
    let root = temp_dir("v3_legacy");
    fs::create_dir_all(&root).expect("should create temp root");

    fs::write(
        root.join(BUILD_MANIFEST_FILENAME),
        "# moth-manifest v3\n# builder: html\n# managed_extensions: .html,.js,.wasm\nindex.html\n",
    )
    .expect("should write v3 manifest");

    let html_policy = CleanupPolicy::html();
    assert_eq!(
        read_build_manifest(&root, &html_policy),
        ManifestReadResult::Recoverable {
            reason: ManifestRecoveryReason::UnsupportedVersion,
        },
        "v3 manifests enter recoverable mode because they lack profile identity"
    );

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[test]
fn managed_extension_drift_preserves_old_files() {
    let root = temp_dir("v4_ext_drift");
    fs::create_dir_all(&root).expect("should create temp root");
    let project_dir = root.join("project");
    fs::create_dir_all(&project_dir).expect("should create project dir");
    let output_root = project_dir.join("dev");
    fs::create_dir_all(&output_root).expect("should create output root");

    write_v4_manifest_text(
        &output_root,
        "html",
        "dev",
        ".html,.js",
        &["about/index.html", "index.html"],
    );
    fs::create_dir_all(output_root.join("about")).expect("should create about dir");
    fs::write(output_root.join("about/index.html"), "<html>stale</html>")
        .expect("should write stale file");

    let project = html_project(
        vec![OutputFile::new(
            PathBuf::from("index.html"),
            FileKind::Html(String::from("<html>Home</html>")),
        )],
        Some(PathBuf::from("index.html")),
    );
    write_project_outputs(
        &project,
        &always_write_options(output_root.clone(), Some(project_dir.clone())),
    )
    .expect("build should succeed");

    assert!(
        output_root.join("about/index.html").exists(),
        "extension drift must preserve stale files"
    );

    assert_eq!(
        read_html_manifest(&output_root),
        valid_manifest(vec![PathBuf::from("index.html")], BuildProfile::Dev)
    );

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[test]
fn failed_stale_deletion_remains_owned_in_next_manifest() {
    let root = temp_dir("cleanup_failed_stale_delete");
    let project_dir = root.join("project");
    let output_root = project_dir.join("dev");
    fs::create_dir_all(output_root.join("stale.html")).expect("should create stale directory");
    write_v4_manifest_text(
        &output_root,
        "html",
        "dev",
        ".html,.js,.wasm",
        &["stale.html"],
    );

    let project = html_project(
        vec![OutputFile::new(
            PathBuf::from("index.html"),
            FileKind::Html(String::from("<html>Home</html>")),
        )],
        Some(PathBuf::from("index.html")),
    );
    write_project_outputs(
        &project,
        &always_write_options(output_root.clone(), Some(project_dir.clone())),
    )
    .expect("failed stale deletion should not fail the build");

    let manifest = read_html_manifest(&output_root);
    assert!(matches!(
        manifest,
        ManifestReadResult::Valid(BuildManifest { paths, .. })
            if paths.contains(&PathBuf::from("stale.html"))
                && paths.contains(&PathBuf::from("index.html"))
    ));
    assert!(output_root.join("stale.html").is_dir());

    fs::remove_dir_all(&root).expect("should remove temp dir");
}
