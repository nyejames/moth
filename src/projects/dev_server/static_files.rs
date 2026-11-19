//! Static file and HTML injection helpers for the dev server.
//!
//! This module resolves safe output-relative paths, maps content types and installs the
//! development client before application bootstrap scripts can call its outcome hooks. It owns
//! the request URL boundary: request paths are percent-decoded once before lookup and redirect
//! locations re-encode the decoded page name.

use crate::build_system::output::{parse_native_relative_path, percent_encode_url_path};
use crate::projects::dev_server::dev_client::{DEV_CLIENT_MARKER, dev_client_snippet};
use crate::projects::dev_server::state::{ServedArtifactSnapshot, ServedHtmlEntrySnapshot};
use crate::projects::routing::{HtmlSiteConfig, PageUrlStyle, prefix_origin};
use std::path::{Component, Path, PathBuf};

use std::sync::Arc;
/// Classification used to keep page-routing behavior separate from exact asset serving behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedRequestKind {
    PageHtml,
    Asset,
}

/// Result of resolving one HTTP request path against the dev output directory.
#[derive(Debug, PartialEq, Eq)]
pub enum ResolvedRequest {
    File {
        path: PathBuf,
        kind: ResolvedRequestKind,
    },
    Redirect {
        location: String,
    },
    MissingEntryPage,
    NotFound,
    /// The request path carried a malformed percent escape or decoded to invalid UTF-8.
    /// Rejected as a bad request rather than reported as missing.
    MalformedPath,
    /// The decoded request path cannot safely name a file inside the output directory, so it is
    /// treated as missing.
    InvalidPath,
}

pub fn inject_dev_client(html: &str, origin: &str, build_version: u64, entry: &str) -> String {
    if html.contains(DEV_CLIENT_MARKER) {
        return html.to_owned();
    }

    let snippet = dev_client_snippet(origin, build_version, Some(entry));
    let lowercase_html = html.to_ascii_lowercase();
    // Install at the start of the generated head, before even head-local classic scripts.
    // This also avoids mistaking commented-out script tags in authored head content for code.
    let insertion_index = lowercase_html
        .find("<head>")
        .map(|index| index + "<head>".len())
        .unwrap_or_else(|| {
            [
                lowercase_html.find("<script"),
                lowercase_html.find("</head>"),
                lowercase_html.rfind("</body>"),
            ]
            .into_iter()
            .flatten()
            .min()
            .unwrap_or(html.len())
        });

    let mut injected = String::with_capacity(html.len() + snippet.len());
    injected.push_str(&html[..insertion_index]);
    injected.push_str(&snippet);
    injected.push_str(&html[insertion_index..]);
    injected
}

/// Reports name exact output pages, not URL aliases or files outside the output root.
pub fn is_known_entry(entry: &str, output_dir: &Path) -> bool {
    if entry.is_empty()
        || entry.contains(['\\', '\0'])
        || entry.split('/').any(|part| matches!(part, "" | "." | ".."))
        || Path::new(entry)
            .extension()
            .and_then(|extension| extension.to_str())
            != Some("html")
    {
        return false;
    }

    let Ok(path) = resolve_relative_path(Path::new(entry), output_dir) else {
        return false;
    };
    let (Ok(output_root), Ok(page)) = (output_dir.canonicalize(), path.canonicalize()) else {
        return false;
    };

    page.starts_with(output_root) && page.is_file()
}

/// Freeze the output-relative HTML paths that passed path and containment validation.
pub fn validated_html_entries(
    output_dir: &Path,
    artifacts: &ServedArtifactSnapshot,
) -> ServedHtmlEntrySnapshot {
    Arc::new(
        artifacts
            .keys()
            .filter_map(|relative_path| {
                // The key is the validated output spelling: the dev client echoes this entry and
                // the report ledger compares the same spelling.
                let entry = parse_native_relative_path(relative_path)
                    .ok()?
                    .portable_spelling();
                is_known_entry(&entry, output_dir).then_some(entry)
            })
            .collect(),
    )
}

