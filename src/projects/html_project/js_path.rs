//! HTML builder JavaScript-only rendering path.
//!
//! WHAT: owns HIR -> JS lowering and inline HTML assembly for the JS-only build path.
//! WHY: keeping this path isolated lets the HTML builder add a Wasm mode without
//! blending two output strategies into one large module.
//!
//! JS-only HTML lifecycle contract (in emission order):
//!   1. Static entry fragments are emitted as raw HTML in source order.
//!   2. Runtime fragment slots are emitted as `<div id="moth-slot-N">` placeholders.
//!   3. The compiled JS bundle is embedded in an inline `<script>` block.
//!      The bundle content is escaped so it cannot contain a raw `</script>` sequence
//!      that would prematurely close the script tag.
//!   4. A second inline `<script>` calls entry `start()` once. start() returns the
//!      runtime fragment array and each element is hydrated into its slot in source order.

use crate::backends::js::{JsLoweringConfig, lower_hir_to_js};
use crate::build_system::build::{
    FileKind, Module, ModuleExternalImport, OutputFile, ProjectLinkedModule, ResolvedConstFragment,
};
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages};
use crate::compiler_frontend::hir::ids::FunctionId;
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::reachability::ReachableReactiveSinkKind;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::projects::html_project::compile_input::HtmlModuleCompileInput;
use crate::projects::html_project::document_config::HtmlDocumentConfig;
use crate::projects::html_project::document_shell::render_html_document_shell;
use crate::projects::html_project::external_js::runtime_glue::generate_module_glue;
use crate::projects::html_project::output_plan::derive_logical_html_path;
use crate::projects::html_project::page_metadata::extract_html_page_metadata;
use crate::timing_guard;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Inputs for rendering a JS-backed HTML document.
///
/// WHAT: groups all data needed to produce the final HTML document from a lowered JS module.
/// WHY: `render_html_document` previously took 9 separate parameters; this struct keeps the
///      call sites readable and the parameter list stable as fields are added or renamed.
pub(crate) struct HtmlDocumentRenderInput<'a> {
    pub hir_module: &'a HirModule,
    pub const_fragments: &'a [ResolvedConstFragment],
    pub string_table: &'a mut StringTable,
    pub document_config: &'a HtmlDocumentConfig,
    pub logical_html_path: &'a Path,
    pub project_name: &'a str,
    pub js_bundle: &'a str,
    pub function_names: &'a HashMap<FunctionId, String>,
    pub entry_runtime_fragment_count: usize,
    /// Whether the emitted JS bundle contains reactive runtime fragments that need the DOM mount
    /// helper instead of plain-string slot insertion.
    pub uses_reactive_runtime_fragments: bool,
    /// Optional import-map HTML to inject into `<head>` before module scripts.
    pub import_map_html: Option<String>,
    /// Whether the runtime bundle must be emitted as an ES module script.
    pub use_module_script: bool,
}

/// Artifacts produced by the JS-only HTML compilation path.
pub(crate) struct CompiledHtmlJsModule {
    pub output_files: Vec<OutputFile>,
    pub html_output_path: PathBuf,
}

/// Complete JS-only compilation input for one entry and its linked module selection.
pub(crate) struct HtmlJsCompileInput<'a> {
    pub(crate) module: &'a Module,
    pub(crate) external_imports: &'a [ModuleExternalImport],
    pub(crate) linked_modules: &'a [ProjectLinkedModule<'a>],
    pub(crate) source_function_names: Arc<
        std::collections::HashMap<
            crate::compiler_frontend::semantic_identity::OriginFunctionId,
            String,
        >,
    >,
    pub(crate) module_private_function_names: Arc<
        std::collections::HashMap<
            crate::compiler_frontend::semantic_identity::ModulePrivateExecutableIdentity,
            String,
        >,
    >,
    /// Generated symbol lookup for the entry module's project boundary.
    pub(crate) generated_function_names: Arc<
        std::collections::HashMap<
            crate::compiler_frontend::semantic_identity::GeneratedFunctionIdentity,
            String,
        >,
    >,
    /// Every generated symbol name assigned to this compilation, in deterministic order.
    pub(crate) all_generated_function_names: Arc<Vec<String>>,
    pub(crate) compile_input: &'a HtmlModuleCompileInput<'a>,
    pub(crate) output_path: PathBuf,
}

