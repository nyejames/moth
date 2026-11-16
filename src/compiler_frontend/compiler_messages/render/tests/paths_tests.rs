//! Contracts for render-owned display paths and file URLs.

use crate::compiler_frontend::compiler_messages::render::{
    file_href_from_path, relative_display_path_from_root,
};
use std::path::Path;

#[test]
fn relative_display_path_strips_root_prefix() {
    let root = Path::new("/workspace/project");
    let scope = Path::new("/workspace/project/src/main.moth");

    let relative = relative_display_path_from_root(scope, root);

    assert_eq!(relative, "src/main.moth");
}

#[test]
fn path_equal_to_display_root_is_not_empty() {
    let path = Path::new("working-directory");
    assert_eq!(relative_display_path_from_root(path, path), ".");
}

#[cfg(unix)]
#[test]
fn file_href_uses_an_empty_authority_and_escapes_a_space() {
    assert_eq!(
        file_href_from_path(Path::new("/workspace/with space.moth")).as_deref(),
        Some("file:///workspace/with%20space.moth")
    );
    assert_eq!(
        file_href_from_path(Path::new("/")).as_deref(),
        Some("file:///")
    );
}

/// `?` cannot name a real Windows file, so the encoding contract is asserted directly on the
/// renderer's URL operation. The dev-server page test exercises the rest end to end.
#[cfg(unix)]
#[test]
fn file_href_percent_encodes_reserved_segment_bytes() {
    for (path, expected) in [
        ("/workspace/100%.moth", "file:///workspace/100%25.moth"),
        (
            "/workspace/issue#1.moth",
            "file:///workspace/issue%231.moth",
        ),
        ("/workspace/what?.moth", "file:///workspace/what%3F.moth"),
        ("/workspace/a&b.moth", "file:///workspace/a%26b.moth"),
        ("/workspace/a'b.moth", "file:///workspace/a%27b.moth"),
    ] {
        assert_eq!(
            file_href_from_path(Path::new(path)).as_deref(),
            Some(expected),
            "{path}"
        );
    }
}

#[cfg(unix)]
#[test]
fn file_href_encodes_unicode_as_utf8_percent_bytes() {
    assert_eq!(
        file_href_from_path(Path::new("/workspace/日本語.moth")).as_deref(),
        Some("file:///workspace/%E6%97%A5%E6%9C%AC%E8%AA%9E.moth")
    );
}

#[cfg(unix)]
#[test]
fn file_href_keeps_unreserved_characters_readable() {
    assert_eq!(
        file_href_from_path(Path::new("/workspace/AZaz09-._~")).as_deref(),
        Some("file:///workspace/AZaz09-._~")
    );
}

/// A literal backslash is an ordinary Unix filename byte, not a separator.
#[cfg(unix)]
#[test]
fn file_href_percent_encodes_a_literal_backslash_component() {
    assert_eq!(
        file_href_from_path(Path::new("/workspace/a\\b.moth")).as_deref(),
        Some("file:///workspace/a%5Cb.moth")
    );
}

#[test]
fn file_href_rejects_relative_paths() {
    assert_eq!(file_href_from_path(Path::new("src/main.moth")), None);
    assert_eq!(file_href_from_path(Path::new("./src/main.moth")), None);
}

#[cfg(unix)]
#[test]
fn file_href_rejects_non_utf8_paths() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let path = Path::new("/workspace").join(OsString::from_vec(b"bad-\xFF.moth".to_vec()));
    assert_eq!(file_href_from_path(&path), None);
}

#[cfg(windows)]
#[test]
fn file_href_uses_drive_url_syntax() {
    assert_eq!(
        file_href_from_path(Path::new(r"C:\work dir\moth#1\main.moth")).as_deref(),
        Some("file:///C:/work%20dir/moth%231/main.moth")
    );
    assert_eq!(
        file_href_from_path(Path::new(r"C:\")).as_deref(),
        Some("file:///C:/")
    );
}

#[cfg(windows)]
#[test]
fn file_href_normalizes_verbatim_drive_prefixes() {
    assert_eq!(
        file_href_from_path(Path::new(r"\\?\C:\work dir\main.moth")).as_deref(),
        Some("file:///C:/work%20dir/main.moth")
    );
}

#[cfg(windows)]
#[test]
fn file_href_uses_unc_server_and_share_as_authority_and_first_segment() {
    for path in [
        r"\\server\share\dir with space\main.moth",
        r"\\?\UNC\server\share\dir with space\main.moth",
    ] {
        assert_eq!(
            file_href_from_path(Path::new(path)).as_deref(),
            Some("file://server/share/dir%20with%20space/main.moth"),
            "{path}"
        );
    }

    // A share root already supplies the first path segment, so no trailing slash is added.
    assert_eq!(
        file_href_from_path(Path::new(r"\\server\share")).as_deref(),
        Some("file://server/share")
    );
}

/// Browsers treat a `localhost` file-URL host as the local machine, so linking to it would
/// address a local path instead of the UNC share.
#[cfg(windows)]
#[test]
fn file_href_rejects_localhost_unc_servers() {
    for path in [
        r"\\localhost\share\main.moth",
        r"\\LocalHost\share\main.moth",
        r"\\?\UNC\localhost\share\main.moth",
    ] {
        assert_eq!(file_href_from_path(Path::new(path)), None, "{path}");
    }
}
