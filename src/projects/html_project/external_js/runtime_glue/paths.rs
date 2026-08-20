//! Output path and relative URL computation for HTML JS glue and runtime modules.
//!
//! WHAT: deterministic glue module paths, runtime module paths, safe file names, and relative
//!       URL paths for ES module imports and HTML asset references.
//! WHY: the browser resolves ES module imports relative to the importing module's URL, so
//!      glue-to-asset and HTML-to-glue paths must be computed carefully.

use crate::compiler_frontend::module_compilation::Module;
use crate::projects::html_project::external_js::path_identity::stable_path_hash_hex;
use std::path::{Path, PathBuf};

/// Deterministic output path for a module's glue ES module.
pub(super) fn glue_module_output_path(module: &Module) -> PathBuf {
    let entry_hash = stable_path_hash_hex(&module.metadata.entry_point);
    PathBuf::from("_moth/js/glue").join(format!("module-{entry_hash}.js"))
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

/// Compute a relative URL path from one emitted file to another emitted asset.
///
/// WHAT: returns a relative path string using `/` separators suitable for use in an HTML
///       `src`, `href`, or module import specifier.
/// WHY: both the HTML and assets are emitted with relative paths from the project root;
///      browsers resolve relative URLs against the file that contains the reference.
pub(super) fn relative_url_path(from_output_file: &Path, to_asset: &Path) -> String {
    let from_components: Vec<_> = from_output_file.components().collect();
    let to_components: Vec<_> = to_asset.components().collect();

    // Find common prefix length, excluding the HTML file name itself.
    let mut common = 0;
    let from_dir_len = from_components.len().saturating_sub(1);
    while common < from_dir_len
        && common < to_components.len()
        && from_components[common] == to_components[common]
    {
        common += 1;
    }

    let mut path = String::new();

    // Go up one level for each remaining directory in the HTML path.
    for _ in common..from_dir_len {
        path.push_str("../");
    }

    // Go down through the remaining asset components.
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