/// Compiles one module through the JS-only HTML builder path.
///
/// WHAT: lowers HIR to JS and embeds the JS with runtime slot hydration into HTML.
/// WHY: this preserves existing builder behavior when `--html-wasm` is not enabled.
pub(crate) fn compile_html_module_js(
    input: HtmlJsCompileInput<'_>,
    string_table: &mut StringTable,
) -> Result<CompiledHtmlJsModule, CompilerMessages> {
    let HtmlJsCompileInput {
        module,
        external_imports,
        linked_modules,
        source_function_names,
        module_private_function_names,
        generated_function_names,
        all_generated_function_names,
        compile_input: input,
        output_path,
    } = input;
    let js_lowering_config = JsLoweringConfig::html_page_bundle(
        input.build_profile.is_release(),
        Arc::clone(&input.external_package_registry),
        input.reachability.backend_selection().clone(),
        Arc::clone(&source_function_names),
        Arc::clone(&module_private_function_names),
        Arc::clone(&generated_function_names),
    );

    let mut js_module = {
        timing_guard!("backend.js.lower_hir");
        lower_hir_to_js(
            input.hir_module,
            input.borrow_analysis,
            string_table,
            js_lowering_config,
            input.type_environment,
        )
        .map_err(|error| CompilerMessages::from_error(error, string_table.clone()))?
    };

    if !linked_modules.is_empty() {
        let mut isolated_modules = Vec::with_capacity(linked_modules.len() + 1);
        for linked in linked_modules {
            let linked_config = JsLoweringConfig::html_page_bundle(
                input.build_profile.is_release(),
                Arc::clone(&linked.module.link_facts.external_package_registry),
                linked.reachability.backend_selection().clone(),
                Arc::clone(&source_function_names),
                Arc::clone(&module_private_function_names),
                Arc::clone(&linked.generated_function_names),
            );
            let linked_js = {
                timing_guard!("backend.js.lower_linked_hir");
                lower_hir_to_js(
                    &linked.module.executable.hir,
                    &linked.module.executable.borrow_analysis,
                    string_table,
                    linked_config,
                    &linked.module.executable.type_environment,
                )
                .map_err(|error| CompilerMessages::from_error(error, string_table.clone()))?
            };
            let exported_names = linked
                .module
                .executable
                .hir
                .function_ids_by_origin
                .values()
                .chain(
                    linked
                        .module
                        .executable
                        .hir
                        .function_ids_by_private_origin
                        .values(),
                )
                .chain(
                    linked
                        .module
                        .executable
                        .hir
                        .function_ids_by_generated
                        .values(),
                )
                .filter_map(|function_id| linked_js.function_name_by_id.get(function_id).cloned())
                .collect::<Vec<_>>();
            isolated_modules.push((linked_js.source, exported_names));
            js_module
                .referenced_external_functions
                .extend(linked_js.referenced_external_functions);
        }
        let mut entry_exported_names = input
            .hir_module
            .function_ids_by_origin
            .values()
            .chain(input.hir_module.function_ids_by_private_origin.values())
            .chain(input.hir_module.function_ids_by_generated.values())
            .filter_map(|function_id| js_module.function_name_by_id.get(function_id).cloned())
            .collect::<Vec<_>>();
        if let Some(start_name) = input
            .hir_module
            .start_function
            .and_then(|function_id| js_module.function_name_by_id.get(&function_id).cloned())
        {
            entry_exported_names.push(start_name);
        }
        isolated_modules.push((std::mem::take(&mut js_module.source), entry_exported_names));
        js_module.source = assemble_isolated_module_sources(
            isolated_modules,
            source_function_names
                .values()
                .chain(module_private_function_names.values())
                .chain(all_generated_function_names.iter())
                .cloned(),
        );
    }

    let uses_reactive_runtime_fragments =
        html_module_uses_reactive_runtime_fragments(input.hir_module, input.reachability);

    // Generate glue modules and import preamble only for external module exports referenced by
    // emitted JS. In HTML page bundles, JS lowering has already filtered unreachable wrappers.
    let glue_result = {
        timing_guard!("backend.js.generate_module_glue");
        generate_module_glue(
            module,
            external_imports,
            &js_module.referenced_external_functions,
            input.external_package_registry.as_ref(),
            &output_path,
            input.build_profile.is_release(),
        )
        .map_err(|error| CompilerMessages::from_error(error, string_table.clone()))?
    };

    let use_module_script = glue_result.bundle_import_preamble.is_some();
    let bundle_with_imports = if let Some(ref preamble) = glue_result.bundle_import_preamble {
        format!("{preamble}{}", js_module.source)
    } else {
        js_module.source.clone()
    };

    let html = {
        timing_guard!("backend.js.render_html_document");
        render_html_document(&mut HtmlDocumentRenderInput {
            hir_module: input.hir_module,
            const_fragments: input.const_fragments,
            string_table,
            document_config: input.document_config,
            logical_html_path: &output_path,
            project_name: input.project_name,
            js_bundle: &bundle_with_imports,
            function_names: &js_module.function_name_by_id,
            entry_runtime_fragment_count: input.root_activity.runtime_fragment_count,
            uses_reactive_runtime_fragments,
            import_map_html: glue_result.import_map_html,
            use_module_script,
        })?
    };

    let mut output_files = Vec::with_capacity(1 + glue_result.glue_output_files.len());
    output_files.push(OutputFile::new(output_path.clone(), FileKind::Html(html)));
    output_files.extend(glue_result.glue_output_files);

    Ok(CompiledHtmlJsModule {
        output_files,
        html_output_path: output_path,
    })
}

