//! Path display helpers for diagnostic rendering.
//!
//! WHAT: converts explicit host paths into stable user-facing relative text and file URLs.
//! WHY: rendered diagnostics show paths relative to a project or display root, so both sides need
//! the Windows verbatim spelling normalized before comparison. Source links also need an RFC 3986
//! file URL, which is a different boundary from display text. Source identity and typed path facts
//! stay with their diagnostic owners.

use crate::compiler_frontend::utilities::basic::normalize_windows_verbatim_path;
use crate::compiler_frontend::utilities::portable_path_display::portable_path_text;
use std::path::{Component, Path};

pub(crate) fn relative_display_path_from_root(scope: &Path, root: &Path) -> String {
    let normalized_scope = normalize_windows_verbatim_path(scope);
    let normalized_root = normalize_windows_verbatim_path(root);

    let display_path = normalized_scope
        .strip_prefix(&normalized_root)
        .unwrap_or(&normalized_scope);

    // A path equal to the display root is still a real host path.
    if display_path.as_os_str().is_empty() {
        ".".to_owned()
    } else {
        portable_path_text(display_path)
    }
}

/// Build the `file:` URL for one absolute host path.
///
/// WHAT: converts an absolute native path into an RFC 3986 file URL and percent-encodes each
/// path segment from its UTF-8 bytes.
/// WHY: the dev-server source link must address the actual host file. HTML escaping is not URL
/// encoding: a raw `#` or `?` becomes a fragment or query, a raw `%` is read as an escape and a
/// raw space is invalid. The lossy display formatter cannot provide this, and the frontend must
/// not depend on the build system's output URL encoder.
///
/// Returns `None` for relative paths, paths with components that are not UTF-8, Windows prefixes
/// that are neither a drive nor a UNC share, and UNC paths whose server is `localhost`, which
/// browsers treat as a local file host rather than a share. The renderer then shows the portable
/// label without a misleading link.
pub(crate) fn file_href_from_path(path: &Path) -> Option<String> {
    // Verbatim prefixes are a Windows spelling detail, so only Windows pays to strip them.
    #[cfg(windows)]
    let normalized = normalize_windows_verbatim_path(path);
    #[cfg(windows)]
    let path = normalized.as_path();

    if !path.is_absolute() {
        return None;
    }

    let mut components = path.components();

    // Windows drive and UNC prefixes define the URL drive or authority. A Unix absolute path
    // starts at the root with an empty authority.
    #[cfg(windows)]
    let mut href = match components.next()? {
        Component::Prefix(prefix) => match prefix.kind() {
            std::path::Prefix::Disk(letter) => {
                let mut href = String::from("file:///");
                href.push(char::from(letter));
                href.push(':');
                href
            }
            std::path::Prefix::UNC(server, share) => {
                let server = server.to_str()?;

                // WHATWG file-URL parsing treats `localhost` as an empty host, so the resulting
                // URL would address a local path instead of the network share.
                if server.eq_ignore_ascii_case("localhost") {
                    return None;
                }

                let mut href = String::from("file://");
                push_percent_encoded_url_path_segment(&mut href, server);
                href.push('/');
                push_percent_encoded_url_path_segment(&mut href, share.to_str()?);
                href
            }
            _ => return None,
        },
        _ => return None,
    };
    #[cfg(unix)]
    let mut href = match components.next()? {
        Component::RootDir => String::from("file://"),
        _ => return None,
    };

    // A Windows prefix is followed by its root; a Unix path starts at its root.
    #[cfg(windows)]
    if !matches!(components.next()?, Component::RootDir) {
        return None;
    }

    let mut has_segment = false;
    for component in components {
        let segment = match component {
            Component::Normal(segment) => segment.to_str()?,
            Component::CurDir => ".",
            Component::ParentDir => "..",
            Component::RootDir | Component::Prefix(_) => return None,
        };
        href.push('/');
        push_percent_encoded_url_path_segment(&mut href, segment);
        has_segment = true;
    }

    // A bare Unix root needs the empty-authority slash; a drive root keeps the slash that
    // separates the drive from its path. A UNC share root needs no trailing slash.
    #[cfg(unix)]
    if !has_segment {
        href.push('/');
    }
    #[cfg(windows)]
    if !has_segment && href.starts_with("file:///") {
        href.push('/');
    }

    Some(href)
}

/// Append one percent-encoded file-URL path segment to `href`.
///
/// WHAT: leaves RFC 3986 unreserved characters (`ALPHA`, `DIGIT`, `-`, `.`, `_`, `~`) readable and
/// emits every other UTF-8 byte as an uppercase percent escape.
/// WHY: the strict unreserved set keeps the URL unambiguous even though some encoded bytes, such
/// as `&` and `'`, would be legal in a path segment. Appending into the destination URL avoids a
/// temporary allocation per segment. URL encoding stays here rather than borrowing the build
/// system's output encoder, which belongs to the output-path owner.
fn push_percent_encoded_url_path_segment(href: &mut String, segment: &str) {
    const HEX_DIGITS: &[u8; 16] = b"0123456789ABCDEF";

    for byte in segment.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            href.push(char::from(byte));
        } else {
            href.push('%');
            href.push(char::from(HEX_DIGITS[usize::from(byte >> 4)]));
            href.push(char::from(HEX_DIGITS[usize::from(byte & 0x0F)]));
        }
    }
}