pub fn content_type_for_path(path: &Path) -> &'static str {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("html") | Some("htm") => "text/html; charset=utf-8",
        Some("js") => "application/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("wasm") => "application/wasm",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("svg") => "image/svg+xml",
        Some("ico") => "image/x-icon",
        Some("txt") => "text/plain; charset=utf-8",
        Some("webmanifest") => "application/manifest+json; charset=utf-8",
        _ => "application/octet-stream",
    }
}

pub fn is_html_content_type(content_type: &str) -> bool {
    content_type.starts_with("text/html")
}

/// Failure to map a raw HTTP request path onto decoded output text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RequestPathError {
    /// A percent escape is malformed or the decoded path is not valid UTF-8.
    MalformedEncoding,
    /// The decoded path is not a safe site-local request path.
    UnsafePath,
}

/// Decode one raw site-local request path into the text used for page and asset lookup.
///
/// WHAT: splits the raw path on `/` before decoding each segment into one buffer and validating
/// the decoded bytes.
/// WHY: browsers send reserved characters percent-encoded, while `/` must stay structural.
/// Decoding after splitting exposes encoded separators, dot segments and NUL to the traversal
/// checks, and each decoded segment maps onto exactly one native path component.
fn decode_request_path(raw_request_path: &str) -> Result<String, RequestPathError> {
    let Some(path_after_root) = raw_request_path.strip_prefix('/') else {
        return Err(RequestPathError::UnsafePath);
    };
    // A trailing separator is significant to routing, but an interior empty segment (`//`) would
    // collapse when mapped onto a native path and is rejected below.
    let has_trailing_slash = path_after_root.ends_with('/');

    let mut decoded_path = Vec::with_capacity(raw_request_path.len());
    decoded_path.push(b'/');

    // Omitting the trailing empty segment avoids treating `/about/` as an interior empty segment;
    // the trailing separator is restored below.
    for (index, raw_segment) in path_after_root.split_terminator('/').enumerate() {
        if index > 0 {
            decoded_path.push(b'/');
        }

        let segment_start = decoded_path.len();
        percent_decode_segment_into(raw_segment, &mut decoded_path)?;

        // A decoded separator or NUL would let the URL spelling describe a different native path
        // than the containment checks validated.
        let decoded_segment = &decoded_path[segment_start..];
        if decoded_segment.is_empty()
            || matches!(decoded_segment, b"." | b"..")
            || decoded_segment
                .iter()
                .any(|&byte| matches!(byte, b'/' | b'\\' | b'\0'))
        {
            return Err(RequestPathError::UnsafePath);
        }
    }

    if has_trailing_slash {
        decoded_path.push(b'/');
    }

    // Decoded separators are rejected per segment, so one whole-path UTF-8 check is equivalent
    // to checking each segment: an inserted `/` cannot be part of a multibyte sequence.
    String::from_utf8(decoded_path).map_err(|_| RequestPathError::MalformedEncoding)
}

/// Append one percent-decoded request path segment to the decoded path buffer.
fn percent_decode_segment_into(
    segment: &str,
    decoded_path: &mut Vec<u8>,
) -> Result<(), RequestPathError> {
    let bytes = segment.as_bytes();
    let mut index = 0;

    while index < bytes.len() {
        let byte = bytes[index];
        if byte != b'%' {
            decoded_path.push(byte);
            index += 1;
            continue;
        }

        let Some(high) = bytes.get(index + 1).and_then(|byte| hex_digit_value(*byte)) else {
            return Err(RequestPathError::MalformedEncoding);
        };
        let Some(low) = bytes.get(index + 2).and_then(|byte| hex_digit_value(*byte)) else {
            return Err(RequestPathError::MalformedEncoding);
        };
        decoded_path.push((high << 4) | low);
        index += 3;
    }

    Ok(())
}

