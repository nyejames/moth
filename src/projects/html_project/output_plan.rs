//! Canonical HTML route and artifact output planning for the HTML builder.
//!
//! WHAT: derives filesystem artifact locations from entry-file paths and route conventions.
//! WHY: both the JS-only and HTML+Wasm builder paths need to agree on where outputs land.
//!      Centralising this here means there is one place to change layout conventions later.
//!
//! This module owns path derivation only. Artifact emission (lowering JS, Wasm, generating HTML)
//! lives in the respective `js_path` and `wasm/artifacts` modules.

use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use std::path::{Path, PathBuf};

/// A resolved output plan for one HTML route.
///
/// `html_path` is the physical HTML file location on disk. For JS-only mode this equals
/// `logical_html_path`; for Wasm mode both can differ only when legacy non-folder paths
/// are normalised into `<route>/index.html` form.
///
/// `js_path` and `wasm_path` are `None` for JS-only builds and `Some` for Wasm builds.
#[allow(clippy::struct_field_names)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HtmlRouteOutputPlan {
    /// Logical route path derived from the entry file (e.g. `about/index.html`).
    pub logical_html_path: PathBuf,
    /// Physical HTML file destination (may differ from `logical_html_path` in Wasm mode).
    pub html_path: PathBuf,
    /// Bootstrap JS path colocated with the HTML file (Wasm mode only).
    pub js_path: Option<PathBuf>,
    /// Wasm binary path colocated with the HTML file (Wasm mode only).
    pub wasm_path: Option<PathBuf>,
}

/// Build an output plan for one route in HTML+Wasm mode from an already-derived logical HTML path.
///
/// WHAT: colocates JS bootstrap and Wasm binary alongside `index.html` under the route folder.
/// WHY: the HTML project builder derives the canonical page route once via `derive_logical_html_path`.
///      This function must not re-derive routes — it only plans colocated artifact placement.
pub(crate) fn plan_wasm_output_from_logical_html_path(
    logical_html_path: &Path,
) -> Result<HtmlRouteOutputPlan, CompilerError> {
    let route_base = derive_wasm_route_base(logical_html_path)?;

    let (html_path, js_path, wasm_path) = if route_base.as_os_str().is_empty() {
        (
            PathBuf::from("index.html"),
            PathBuf::from("page.js"),
            PathBuf::from("page.wasm"),
        )
    } else {
        (
            route_base.join("index.html"),
            route_base.join("page.js"),
            route_base.join("page.wasm"),
        )
    };

    Ok(HtmlRouteOutputPlan {
        logical_html_path: logical_html_path.to_path_buf(),
        html_path,
        js_path: Some(js_path),
        wasm_path: Some(wasm_path),
    })
}

/// Derive the logical HTML output path from an entry file.
///
/// WHAT: maps Moth entry conventions to HTML paths:
/// - Directory builds use only the module root directory relative to `entry_root`, so a root
///   module emits `index.html` and a nested module emits `<directory>/index.html`.
/// - Single-file builds strip `#` prefix and use legacy `.html` extension.
pub(crate) fn derive_logical_html_path(
    entry_point: &Path,
    entry_root: Option<&Path>,
    string_table: &mut StringTable,
) -> Result<PathBuf, CompilerError> {
    if let Some(entry_root) = entry_root {
        return derive_logical_html_path_from_entry_root(entry_point, entry_root, string_table);
    }

    derive_single_file_logical_html_path(entry_point, string_table)
}

fn derive_logical_html_path_from_entry_root(
    entry_point: &Path,
    entry_root: &Path,
    string_table: &mut StringTable,
) -> Result<PathBuf, CompilerError> {
    // Route derivation is deterministic: discovery order never affects output paths.
    let relative_entry = entry_point.strip_prefix(entry_root).map_err(|_| {
        CompilerError::file_error(
            entry_point,
            format!(
                "HTML entry '{}' is not inside the configured entry root '{}'.",
                entry_point.display(),
                entry_root.display(),
            ),
            string_table,
        )
    })?;
    let parent = relative_entry.parent().unwrap_or_else(|| Path::new(""));

    // Directory routes describe module directories, not cosmetic hash-root filenames. The
    // active root module is the homepage and every nested module is folder-backed at its
    // entry-root-relative directory.
    if parent.as_os_str().is_empty() {
        return Ok(PathBuf::from("index.html"));
    }

    Ok(parent.join("index.html"))
}

/// Derive the logical HTML path for a single-file build.
///
/// WHAT: converts the entry stem to an exact UTF-8 route name, then maps `#page` to the
/// homepage and strips a cosmetic leading `#` from any other stem.
/// WHY: the stem is filesystem-authored, so an empty or non-UTF-8 stem is a File
///      infrastructure error. It must never collapse to a generic `main` fallback, which
///      would alias distinct source identities to one route.
fn derive_single_file_logical_html_path(
    entry_point: &Path,
    string_table: &mut StringTable,
) -> Result<PathBuf, CompilerError> {
    let raw_stem = entry_point.file_stem().ok_or_else(|| {
        CompilerError::file_error(
            entry_point,
            format!(
                "HTML single-file entry {entry_point:?} has no file stem; Moth routes need a non-empty UTF-8 stem."
            ),
            string_table,
        )
    })?;

    let file_stem = raw_stem.to_str().ok_or_else(|| {
        CompilerError::file_error(
            entry_point,
            "HTML single-file entry stem is not valid UTF-8; Moth routes require UTF-8 stems."
                .to_string(),
            string_table,
        )
    })?;

    if file_stem.is_empty() {
        return Err(CompilerError::file_error(
            entry_point,
            "HTML single-file entry has an empty stem; Moth routes require a non-empty UTF-8 stem.".to_string(),
            string_table,
        ));
    }

    if file_stem == "#page" {
        return Ok(PathBuf::from("index.html"));
    }

    let route_name = file_stem.strip_prefix('#').unwrap_or(file_stem);

    if route_name.is_empty() {
        return Err(CompilerError::file_error(
            entry_point,
            "HTML single-file entry stem is empty after stripping the cosmetic '#' prefix; Moth routes require a non-empty route name.".to_string(),
            string_table,
        ));
    }

    Ok(PathBuf::from(format!("{route_name}.html")))
}

/// Derive the route folder base from a logical HTML path for Wasm artifact co-location.
///
/// - `index.html` -> empty route base (root)
/// - `about/index.html` → `about`
/// - `about.html` (legacy) → `about`
fn derive_wasm_route_base(logical_html_path: &Path) -> Result<PathBuf, CompilerError> {
    if logical_html_path == Path::new("index.html") {
        return Ok(PathBuf::new());
    }

    if logical_html_path.file_name().and_then(|name| name.to_str()) == Some("index.html") {
        return Ok(logical_html_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default());
    }

    // Legacy flat path: normalise to route folder.
    if logical_html_path.extension().and_then(|ext| ext.to_str()) != Some("html") {
        return Err(CompilerError::compiler_error(format!(
            "HTML Wasm output conversion expected an '.html' path, got '{}'",
            logical_html_path.display()
        )));
    }
    Ok(logical_html_path.with_extension(""))
}

#[cfg(test)]
#[path = "tests/output_plan_tests.rs"]
mod tests;
