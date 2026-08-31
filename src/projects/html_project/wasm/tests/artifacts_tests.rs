//! Tests for HTML+Wasm artifact planning and emission.

use super::*;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::projects::html_project::compile_input::HtmlModuleCompileInput;
use crate::projects::html_project::document_config::HtmlDocumentConfig;
use crate::projects::html_project::page_metadata::HtmlPageMetadataPlan;
use crate::projects::html_project::resource_output_plan::{
    HtmlResourceOutputPlan, ResourceUrlContext,
};
use crate::projects::html_project::structural_url_renderer::StructuralUrlRenderer;
use crate::projects::html_project::tests::test_support::{create_test_module, expect_js_output};
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn entry_reachability(
    module: &crate::compiler_frontend::module_compilation::Module,
) -> crate::compiler_frontend::hir::reachability::HirReachability {
    crate::compiler_frontend::hir::reachability::collect_reachability_from_function_link_facts(
        &module.link_facts.functions,
        &[module
            .executable
            .hir
            .start_function
            .expect("entry module should have start")],
    )
    .expect("test module should produce entry reachability")
}

#[test]
fn compile_html_module_wasm_exports_moth_start_directly() {
    // WHAT: verify that the export plan exports entry start() as "moth_start", not per-function
    //       wrappers discovered by entry-body call scanning.
    // WHY: entry start() is the sole runtime fragment producer; JS calls it directly.
    let mut string_table = StringTable::new();
    let module = create_test_module(PathBuf::from("@page.moth"), &mut string_table);
    let reachability = entry_reachability(&module);
    let page_metadata_plan = HtmlPageMetadataPlan::default();

    let compile_input = HtmlModuleCompileInput {
        hir_module: &module.executable.hir,
        resource_table: &module.executable.resource_table,
        reachability: &reachability,
        type_environment: &module.executable.type_environment,
        const_fragments: &[],
        page_metadata_plan: &page_metadata_plan,
        borrow_analysis: &module.executable.borrow_analysis,
        project_name: "",
        document_config: &HtmlDocumentConfig::default(),
        build_profile: crate::build_system::BuildProfile::Dev,
        root_activity: &module.metadata.root_activity,
        external_package_registry: Arc::new(
            crate::compiler_frontend::external_packages::ExternalPackageRegistry::new(),
        ),
    };
    let output_plan = HtmlResourceOutputPlan::new("");
    let resource_url_context = ResourceUrlContext::PageDocument(PathBuf::from("index.html"));
    let structural_url_renderer =
        StructuralUrlRenderer::new(&output_plan, &resource_url_context, Some("/"));
    let compiled = compile_html_module_wasm(
        &compile_input,
        &mut string_table,
        Path::new("index.html"),
        &structural_url_renderer,
    )
    .expect("wasm mode compilation should succeed");
    let js = expect_js_output(&compiled.output_files, "page.js");

    assert!(
        js.contains("instance.exports.moth_start()"),
        "bootstrap must call moth_start() directly, got:\n{js}"
    );
    assert!(
        !js.contains("moth_call_0"),
        "per-function wrapper exports must not appear in the new architecture"
    );
    assert!(
        !js.contains("__moth_install_wasm_wrappers"),
        "wrapper installation must not appear in the new architecture"
    );
}

#[test]
fn wasm_export_plan_contains_single_entry_start_export() {
    // WHAT: export plan must contain exactly one function export: moth_start for the start function.
    let mut string_table = StringTable::new();
    let module = create_test_module(PathBuf::from("@page.moth"), &mut string_table);
    let reachability = entry_reachability(&module);

    let plan_a = build_html_wasm_plan(&module.executable.hir, &reachability, Vec::new())
        .expect("wasm plan should build");
    let plan_b = build_html_wasm_plan(&module.executable.hir, &reachability, Vec::new())
        .expect("wasm plan should build");

    assert_eq!(
        plan_a.export_plan.function_exports.len(),
        1,
        "export plan must have exactly one function export"
    );
    assert_eq!(
        plan_a.export_plan.function_exports[0].function_id,
        module
            .executable
            .hir
            .start_function
            .expect("entry module should have start"),
        "exported function must be the start function"
    );
    assert_eq!(
        plan_a.export_plan.function_exports[0].export_name, "moth_start",
        "export name must be moth_start"
    );
    // Verify determinism.
    assert_eq!(
        plan_a.export_plan.function_exports[0].export_name,
        plan_b.export_plan.function_exports[0].export_name,
    );
}

#[test]
fn wasm_export_plan_wires_required_helper_exports() {
    let mut string_table = StringTable::new();
    let module = create_test_module(PathBuf::from("@page.moth"), &mut string_table);
    let reachability = entry_reachability(&module);

    let plan = build_html_wasm_plan(&module.executable.hir, &reachability, Vec::new())
        .expect("wasm plan should build");
    let helper = plan.wasm_request.export_policy.helper_exports;

    assert!(helper.export_memory);
    assert!(helper.export_str_ptr);
    assert!(helper.export_str_len);
    assert!(helper.export_vec_new);
    assert!(helper.export_vec_push);
    assert!(helper.export_vec_len);
    assert!(helper.export_vec_get);
    assert!(helper.export_release);
}

#[test]
fn compile_html_module_wasm_preserves_nested_logical_html_route() {
    let mut string_table = StringTable::new();
    let module = create_test_module(PathBuf::from("docs/@page.moth"), &mut string_table);
    let reachability = entry_reachability(&module);

    let page_metadata_plan = HtmlPageMetadataPlan::default();
    let compile_input = HtmlModuleCompileInput {
        hir_module: &module.executable.hir,
        resource_table: &module.executable.resource_table,
        reachability: &reachability,
        type_environment: &module.executable.type_environment,
        const_fragments: &[],
        borrow_analysis: &module.executable.borrow_analysis,
        page_metadata_plan: &page_metadata_plan,
        project_name: "",
        document_config: &HtmlDocumentConfig::default(),
        build_profile: crate::build_system::BuildProfile::Dev,
        root_activity: &module.metadata.root_activity,
        external_package_registry: Arc::new(
            crate::compiler_frontend::external_packages::ExternalPackageRegistry::new(),
        ),
    };
    let output_plan = HtmlResourceOutputPlan::new("");
    let resource_url_context = ResourceUrlContext::PageDocument(PathBuf::from("docs/index.html"));
    let structural_url_renderer =
        StructuralUrlRenderer::new(&output_plan, &resource_url_context, Some("/"));
    let compiled = compile_html_module_wasm(
        &compile_input,
        &mut string_table,
        Path::new("docs/index.html"),
        &structural_url_renderer,
    )
    .expect("wasm mode compilation should succeed for nested route");

    let output_paths: Vec<PathBuf> = compiled
        .output_files
        .iter()
        .map(|file| file.relative_output_path().to_path_buf())
        .collect();
    assert!(
        output_paths.contains(&PathBuf::from("docs/index.html")),
        "nested HTML route should be preserved, got: {output_paths:?}"
    );
    assert!(
        output_paths.contains(&PathBuf::from("docs/page.js")),
        "nested JS artifact should be colocated, got: {output_paths:?}"
    );
    assert!(
        output_paths.contains(&PathBuf::from("docs/page.wasm")),
        "nested Wasm artifact should be colocated, got: {output_paths:?}"
    );
    assert_eq!(compiled.html_output_path, PathBuf::from("docs/index.html"));
}