fn hex_digit_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Resolve a request using the effective HTML routing policy from the latest successful build.
///
/// WHAT: decodes the raw site-local request path once, applies strict path sanitization, exact-file
/// asset lookup, and page canonicalization.
/// WHY: page routes and assets must remain separate so canonical redirects never affect assets.
#[cfg(test)]
pub fn resolve_request(
    request_path: &str,
    request_query: Option<&str>,
    output_dir: &Path,
    entry_page_rel: Option<&Path>,
    config: HtmlSiteConfig,
) -> ResolvedRequest {
    resolve_request_with_file_exists(
        request_path,
        request_query,
        output_dir,
        entry_page_rel,
        config,
        |path| path.is_file(),
    )
}

pub(crate) fn resolve_request_with_file_exists(
    request_path: &str,
    request_query: Option<&str>,
    output_dir: &Path,
    entry_page_rel: Option<&Path>,
    config: HtmlSiteConfig,
    file_exists: impl Fn(&Path) -> bool,
) -> ResolvedRequest {
    let decoded_request_path = match decode_request_path(request_path) {
        Ok(decoded_request_path) => decoded_request_path,
        Err(RequestPathError::MalformedEncoding) => return ResolvedRequest::MalformedPath,
        Err(RequestPathError::UnsafePath) => return ResolvedRequest::InvalidPath,
    };
    let request_path = decoded_request_path.as_str();

    if request_path == "/" {
        let Some(entry_page) = entry_page_rel else {
            return ResolvedRequest::MissingEntryPage;
        };
        return match resolve_relative_path(entry_page, output_dir) {
            Ok(path) if file_exists(&path) => ResolvedRequest::File {
                path,
                kind: ResolvedRequestKind::PageHtml,
            },
            Ok(_) => ResolvedRequest::NotFound,
            Err(_) => ResolvedRequest::InvalidPath,
        };
    }

    // Step 1: explicit `/index.html` forms are page-route aliases and are resolved before
    // generic exact-file lookup so canonical page redirects stay deterministic.
    if let Some(page_base) = page_base_from_index_alias(request_path) {
        let page_file_rel = page_file_relative_path(&page_base);
        let page_file_path = match resolve_relative_path(&page_file_rel, output_dir) {
            Ok(path) => path,
            Err(_) => return ResolvedRequest::InvalidPath,
        };

        if !file_exists(&page_file_path) {
            return ResolvedRequest::NotFound;
        }

        if config.redirect_index_html {
            let redirect_target =
                index_alias_redirect_target(&page_base, config.page_url_style, &config.origin);
            return ResolvedRequest::Redirect {
                location: with_query_string(redirect_target, request_query),
            };
        }

        return ResolvedRequest::File {
            path: page_file_path,
            kind: ResolvedRequestKind::PageHtml,
        };
    }

    let has_trailing_slash = request_path.ends_with('/');

    // Step 2: exact existing files are served directly (assets and literal extensionless files).
    if !has_trailing_slash {
        let exact_file_path = match resolve_request_file_path(request_path, output_dir) {
            Ok(path) => path,
            Err(_) => return ResolvedRequest::InvalidPath,
        };

        if file_exists(&exact_file_path) {
            return ResolvedRequest::File {
                path: exact_file_path,
                kind: ResolvedRequestKind::Asset,
            };
        }
    }

    // Step 3: directory-backed page resolution (`/about` or `/about/` -> `about/index.html`).
    let page_base = page_base_from_request_path(request_path);
    let page_file_rel = page_file_relative_path(&page_base);
    let page_file_path = match resolve_relative_path(&page_file_rel, output_dir) {
        Ok(path) => path,
        Err(_) => return ResolvedRequest::InvalidPath,
    };

    if !file_exists(&page_file_path) {
        return ResolvedRequest::NotFound;
    }

    if let Some(canonical) = canonical_redirect_target(
        request_path,
        &page_base,
        config.page_url_style,
        &config.origin,
    ) {
        return ResolvedRequest::Redirect {
            location: with_query_string(canonical, request_query),
        };
    }

    ResolvedRequest::File {
        path: page_file_path,
        kind: ResolvedRequestKind::PageHtml,
    }
}

