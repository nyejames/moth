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

    assert_eq!(
        error.kind,
        InfrastructureFailureKind::Filesystem,
        "missing stem should surface as a File infrastructure error"
    );
}

#[test]
fn directory_route_outside_source_root_reports_both_paths() {
    let error = derive_logical_html_path(
        Path::new("project/src/@entry.moth"),
        Some(Path::new("project/pages")),
    )
    .expect_err("an entry outside the selected source root must be rejected");

    assert_eq!(
        error.message.as_ref(),
        "HTML entry 'project/src/@entry.moth' is not inside the selected source root 'project/pages'."
    );
}

#[cfg(unix)]
#[test]
fn directory_route_outside_source_root_renders_backslashes_as_portable_text() {
    let error = derive_logical_html_path(
        Path::new("/project/src\\pages/@entry.moth"),
        Some(Path::new("/project/src")),
    )
    .expect_err("a literal backslash component must not match the source root");

    assert_eq!(
        error.message.as_ref(),
        "HTML entry '/project/src/pages/@entry.moth' is not inside the selected source root '/project/src'."
    );
}

#[test]
fn directory_route_records_folder_and_title_projections() {
    let route = derive_logical_html_path(
        Path::new("src/docs/basics/@entry.moth"),
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

    assert_eq!(
        error.kind,
        InfrastructureFailureKind::Filesystem,
        "non-UTF-8 stem should surface as a File infrastructure error"
    );
}
