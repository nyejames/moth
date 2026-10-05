//! Static file and HTML injection helpers for the dev server.
//!
//! This module resolves safe output-relative paths, maps content types and installs the
//! development client before application bootstrap scripts can call its outcome hooks.

use crate::projects::dev_server::dev_client::{DEV_CLIENT_MARKER, dev_client_snippet};
use crate::projects::routing::{HtmlSiteConfig, PageUrlStyle, prefix_origin};
use std::path::{Component, Path, PathBuf};

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

/// Resolve a request using the effective HTML routing policy from the latest successful build.
///
/// WHAT: applies strict path sanitization, exact-file asset lookup, and page canonicalization.
/// WHY: page routes and assets must remain separate so canonical redirects never affect assets.
pub fn resolve_request(
    request_path: &str,
    request_query: Option<&str>,
    output_dir: &Path,
    entry_page_rel: Option<&Path>,
    config: HtmlSiteConfig,
) -> ResolvedRequest {
    if request_path == "/" {
        let Some(entry_page) = entry_page_rel else {
            return ResolvedRequest::MissingEntryPage;
        };
        return match resolve_relative_path(entry_page, output_dir) {
            Ok(path) if path.exists() && path.is_file() => ResolvedRequest::File {
                path,
                kind: ResolvedRequestKind::PageHtml,
            },
            Ok(_) => ResolvedRequest::NotFound,
            Err(_) => ResolvedRequest::InvalidPath,
        };
    }

    if !request_path.starts_with('/') {
        return ResolvedRequest::InvalidPath;
    }

    // Step 1: explicit `/index.html` forms are page-route aliases and are resolved before
    // generic exact-file lookup so canonical page redirects stay deterministic.
    if let Some(page_base) = page_base_from_index_alias(request_path) {
        let page_file_rel = page_file_relative_path(&page_base);
        let page_file_path = match resolve_relative_path(&page_file_rel, output_dir) {
            Ok(path) => path,
            Err(_) => return ResolvedRequest::InvalidPath,
        };

        if !page_file_path.exists() || !page_file_path.is_file() {
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

        if exact_file_path.exists() && exact_file_path.is_file() {
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

    if !page_file_path.exists() || !page_file_path.is_file() {
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
        Some(prefix_origin(origin, &site_local_canonical))
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

    prefix_origin(origin, &site_local_target)
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
