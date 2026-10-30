//! Output path and relative URL computation for HTML JS glue and runtime modules.
//!
//! WHAT: deterministic glue module paths, runtime module paths, safe file names, and relative
//!       URL paths for ES module imports and HTML asset references.
//! WHY: the browser resolves ES module imports relative to the importing module's URL, so
//!      glue-to-asset and HTML-to-glue paths must be computed carefully.

use crate::build_system::utils::stable_content_hash;
use std::path::{Path, PathBuf};

/// Shared directory for every generated glue module. Glue source imports are written relative to
/// it, so the source never depends on its own content-derived file name.
pub(super) const GLUE_OUTPUT_DIR: &str = "_moth/js/glue";

/// Content-addressed output path for a generated glue ES module.
///
/// WHY: pages selecting the same wrappers generate byte-identical glue, so naming the file by its
///      source lets the builder emit it once and lets browsers cache it once.
pub(super) fn glue_module_output_path(glue_source: &str) -> PathBuf {
    let digest = stable_content_hash(glue_source.as_bytes());
    Path::new(GLUE_OUTPUT_DIR).join(format!("module-{digest:016x}.js"))
}

/// Whether `path` is a generated glue module path, the only builder output pages may share.
pub(crate) fn is_glue_module_output_path(path: &Path) -> bool {
    path.parent() == Some(Path::new(GLUE_OUTPUT_DIR))
}

/// Deterministic output paths for every required core runtime module.
///
/// This path-only preflight is intentionally separate from runtime source generation so builders
/// can reject resource collisions before any provider-backed asset is read.
pub(crate) fn planned_runtime_module_output_paths(
    specifiers: &std::collections::BTreeSet<String>,
) -> Vec<PathBuf> {
    specifiers
        .iter()
        .map(|specifier| runtime_module_output_path(specifier))
        .collect()
}

/// Deterministic output path for a core runtime module.
pub(super) fn runtime_module_output_path(specifier: &str) -> PathBuf {
    let safe_name = runtime_module_safe_name(specifier);
    PathBuf::from("_moth/js/runtime").join(format!("{safe_name}.js"))
}

/// Sanitize a runtime module specifier into a safe file name.
pub(super) fn runtime_module_safe_name(specifier: &str) -> String {
    let trimmed = specifier.trim_start_matches('@');
    let mut safe_name = String::new();

    for ch in trimmed.chars() {
        if ch.is_ascii_alphanumeric() {
            safe_name.push(ch.to_ascii_lowercase());
        } else if !safe_name.ends_with('-') {
            safe_name.push('-');
        }
    }

    let safe_name = safe_name.trim_matches('-');
    if safe_name.is_empty() {
        "runtime-module".to_owned()
    } else {
        safe_name.to_owned()
    }
}

/// Compute a relative URL path from an emitted file's directory to another emitted asset.
///
/// WHAT: returns a relative path string using `/` separators suitable for use in an HTML
///       `src`, `href`, or module import specifier.
/// WHY: both the HTML and assets are emitted with relative paths from the project root;
///      browsers resolve relative URLs against the directory of the file that contains the
///      reference.
pub(super) fn relative_url_path(from_directory: &Path, to_asset: &Path) -> String {
    let from_components: Vec<_> = from_directory.components().collect();
    let to_components: Vec<_> = to_asset.components().collect();

    let mut common = 0;
    let from_dir_len = from_components.len();
    while common < from_dir_len
        && common < to_components.len()
        && from_components[common] == to_components[common]
    {
        common += 1;
    }

    let mut path = String::new();

    // Go up one level for each remaining directory below the shared prefix.
    for _ in common..from_dir_len {
        path.push_str("../");
    }

    for component in to_components.iter().skip(common) {
        if !path.is_empty() && !path.ends_with('/') {
            path.push('/');
        }
        path.push_str(&component.as_os_str().to_string_lossy());
    }

    if path.is_empty() {
        path.push_str(
            &to_asset
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
        );
    }

    if !path.starts_with("./") && !path.starts_with("../") && !path.starts_with('/') {
        path.insert_str(0, "./");
    }

    path
}