fn resolve_request_file_path(request_path: &str, output_dir: &Path) -> Result<PathBuf, ()> {
    let path_without_slash = request_path.trim_start_matches('/');
    if path_without_slash.is_empty() {
        return Err(());
    }

    resolve_relative_path(Path::new(path_without_slash), output_dir).map_err(|_| ())
}

fn page_base_from_index_alias(request_path: &str) -> Option<String> {
    if request_path == "/index.html" {
        return Some(String::from("/"));
    }

    let without_suffix = request_path.strip_suffix("/index.html")?;
    if without_suffix.is_empty() {
        Some(String::from("/"))
    } else {
        Some(without_suffix.to_string())
    }
}

fn page_base_from_request_path(request_path: &str) -> String {
    if request_path == "/" {
        return String::from("/");
    }

    let trimmed = request_path.trim_end_matches('/');
    if trimmed.is_empty() {
        String::from("/")
    } else {
        trimmed.to_string()
    }
}

fn page_file_relative_path(page_base: &str) -> PathBuf {
    if page_base == "/" {
        return PathBuf::from("index.html");
    }

    let mut relative = PathBuf::from(page_base.trim_start_matches('/'));
    relative.push("index.html");
    relative
}

fn canonical_redirect_target(
    site_local_request_path: &str,
    page_base: &str,
    style: PageUrlStyle,
    origin: &str,
) -> Option<String> {
    if style == PageUrlStyle::Ignore {
        return None;
    }

    let site_local_canonical = site_local_canonical_page_url(page_base, style);
    if site_local_request_path == site_local_canonical {
        None
    } else {
        Some(prefix_origin(
            origin,
            &percent_encode_url_path(&site_local_canonical),
        ))
    }
}

fn site_local_canonical_page_url(page_base: &str, style: PageUrlStyle) -> String {
    if page_base == "/" {
        return String::from("/");
    }

    match style {
        PageUrlStyle::TrailingSlash => format!("{page_base}/"),
        PageUrlStyle::NoTrailingSlash | PageUrlStyle::Ignore => page_base.to_string(),
    }
}

fn index_alias_redirect_target(page_base: &str, style: PageUrlStyle, origin: &str) -> String {
    let site_local_target = match style {
        PageUrlStyle::Ignore => directory_url(page_base),
        PageUrlStyle::TrailingSlash | PageUrlStyle::NoTrailingSlash => {
            site_local_canonical_page_url(page_base, style)
        }
    };

    prefix_origin(origin, &percent_encode_url_path(&site_local_target))
}

fn directory_url(page_base: &str) -> String {
    if page_base == "/" {
        return String::from("/");
    }

    format!("{page_base}/")
}

fn with_query_string(base_location: String, query: Option<&str>) -> String {
    let Some(query) = query.filter(|query| !query.is_empty()) else {
        return base_location;
    };
    format!("{base_location}?{query}")
}

fn resolve_relative_path(
    relative_path: &Path,
    output_dir: &Path,
) -> Result<PathBuf, ResolvedRequest> {
    let mut sanitized = PathBuf::new();
    // Reject any non-normal components so HTTP paths cannot escape the dev output root.
    for component in relative_path.components() {
        match component {
            Component::Normal(part) => sanitized.push(part),
            _ => return Err(ResolvedRequest::InvalidPath),
        }
    }

    if sanitized.as_os_str().is_empty() {
        return Err(ResolvedRequest::InvalidPath);
    }

    let full_path = output_dir.join(&sanitized);
    if !full_path.starts_with(output_dir) {
        return Err(ResolvedRequest::InvalidPath);
    }

    Ok(full_path)
}

#[cfg(test)]
#[path = "tests/static_files_tests.rs"]
mod tests;