fn assemble_isolated_module_sources(
    modules: Vec<(String, Vec<String>)>,
    source_function_names: impl Iterator<Item = String>,
) -> String {
    let mut shared_names = source_function_names.collect::<Vec<_>>();
    shared_names.extend(
        modules
            .iter()
            .flat_map(|(_, exported_names)| exported_names.iter().cloned()),
    );
    shared_names.sort();
    shared_names.dedup();

    let mut source = String::new();
    if !shared_names.is_empty() {
        source.push_str("let ");
        source.push_str(&shared_names.join(", "));
        source.push_str(";\n");
    }

    for (module_source, mut exported_names) in modules {
        exported_names.sort();
        exported_names.dedup();
        if exported_names.is_empty() {
            source.push_str("(() => {\n");
            source.push_str(&module_source);
            source.push_str("\n})();\n");
            continue;
        }

        source.push_str("({ ");
        source.push_str(&exported_names.join(", "));
        source.push_str(" } = (() => {\n");
        source.push_str(&module_source);
        source.push_str("\nreturn { ");
        source.push_str(&exported_names.join(", "));
        source.push_str(" };\n})());\n");
    }

    source
}

/// Returns true when the JS bootstrap must route runtime fragments through the reactive mount
/// helper.
///
/// WHAT: HIR can carry placeholder template metadata for ordinary `String` parameters so helper
/// functions can preserve reactive template objects when callers provide them. Those placeholders
/// should not by themselves make a non-reactive page reference the mount helper. A runtime fragment
/// needs mounting only when it has a direct source dependency, or when a placeholder dependency can
/// be satisfied by a concrete reachable reactive source in this emitted page.
/// WHY: this mirrors JS helper gating and keeps ordinary pages on the plain insertion path.
fn html_module_uses_reactive_runtime_fragments(
    hir_module: &HirModule,
    reachability: &crate::compiler_frontend::hir::reachability::HirReachability,
) -> bool {
    let reachable_reactive_sources = hir_module
        .blocks
        .iter()
        .filter(|block| {
            reachability
                .backend_selection()
                .blocks()
                .contains(&block.id)
        })
        .flat_map(|block| block.locals.iter())
        .any(|local| {
            hir_module
                .side_table
                .reactive_source_id_for_local(local.id)
                .is_some()
        });

    reachability.reachable_reactive_sinks.iter().any(|sink| {
        if !matches!(sink.kind, ReachableReactiveSinkKind::RuntimeFragment) {
            return false;
        }

        let Some(template) = hir_module
            .side_table
            .reactive_templates()
            .find(|template| template.id == sink.template_id)
        else {
            return false;
        };

        !template.dependencies.is_empty()
            || (reachable_reactive_sources && !template.template_value_parameters.is_empty())
    })
}

