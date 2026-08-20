//! Generated HTML JS glue for provider-created external module exports.
//!
//! WHAT: produces small ES modules that import from emitted JS runtime assets and re-export
//!       wrapper functions with stable names the JS backend can call.
//! WHY: keeps user-authored JS separate from generated glue, and gives the HTML builder control
//!      over module resolution, wrapper semantics, and dev/debug vs release validation.

mod exports;
mod import_map;
mod paths;
mod runtime_modules;
mod source;

pub(crate) use runtime_modules::emit_build_runtime_modules;

use crate::backends::js::external_module_export_glue_function_name;
use crate::build_system::build::{FileKind, OutputFile};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::external_packages::{
    ExternalFunctionId, ExternalPackageId, ExternalPackageRegistry,
};
use crate::compiler_frontend::module_compilation::{Module, ModuleExternalImport};
use crate::projects::html_project::external_js::runtime_assets::js_runtime_asset_output_path;
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// Result of generating glue for a single compiled module.
///
/// WHAT: carries any emitted glue files and the import preamble that must be prepended to the
///       JS bundle so it can call the generated wrapper functions.
/// WHY: the caller (JS-only HTML path) decides how to embed the bundle and imports together.
pub(crate) struct ModuleGlueResult {
    /// Additional JS output files to emit (glue modules).
    pub glue_output_files: Vec<OutputFile>,
    /// Optional `import { ... } from "...";` statement to prepend to the bundle.
    pub bundle_import_preamble: Option<String>,
    /// Optional import-map HTML to inject before module scripts.
    pub import_map_html: Option<String>,
}

/// Generates glue and import-map HTML for a single compiled module.
///
/// WHAT: inspects the set of external functions referenced by the JS backend and produces:
///       - a glue ES module if any `ExternalModuleExport` functions were referenced;
///       - the import statement to prepend to the JS bundle;
///       - optional import-map HTML for bare runtime specifiers.
/// WHY: caller (JS-only HTML path) decides whether to inline the bundle as a module script
///      or emit it separately.
pub(crate) fn generate_module_glue(
    module: &Module,
    external_imports: &[ModuleExternalImport],
    referenced_external_functions: &HashSet<ExternalFunctionId>,
    registry: &ExternalPackageRegistry,
    html_output_path: &Path,
    release_build: bool,
) -> Result<ModuleGlueResult, CompilerError> {
    let referenced_exports =
        exports::collect_referenced_exports(referenced_external_functions, registry)?;
    if referenced_exports.is_empty() {
        return Ok(ModuleGlueResult {
            glue_output_files: Vec::new(),
            bundle_import_preamble: None,
            import_map_html: None,
        });
    }

    // Build a map from package ID to its emitted asset path so glue can import it.
    // WHY: paths must be relative to the glue module, not the HTML document, because
    //      the browser resolves ES module imports relative to the importing module's URL.
    let package_asset_paths = build_package_asset_path_map(module, external_imports);

    // Generate the glue module source.
    let glue_source = source::generate_glue_module_source(
        &referenced_exports,
        &package_asset_paths,
        release_build,
    )?;

    let glue_output_path = paths::glue_module_output_path(module);

    let glue_output_file = OutputFile::new(glue_output_path.clone(), FileKind::Js(glue_source));

    let glue_import_path = paths::relative_url_path(html_output_path, &glue_output_path);
    let glue_names: Vec<String> = referenced_exports
        .iter()
        .map(|exp| external_module_export_glue_function_name(exp.function_id))
        .collect();

    let import_preamble = format!(
        "import {{ {} }} from \"{}\";\n",
        glue_names.join(", "),
        glue_import_path
    );

    let import_map_html = import_map::build_import_map_html(external_imports, html_output_path);

    Ok(ModuleGlueResult {
        glue_output_files: vec![glue_output_file],
        bundle_import_preamble: Some(import_preamble),
        import_map_html,
    })
}

/// Build a map from package ID to the relative URL path of its emitted JS asset.
///
/// WHAT: computes paths relative to the glue module so ES module imports resolve correctly
///       when the glue module imports from emitted JS runtime assets.
fn build_package_asset_path_map(
    module: &Module,
    external_imports: &[ModuleExternalImport],
) -> HashMap<ExternalPackageId, String> {
    let mut map = HashMap::new();
    let glue_output_path = paths::glue_module_output_path(module);

    for external_import in external_imports {
        let Some(asset) = &external_import.runtime_asset else {
            continue;
        };
        if asset.asset_kind != "js" {
            continue;
        }
        let output_path = js_runtime_asset_output_path(&asset.canonical_source_path);
        let relative = paths::relative_url_path(&glue_output_path, &output_path);
        map.insert(external_import.package_id, relative);
    }

    map
}

#[cfg(test)]
#[path = "../tests/runtime_glue_tests.rs"]
mod tests;
