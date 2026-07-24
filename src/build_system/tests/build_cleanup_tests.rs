//! Tests for the core build orchestration and output writer APIs.
// NOTE: temp file creation processes have to be explicitly dropped
// Or these tests will fail on Windows due to attempts to delete non-empty temp directories while files are still open.

use super::*;
use crate::build_system::build::{FileKind, OutputFile, WriteMode};
use crate::build_system::output_cleanup::{
    BuilderKind, ManifestLimitedSafeModeReason, ManifestLoadResult,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use std::fs;
use std::path::{Path, PathBuf};

use crate::build_system::output_cleanup::{
    BUILD_MANIFEST_FILENAME, read_build_manifest, validate_output_root_is_safe,
    write_build_manifest,
};
use crate::compiler_tests::test_support::temp_dir;
use std::collections::{BTreeSet, HashSet};

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
        read_build_manifest(&output_root, &html_cleanup_policy()),
        ManifestLoadResult::Valid {
            paths: vec![
                PathBuf::from("assets/logo.png"),
                PathBuf::from("index.html")
            ],
            builder_kind: BuilderKind::Html,
        }
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

    let manifest = read_build_manifest(&output_root, &html_cleanup_policy());
    assert_eq!(
        manifest,
        ManifestLoadResult::LimitedSafeMode {
            reason: ManifestLimitedSafeModeReason::Missing,
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
        read_build_manifest(&output_root, &html_cleanup_policy()),
        ManifestLoadResult::Valid {
            paths: vec![PathBuf::from("index.html")],
            builder_kind: BuilderKind::Html,
        }
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
        &output_root,
        &manifest_paths,
        &html_cleanup_policy(),
        WriteMode::AlwaysWrite,
        &StringTable::new(),
    )
    .expect("should write v3 manifest");

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
        let result = validate_output_root_is_safe(&dangerous, &project_dir, &StringTable::new());
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

    let result = validate_output_root_is_safe(&root.join("dev"), &root, &StringTable::new());
    assert!(
        result.is_ok(),
        "should accept output root inside project directory"
    );

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[test]
fn cleanup_unreadable_manifest_enters_limited_safe_mode_and_preserves_existing_files() {
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
        read_build_manifest(&output_root, &html_cleanup_policy()),
        ManifestLoadResult::LimitedSafeMode {
            reason: ManifestLimitedSafeModeReason::Unreadable,
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
        read_build_manifest(&output_root, &html_cleanup_policy()),
        ManifestLoadResult::LimitedSafeMode {
            reason: ManifestLimitedSafeModeReason::UnsupportedVersion,
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
        "limited safe mode must preserve non-managed file types"
    );

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[test]
fn read_build_manifest_rejects_builder_mismatch_in_v3_manifest() {
    let root = temp_dir("cleanup_builder_mismatch");
    fs::create_dir_all(&root).expect("should create temp root");

    let paths: HashSet<PathBuf> = [PathBuf::from("index.html")].into_iter().collect();
    write_build_manifest(
        &root,
        &paths,
        &generic_cleanup_policy(),
        WriteMode::AlwaysWrite,
        &StringTable::new(),
    )
    .expect("should write manifest");

    assert_eq!(
        read_build_manifest(&root, &html_cleanup_policy()),
        ManifestLoadResult::LimitedSafeMode {
            reason: ManifestLimitedSafeModeReason::BuilderMismatch {
                manifest_builder_kind: BuilderKind::Generic,
                active_builder_kind: BuilderKind::Html,
            },
        }
    );

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

/// Write a v3 manifest directly so extension-mismatch cases can vary metadata without reusing the
/// active policy's writer.
fn write_v3_manifest_text(root: &Path, builder: &str, extensions_csv: &str, paths: &[&str]) {
    let mut lines = vec![
        String::from("# moth-manifest v3"),
        format!("# builder: {builder}"),
        format!("# managed_extensions: {extensions_csv}"),
    ];
    for path in paths {
        lines.push((*path).to_string());
    }
    fs::write(root.join(BUILD_MANIFEST_FILENAME), lines.join("\n"))
        .expect("should write v3 manifest");
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

    write_v3_manifest_text(&root, "html", ".wasm,.html,.js", &["index.html"]);

    assert_eq!(
        read_build_manifest(&root, &html_cleanup_policy()),
        ManifestLoadResult::Valid {
            paths: vec![PathBuf::from("index.html")],
            builder_kind: BuilderKind::Html,
        }
    );

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[test]
fn read_build_manifest_normalizes_managed_extension_case_and_leading_dot() {
    let root = temp_dir("cleanup_ext_normalize");
    fs::create_dir_all(&root).expect("should create temp root");

    // Uppercase and dotless forms must normalize to the active lowercased dotted set.
    write_v3_manifest_text(&root, "html", "HTML,js,.WASM", &["index.html"]);

    assert_eq!(
        read_build_manifest(&root, &html_cleanup_policy()),
        ManifestLoadResult::Valid {
            paths: vec![PathBuf::from("index.html")],
            builder_kind: BuilderKind::Html,
        }
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

    write_v3_manifest_text(&root, "html", ".html,.js", &["index.html"]);

    assert_eq!(
        read_build_manifest(&root, &html_cleanup_policy()),
        ManifestLoadResult::LimitedSafeMode {
            reason: ManifestLimitedSafeModeReason::ManagedExtensionsMismatch {
                manifest_extensions,
                active_extensions: html_active_extensions(),
            },
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

    write_v3_manifest_text(&root, "html", ".html,.js,.wasm,.css", &["index.html"]);

    assert_eq!(
        read_build_manifest(&root, &html_cleanup_policy()),
        ManifestLoadResult::LimitedSafeMode {
            reason: ManifestLimitedSafeModeReason::ManagedExtensionsMismatch {
                manifest_extensions,
                active_extensions: html_active_extensions(),
            },
        }
    );

    fs::remove_dir_all(&root).expect("should remove temp dir");
}

#[test]
fn read_build_manifest_rejects_malformed_v3_metadata() {
    let root = temp_dir("cleanup_ext_malformed_metadata");
    fs::create_dir_all(&root).expect("should create temp root");

    // A v3 header with an unknown builder name is invalid metadata, distinct from an extension
    // mismatch or builder mismatch between known builders.
    write_v3_manifest_text(&root, "unknown", ".html,.js,.wasm", &["index.html"]);

    assert_eq!(
        read_build_manifest(&root, &html_cleanup_policy()),
        ManifestLoadResult::LimitedSafeMode {
            reason: ManifestLimitedSafeModeReason::InvalidMetadata,
        }
    );

    fs::remove_dir_all(&root).expect("should remove temp dir");
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

    // The manifest claims a different managed-extension set, so cleanup must enter limited safe
    // mode rather than delete files under a mismatched ownership contract.
    write_v3_manifest_text(
        &output_root,
        "html",
        ".html,.js",
        &[
            "about/index.html",
            "scripts/page.js",
            "notes.txt",
            "index.html",
        ],
    );

    assert_eq!(
        read_build_manifest(&output_root, &html_cleanup_policy()),
        ManifestLoadResult::LimitedSafeMode {
            reason: ManifestLimitedSafeModeReason::ManagedExtensionsMismatch {
                manifest_extensions: [".html", ".js"]
                    .iter()
                    .map(|ext| (*ext).to_string())
                    .collect(),
                active_extensions: html_active_extensions(),
            },
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

    // Limited safe mode preserves stale files while the extension ownership differs.
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
        "limited safe mode must preserve non-managed file types"
    );

    // The finalize path rewrites the manifest using the active policy, so the next read is valid.
    assert_eq!(
        read_build_manifest(&output_root, &html_cleanup_policy()),
        ManifestLoadResult::Valid {
            paths: vec![PathBuf::from("index.html")],
            builder_kind: BuilderKind::Html,
        }
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
        &root,
        &paths,
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
            "# moth-manifest v3",
            "# builder: html",
            "# managed_extensions: .html,.js,.wasm",
            "about/index.html",
            "index.html",
            "z/page.js",
        ]
    );

    fs::remove_dir_all(&root).expect("should remove temp dir");
}