/// Renders entry-file start fragments into static HTML and an ordered list of slot IDs.
///
/// WHAT: merges const fragments (with runtime insertion indices) and runtime slot placeholders
/// into source-order HTML. Returns slot IDs so the bootstrap script can hydrate them in order.
/// WHY: source order requires interleaving const strings at their indexed positions
///      relative to runtime slots. Slot count is supplied by the caller from HIR.
pub(crate) fn render_entry_fragments(
    const_fragments: &[ResolvedConstFragment],
    slot_count: usize,
) -> (String, Vec<String>) {
    let mut html = String::new();
    let mut slot_ids: Vec<String> = Vec::new();
    let mut runtime_index = 0usize;

    // Sort const fragments by runtime_insertion_index to handle them in order.
    let mut sorted_const: Vec<(usize, &str)> = const_fragments
        .iter()
        .map(|f| (f.runtime_insertion_index, f.rendered_text.as_str()))
        .collect();
    sorted_const.sort_by_key(|(idx, _)| *idx);

    let mut const_iter = sorted_const.iter().peekable();

    // Emit const fragments with insertion_index == 0 (before any runtime slots).
    while let Some((_, html_str)) = const_iter.next_if(|(idx, _)| *idx == runtime_index) {
        html.push_str(html_str);
        html.push('\n');
    }

    // Interleave runtime slots and const fragments.
    for _ in 0..slot_count {
        let slot_id = format!("moth-slot-{runtime_index}");
        html.push_str(&format!("<div id=\"{slot_id}\"></div>\n"));
        slot_ids.push(slot_id);
        runtime_index += 1;

        // Emit any const fragments whose insertion_index matches this runtime slot position.
        while let Some((_, html_str)) = const_iter.next_if(|(idx, _)| *idx == runtime_index) {
            html.push_str(html_str);
            html.push('\n');
        }
    }

    // Emit any remaining const fragments after all runtime slots.
    for (_, html_str) in const_iter {
        html.push_str(html_str);
        html.push('\n');
    }

    (html, slot_ids)
}

pub(crate) fn render_html_document(
    input: &mut HtmlDocumentRenderInput<'_>,
) -> Result<String, CompilerMessages> {
    let (body_html, slot_ids) =
        render_entry_fragments(input.const_fragments, input.entry_runtime_fragment_count);
    let start_function = input
        .hir_module
        .require_start_function("HTML document rendering")
        .map_err(|error| CompilerMessages::from_error(error, input.string_table.clone()))?;
    let page_metadata =
        extract_html_page_metadata(input.hir_module, start_function, input.string_table).map_err(
            |diagnostic| CompilerMessages::from_diagnostic_ref(*diagnostic, input.string_table),
        )?;
    let Some(start_function_name) = input.function_names.get(&start_function) else {
        return Err(CompilerMessages::from_error(
            CompilerError::compiler_error(format!(
                "HTML builder could not resolve start function {:?}",
                start_function
            )),
            input.string_table.clone(),
        ));
    };

    let script_html = render_runtime_bootstrap_script_html(
        start_function_name,
        input.js_bundle,
        &slot_ids,
        input.use_module_script,
        input.uses_reactive_runtime_fragments,
    );

    render_html_document_shell(
        input.document_config,
        &page_metadata,
        input.logical_html_path,
        input.project_name,
        body_html,
        script_html,
        input.import_map_html.clone(),
    )
    .map_err(|error| CompilerMessages::from_error(error, input.string_table.clone()))
}

