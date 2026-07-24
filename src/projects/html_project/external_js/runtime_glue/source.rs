//! Glue module source generation for HTML JS external exports.
//!
//! WHAT: generates ES module source that imports raw JS exports and re-exports stable wrapper
//!       functions, including fallible result-shape validation.
//! WHY: the JS backend calls wrappers by stable names; wrappers adapt raw JS return shapes
//!      to Moth's internal conventions.

use crate::backends::js::{
    builtin_error_code_js_field_name, builtin_error_message_js_field_name,
    external_module_export_glue_function_name,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::external_packages::ExternalPackageId;
use crate::projects::html_project::external_js::runtime_glue::exports::ReferencedExport;
use std::collections::HashMap;

/// Generate the glue module ES module source.
pub(super) fn generate_glue_module_source(
    exports: &[ReferencedExport],
    package_asset_paths: &HashMap<ExternalPackageId, String>,
    release_build: bool,
) -> Result<String, CompilerError> {
    let mut source = String::new();

    // Group imports by asset path so we emit one import statement per asset.
    let mut imports_by_path: HashMap<String, Vec<(String, String)>> = HashMap::new();
    for export in exports {
        let path = package_asset_paths.get(&export.package_id).ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "HTML JS glue could not find a runtime asset for external package {:?}.",
                export.package_id
            ))
        })?;
        imports_by_path
            .entry(path.clone())
            .or_default()
            .push((export.export_name.clone(), export.raw_import_name.clone()));
    }

    // Emit import statements.
    let mut sorted_paths: Vec<_> = imports_by_path.keys().cloned().collect();
    sorted_paths.sort();
    for path in sorted_paths {
        let mut names = imports_by_path.get(&path).cloned().unwrap_or_default();
        names.sort();
        names.dedup();
        let import_names = names
            .iter()
            .map(|(export_name, local_name)| format!("{export_name} as {local_name}"))
            .collect::<Vec<_>>();
        source.push_str(&format!(
            "import {{ {} }} from \"{}\";\n",
            import_names.join(", "),
            path
        ));
    }

    // Emit wrapper functions.
    for export in exports {
        let wrapper_name = external_module_export_glue_function_name(export.function_id);
        source.push('\n');

        if export.is_fallible {
            source.push_str(&generate_fallible_wrapper(
                &wrapper_name,
                &export.raw_import_name,
                release_build,
            ));
        } else {
            source.push_str(&generate_infallible_wrapper(
                &wrapper_name,
                &export.raw_import_name,
            ));
        }
    }

    Ok(source)
}

/// Generates a non-fallible wrapper that forwards all arguments and returns the raw result.
pub(super) fn generate_infallible_wrapper(wrapper_name: &str, export_name: &str) -> String {
    format!(
        "export function {wrapper_name}(...args) {{
    return {export_name}(...args);
}}
"
    )
}

/// Generates a fallible wrapper that validates the external result shape and converts it to
/// Moth's internal fallible carrier.
///
/// WHAT: calls the raw JS export, expects `{ ok: boolean, value? }` or `{ ok: false, error }`,
///       and returns `{ tag: "ok", value: ... }` or an internal Moth `Error` struct value.
/// WHY: the JS backend HIR lowering assumes all fallible calls return this carrier shape.
pub(super) fn generate_fallible_wrapper(
    wrapper_name: &str,
    export_name: &str,
    release_build: bool,
) -> String {
    let invalid_error = internal_error_object_source(
        "\"Invalid result wrapper from external JavaScript function\"",
        "0",
        release_build,
    );
    let catch_error = internal_error_object_source("String(e.message || e)", "0", release_build);
    let returned_error = internal_error_object_source(
        "error.message || \"Unknown error\"",
        "typeof error.code === \"number\" ? error.code : 0",
        release_build,
    );

    let invalid_wrapper_handling = if release_build {
        format!("        return {{ tag: \"err\", value: {invalid_error} }};")
    } else {
        format!(
            "        throw new Error(
            \"Invalid result wrapper from external function '{wrapper_name}': \" +
            \"expected {{ ok: boolean, value? }} or {{ ok: false, error: {{ code, message }} }}\"
        );"
        )
    };

    format!(
        "export function {wrapper_name}(...args) {{
    let result;
    try {{
        result = {export_name}(...args);
    }} catch (e) {{
        return {{ tag: \"err\", value: {catch_error} }};
    }}

    if (result && typeof result.ok === \"boolean\") {{
        if (result.ok === true) {{
            return {{ tag: \"ok\", value: result.value }};
        }}
        if (result.ok === false) {{
            const error = result.error || {{ message: \"Unknown error\", code: 0 }};
            return {{ tag: \"err\", value: {returned_error} }};
        }}
    }}

{invalid_wrapper_handling}
}}
"
    )
}

fn internal_error_object_source(
    message_expression: &str,
    code_expression: &str,
    release_build: bool,
) -> String {
    let message_field = builtin_error_message_js_field_name(release_build);
    let code_field = builtin_error_code_js_field_name(release_build);

    format!("{{ {message_field}: {message_expression}, {code_field}: {code_expression} }}")
}
