//! Tests for canonical HTML output planning.

use super::*;
use std::path::{Path, PathBuf};

#[test]
fn single_file_route_uses_exact_utf8_stem() {
    let route = derive_logical_html_path(Path::new("main.moth"), None)
        .expect("ordinary single-file route should resolve");

    assert_eq!(route, PathBuf::from("main.html"));
}

#[test]
fn single_file_hash_prefix_strips_cosmetic_hash() {
    let route = derive_logical_html_path(Path::new("@about.moth"), None)
        .expect("hash-prefixed single-file route should resolve");

    assert_eq!(route, PathBuf::from("about.html"));
}

#[test]
fn single_file_missing_stem_is_rejected_not_main() {
    let error = derive_logical_html_path(Path::new("."), None)
        .expect_err("missing single-file stem should be rejected, never fall back to main");

    assert_eq!(
        error.error_type,
        crate::compiler_frontend::compiler_errors::ErrorType::File,
        "missing stem should surface as a File infrastructure error"
    );
}

#[cfg(unix)]
#[test]
fn single_file_non_utf8_stem_is_rejected() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let bad_stem = OsString::from_vec(vec![0xC3, 0x28]);
    let entry = Path::new(&bad_stem).with_extension("moth");

    let error = derive_logical_html_path(&entry, None)
        .expect_err("non-UTF-8 single-file stem should be rejected");

    assert_eq!(
        error.error_type,
        crate::compiler_frontend::compiler_errors::ErrorType::File,
        "non-UTF-8 stem should surface as a File infrastructure error"
    );
}

#[test]
fn single_file_at_only_stem_is_rejected_not_empty_route() {
    let error = derive_logical_html_path(Path::new("@.moth"), None)
        .expect_err("at-only stem should be rejected, not produce an empty route name");

    assert_eq!(
        error.error_type,
        crate::compiler_frontend::compiler_errors::ErrorType::File,
        "at-only stem should surface as a File infrastructure error"
    );
}
