//! Canonical HTML route and artifact output planning for the HTML builder.
//!
//! WHAT: derives filesystem artifact locations from entry-file paths and route conventions.
//! WHY: both the JS-only and HTML+Wasm builder paths need to agree on where outputs land.
//!      Centralising this here means there is one place to change layout conventions later.
//!
//! This module owns path derivation only. Artifact emission (lowering JS, Wasm, generating HTML)
//! lives in the respective `js_path` and `wasm/artifacts` modules.

use crate::compiler_frontend::compiler_errors::CompilerError;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// The canonical route projections shared by every HTML output consumer.
///
/// Route semantics are derived from the entry path exactly once. Consumers use the projection
/// they need: JS output uses `logical_html_path`, Wasm co-location uses `route_base`, and page
/// metadata uses `route_segment` for its display fallback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CanonicalPageRoute {
    /// Logical HTML path derived from the entry file.
    pub(crate) logical_html_path: PathBuf,
    /// Route folder used to colocate Wasm artifacts.
    pub(crate) route_base: PathBuf,
    /// Raw final route component used by page-title formatting. Keeping the OS string preserves
    /// the shell's existing non-UTF-8 infrastructure error boundary.
    pub(crate) route_segment: Option<OsString>,
}

/// A resolved output plan for one HTML route.
///
/// `html_path` is the physical HTML file location on disk. For JS-only mode this equals the
/// route's `logical_html_path`; for Wasm mode both can differ only when legacy non-folder paths
/// are normalised into `<route>/index.html` form.
///
/// `js_path` and `wasm_path` are `None` for JS-only builds and `Some` for Wasm builds.
#[allow(clippy::struct_field_names)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HtmlRouteOutputPlan {
    /// Physical HTML file destination (may differ from the route's logical path in Wasm mode).
    pub html_path: PathBuf,
    /// Bootstrap JS path colocated with the HTML file (Wasm mode only).
    pub js_path: Option<PathBuf>,
    /// Wasm binary path colocated with the HTML file (Wasm mode only).
    pub wasm_path: Option<PathBuf>,
}

/// Build an output plan for one route in HTML+Wasm mode.
///
/// WHAT: colocates JS bootstrap and Wasm binary alongside `index.html` under the route folder.
/// WHY: the HTML project builder derives the canonical page route once, then every downstream
///      output consumer uses this structured route without re-parsing a path string.
pub(crate) fn plan_wasm_output_from_route(route: &CanonicalPageRoute) -> HtmlRouteOutputPlan {
    let (html_path, js_path, wasm_path) = if route.route_base.as_os_str().is_empty() {
        (
            PathBuf::from("index.html"),
            PathBuf::from("page.js"),
            PathBuf::from("page.wasm"),
        )
    } else {
        (
            route.route_base.join("index.html"),
            route.route_base.join("page.js"),
            route.route_base.join("page.wasm"),
        )
    };

    HtmlRouteOutputPlan {
        html_path,
        js_path: Some(js_path),
        wasm_path: Some(wasm_path),
    }
}

/// Derive the canonical HTML page route from an entry file.
///
/// WHAT: maps Moth entry conventions to one route value:
///
/// - Directory builds use only the module root directory relative to `entry_root`, so a root
///   module emits `index.html` and a nested module emits `<directory>/index.html`.
/// - Single-file builds strip `@` prefix and use legacy `.html` extension.
///
/// WHY: all HTML output consumers must use the same route projections rather than re-deriving
/// route semantics from the logical path.
pub(crate) fn derive_logical_html_path(
    entry_point: &Path,
    entry_root: Option<&Path>,
) -> Result<CanonicalPageRoute, CompilerError> {
    if let Some(entry_root) = entry_root {
        return derive_logical_html_path_from_entry_root(entry_point, entry_root);
    }

    derive_single_file_logical_html_path(entry_point)
}

fn derive_logical_html_path_from_entry_root(
    entry_point: &Path,
    entry_root: &Path,
) -> Result<CanonicalPageRoute, CompilerError> {
    // Route derivation is deterministic: discovery order never affects output paths.
    let relative_entry = entry_point.strip_prefix(entry_root).map_err(|_| {
        CompilerError::file_error(
            entry_point,
            format!(
                "HTML entry '{}' is not inside the configured entry root '{}'.",
                entry_point.display(),
                entry_root.display(),
            ),
        )
    })?;
    let parent = relative_entry.parent().unwrap_or_else(|| Path::new(""));

    // Directory routes describe module directories, not cosmetic normal-root filenames. The
    // active root module is the homepage and every nested module is folder-backed at its
    // entry-root-relative directory.
    if parent.as_os_str().is_empty() {
        return Ok(CanonicalPageRoute {
            logical_html_path: PathBuf::from("index.html"),
            route_base: PathBuf::new(),
            route_segment: None,
        });
    }

    Ok(CanonicalPageRoute {
        logical_html_path: parent.join("index.html"),
        route_base: parent.to_path_buf(),
        route_segment: parent.file_name().map(OsString::from),
    })
}

/// Derive the logical HTML route for a single-file build.
///
/// WHAT: converts the entry stem to an exact UTF-8 route name, then maps `@page` to the
/// homepage and strips a cosmetic leading `@` from any other stem.
/// WHY: the stem is filesystem-authored, so an empty or non-UTF-8 stem is a File
///      infrastructure error. It must never collapse to a generic `main` fallback, which
///      would alias distinct source identities to one route.
fn derive_single_file_logical_html_path(
    entry_point: &Path,
) -> Result<CanonicalPageRoute, CompilerError> {
    let raw_stem = entry_point.file_stem().ok_or_else(|| {
        CompilerError::file_error(
            entry_point,
            format!(
                "HTML single-file entry {entry_point:?} has no file stem; Moth routes need a non-empty UTF-8 stem."
            ),
        )
    })?;

    let file_stem = raw_stem.to_str().ok_or_else(|| {
        CompilerError::file_error(
            entry_point,
            "HTML single-file entry stem is not valid UTF-8; Moth routes require UTF-8 stems."
                .to_string(),
        )
    })?;

    if file_stem.is_empty() {
        return Err(CompilerError::file_error(
            entry_point,
            "HTML single-file entry has an empty stem; Moth routes require a non-empty UTF-8 stem."
                .to_string(),
        ));
    }

    if file_stem == "@page" {
        return Ok(CanonicalPageRoute {
            logical_html_path: PathBuf::from("index.html"),
            route_base: PathBuf::new(),
            route_segment: None,
        });
    }

    let route_name = file_stem.strip_prefix('@').unwrap_or(file_stem);

    if route_name.is_empty() {
        return Err(CompilerError::file_error(
            entry_point,
            "HTML single-file entry stem is empty after stripping the cosmetic '@' prefix; Moth routes require a non-empty route name.".to_string(),
        ));
    }

    let logical_html_path = PathBuf::from(format!("{route_name}.html"));
    // Preserve the canonical homepage projection for legacy `index.moth` entries too.
    let is_homepage = route_name == "index";

    Ok(CanonicalPageRoute {
        logical_html_path,
        route_base: if is_homepage {
            PathBuf::new()
        } else {
            PathBuf::from(route_name)
        },
        route_segment: (!is_homepage).then(|| OsString::from(route_name)),
    })
}

#[cfg(test)]
#[path = "tests/output_plan_tests.rs"]
mod tests;
