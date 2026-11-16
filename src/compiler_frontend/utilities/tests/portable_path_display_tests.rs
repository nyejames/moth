//! Tests for the sole host-path display formatter and Windows verbatim normalization.
//!
//! Explicit expected strings keep the formatter and the normalizer independently pinned so a
//! shared bug cannot make both helpers agree with each other.

use crate::compiler_frontend::utilities::basic::normalize_windows_verbatim_path;
use crate::compiler_frontend::utilities::portable_path_display::portable_path_text;
use std::path::Path;
#[cfg(windows)]
use std::path::PathBuf;

#[test]
fn portable_path_text_joins_native_components_with_forward_slashes() {
    let native_path = Path::new("src").join("docs").join("main.moth");

    assert_eq!(portable_path_text(&native_path), "src/docs/main.moth");
}

#[test]
fn portable_path_text_rewrites_backslash_separated_input() {
    assert_eq!(
        portable_path_text(Path::new(r"src\docs\main.moth")),
        "src/docs/main.moth"
    );
}

#[cfg(unix)]
#[test]
fn portable_path_text_replaces_non_utf8_bytes_lossily() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let non_utf8_name = OsStr::from_bytes(b"bad\xFF.moth");

    assert_eq!(
        portable_path_text(Path::new(non_utf8_name)),
        "bad\u{FFFD}.moth"
    );
}

#[cfg(windows)]
#[test]
fn portable_path_text_strips_verbatim_disk_prefix() {
    assert_eq!(
        portable_path_text(Path::new(r"\\?\C:\work\main.moth")),
        "C:/work/main.moth"
    );
}

#[cfg(windows)]
#[test]
fn portable_path_text_strips_verbatim_unc_prefix() {
    assert_eq!(
        portable_path_text(Path::new(r"\\?\UNC\server\share\dir\main.moth")),
        "//server/share/dir/main.moth"
    );
}

#[cfg(windows)]
#[test]
fn portable_path_display_normalizer_preserves_parent_components() {
    assert_eq!(
        normalize_windows_verbatim_path(Path::new(r"\\?\C:\a\..\b")),
        PathBuf::from(r"C:\a\..\b")
    );
}

#[cfg(windows)]
#[test]
fn portable_path_display_normalizer_strips_verbatim_drive_root() {
    assert_eq!(
        normalize_windows_verbatim_path(Path::new(r"\\?\C:\")),
        PathBuf::from(r"C:\")
    );
}

#[cfg(windows)]
#[test]
fn portable_path_display_normalizer_rewrites_verbatim_unc_without_duplication() {
    assert_eq!(
        normalize_windows_verbatim_path(Path::new(r"\\?\UNC\server\share\dir")),
        PathBuf::from(r"\\server\share\dir")
    );
}

#[cfg(not(windows))]
#[test]
fn portable_path_display_normalizer_is_identity_off_windows() {
    let path = Path::new(r"\\?\C:\a\..\b");

    assert_eq!(normalize_windows_verbatim_path(path), path);
}
