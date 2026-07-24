//! Import-map HTML generation for HTML JS runtime modules.
//!
//! WHAT: produces a `<script type="importmap">` that maps bare runtime module specifiers to
//!       their emitted relative paths.
//! WHY: provider-created JS assets and emitted runtime modules use bare imports like
//!      `@moth/runtime`; the import map lets the browser resolve them.

use crate::build_system::build::Module;
use crate::projects::html_project::external_js::runtime_glue::paths::{
    relative_url_path, runtime_module_output_path,
};
use std::path::Path;

/// Build import-map HTML for bare runtime specifiers.
///
/// WHAT: produces a `<script type="importmap">` that maps registered core module specifiers
///       to their emitted relative paths.
/// WHY: provider-created JS assets use bare imports like
///      `import {{ mothOk }} from "@moth/runtime";`;
///      the import map lets the browser resolve those without rewriting user files.
pub(super) fn build_import_map_html(module: &Module, html_output_path: &Path) -> Option<String> {
    let mut entries: Vec<(String, String)> = Vec::new();

    for external_import in &module.link_facts.module_external_imports {
        for runtime_import in &external_import.required_runtime_imports {
            let runtime_path = runtime_module_output_path(&runtime_import.module_name);
            let relative = relative_url_path(html_output_path, &runtime_path);
            entries.push((runtime_import.module_name.clone(), relative));
        }
    }

    if entries.is_empty() {
        return None;
    }

    // Deduplicate by specifier.
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    entries.dedup_by(|a, b| a.0 == b.0);

    let mut map_json = String::from("{\n  \"imports\": {\n");
    for (index, (specifier, path)) in entries.iter().enumerate() {
        if index > 0 {
            map_json.push_str(",\n");
        }
        map_json.push_str(&format!("    \"{specifier}\": \"{path}\""));
    }
    map_json.push_str("\n  }\n}");

    Some(format!(
        "<script type=\"importmap\">\n{map_json}\n</script>\n"
    ))
}
