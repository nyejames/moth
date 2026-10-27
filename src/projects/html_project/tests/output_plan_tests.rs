//! Tests for canonical HTML output planning.

use super::*;
use crate::compiler_frontend::compiler_errors::InfrastructureFailureKind;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

#[test]
fn single_file_route_uses_exact_utf8_stem() {
    let route = derive_logical_html_path(Path::new("main.moth"), None)
        .expect("ordinary single-file route should resolve");

    assert_eq!(route.logical_html_path, PathBuf::from("main.html"));
    assert_eq!(route.route_base, PathBuf::from("main"));
    assert_eq!(route.route_segment.as_deref(), Some(OsStr::new("main")));
}

#[test]
fn single_file_hash_prefix_strips_cosmetic_hash() {
    let route = derive_logical_html_path(Path::new("@about.moth"), None)
        .expect("hash-prefixed single-file route should resolve");

    assert_eq!(route.logical_html_path, PathBuf::from("about.html"));
    assert_eq!(route.route_base, PathBuf::from("about"));
    assert_eq!(route.route_segment.as_deref(), Some(OsStr::new("about")));
}

#[test]
fn single_file_index_route_uses_homepage_projections() {
    let route = derive_logical_html_path(Path::new("index.moth"), None)
        .expect("index single-file route should resolve");

    assert_eq!(route.logical_html_path, PathBuf::from("index.html"));
    assert!(route.route_base.as_os_str().is_empty());
    assert_eq!(route.route_segment, None);
}

#[test]
fn single_file_missing_stem_is_rejected_not_main() {
    let error = derive_logical_html_path(Path::new("."), None)
        .expect_err("missing single-file stem should be rejected, never fall back to main");

    let LogicalHtmlPathError::Infrastructure(error) = error else {
        panic!("a missing stem must remain a filesystem failure");
    };
    assert_eq!(
        error.kind,
        InfrastructureFailureKind::Filesystem,
        "missing stem should surface as a File infrastructure error"
    );
}

#[test]
fn directory_route_records_folder_and_title_projections() {
    let route = derive_logical_html_path(
        Path::new("src/docs/basics/@page.moth"),
        Some(Path::new("src")),
    )
    .expect("nested directory route should resolve");

    assert_eq!(
        route.logical_html_path,
        PathBuf::from("docs/basics/index.html")
    );
    assert_eq!(route.route_base, PathBuf::from("docs/basics"));
    assert_eq!(route.route_segment.as_deref(), Some(OsStr::new("basics")));
}

#[cfg(unix)]
#[test]
fn single_file_non_utf8_stem_is_rejected() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let bad_stem = OsString::from_vec(vec![0xC3, 0x28]);
    let entry = Path::new(&bad_stem).with_extension("moth");
    let error = derive_logical_html_path(&entry, None)
        .expect_err("non-UTF-8 single-file stems must remain filesystem failures");

    let LogicalHtmlPathError::Infrastructure(error) = error else {
        panic!("a non-UTF-8 stem must remain a filesystem failure");
    };

    assert_eq!(
        error.kind,
        InfrastructureFailureKind::Filesystem,
        "non-UTF-8 stem should surface as a File infrastructure error"
    );
}

#[test]
fn single_file_at_only_stem_is_a_diagnostic() {
    let error = derive_logical_html_path(Path::new("@.moth"), None)
        .expect_err("an at-only stem must not produce an empty route name");

    assert!(
        matches!(error, LogicalHtmlPathError::EmptyRouteName(_)),
        "an authored empty route name must be a diagnostic, not infrastructure"
    );
}