fn render_runtime_bootstrap_script_html(
    start_function_name: &str,
    js_bundle: &str,
    slot_ids: &[String],
    is_module_script: bool,
    uses_reactive_runtime_fragments: bool,
) -> String {
    // Escape the bundle so any `</script>` sequence inside string literals or comments cannot
    // prematurely terminate the HTML script tag and corrupt the page.
    let safe_bundle = escape_inline_script(js_bundle);

    if is_module_script {
        // Emit a single module script so ES module imports are valid and share scope with
        // the bootstrap code.
        let mut html = String::new();
        html.push_str("<script type=\"module\">\n");
        html.push_str(&safe_bundle);
        html.push('\n');
        append_runtime_bootstrap(
            &mut html,
            start_function_name,
            slot_ids,
            "",
            uses_reactive_runtime_fragments,
        );
        html.push_str("</script>\n");
        html
    } else {
        // Classic script path: emit bundle and bootstrap in separate inline scripts.
        let mut html = String::new();
        html.push_str("<script>\n");
        html.push_str(&safe_bundle);
        html.push_str("\n</script>\n");
        html.push_str("<script>\n");
        html.push_str("(function () {\n");
        append_runtime_bootstrap(
            &mut html,
            start_function_name,
            slot_ids,
            "  ",
            uses_reactive_runtime_fragments,
        );
        html.push_str("})();\n");
        html.push_str("</script>\n");
        html
    }
}

fn append_runtime_bootstrap(
    html: &mut String,
    start_function_name: &str,
    slot_ids: &[String],
    indent: &str,
    uses_reactive_runtime_fragments: bool,
) {
    if slot_ids.is_empty() {
        html.push_str(&format!(
            "{indent}if (typeof {start_function_name} === \"function\") {start_function_name}();\n"
        ));
        return;
    }

    // WHAT: call entry start() once; it returns the runtime fragment array in source order.
    // WHY: start() accumulates fragments via PushRuntimeFragment and returns them as a JS array.
    //      Calling start() here both produces the fragments and runs the lifecycle.
    html.push_str(&format!(
        "{indent}var moth_frags = {start_function_name}();\n"
    ));
    html.push_str(&format!("{indent}var moth_slots = [\n"));
    for slot_id in slot_ids {
        html.push_str(&format!("{indent}  \"{slot_id}\",\n"));
    }
    html.push_str(&format!("{indent}];\n"));
    html.push_str(&format!(
        "{indent}for (var i = 0; i < moth_slots.length; i++) {{\n"
    ));
    html.push_str(&format!(
        "{indent}  var el = document.getElementById(moth_slots[i]);\n"
    ));
    html.push_str(&format!(
        "{indent}  if (!el) throw new Error(\"Missing runtime mount slot: \" + moth_slots[i]);\n"
    ));

    if uses_reactive_runtime_fragments {
        // Reactive pages use the backend mount helper so template fragments can register for
        // rerendering. The helper also handles plain-string fragments, preserving source order.
        html.push_str(&format!(
            "{indent}  __moth_mount_template_fragment(el, moth_frags[i]);\n"
        ));
    } else {
        // Non-reactive pages keep the plain direct insertion path and avoid referencing the
        // optional mount helper global.
        html.push_str(&format!(
            "{indent}  el.insertAdjacentHTML(\"beforeend\", moth_frags[i] || \"\");\n"
        ));
    }

    html.push_str(&format!("{indent}}}\n"));
}

/// Derive the logical HTML output path for this entry file.
///
/// Delegates to the canonical output planner so JS-only and Wasm paths agree on route derivation.
pub(crate) fn html_output_path(
    entry_point: &Path,
    entry_root: Option<&Path>,
    string_table: &mut StringTable,
) -> Result<PathBuf, CompilerError> {
    derive_logical_html_path(entry_point, entry_root, string_table)
}

/// Escapes JS source so it is safe to embed inside an HTML `<script>` block.
///
/// WHAT: replaces every `</` occurrence with `<\/` so the HTML parser cannot see a closing tag
/// sequence inside the script content.
/// WHY: a raw `</script>` anywhere in an inlined JS bundle — including inside string literals or
/// comments — causes the browser to terminate the script tag early and corrupt the page.
/// `<\/` is a valid JS string escape sequence equivalent to `</`, so the JS semantics are
/// preserved while the HTML parser sees a harmless non-tag sequence.
pub(crate) fn escape_inline_script(js: &str) -> String {
    js.replace("</", "<\\/")
}

#[cfg(test)]
#[path = "tests/js_path_tests.rs"]
mod tests;
