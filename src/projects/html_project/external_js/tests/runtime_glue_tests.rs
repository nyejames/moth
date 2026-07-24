//! Tests for generated HTML JS glue and runtime module resolution.

use super::import_map::build_import_map_html;
use super::paths::relative_url_path;
use super::runtime_modules::emit_build_runtime_modules;
use super::source::{generate_fallible_wrapper, generate_infallible_wrapper};
use super::*;
use crate::build_system::build::{FileKind, Module};
use crate::compiler_frontend::external_packages::{
    ExternalAbiType, ExternalFunctionDef, ExternalFunctionId, ExternalFunctionLowerings,
    ExternalJsLowering, ExternalPackageId, ExternalPackageRegistry, ExternalReturnSlot,
    ExternalSignatureType,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::projects::html_project::external_js::runtime_emission_plan::HtmlExternalRuntimeEmissionPlan;
use crate::projects::html_project::tests::test_support::create_test_module;
use std::collections::HashSet;
use std::path::PathBuf;

#[test]
fn relative_url_path_same_directory() {
    let html = PathBuf::from("index.html");
    let asset = PathBuf::from("_moth/js/glue/module.js");
    assert_eq!(
        relative_url_path(&html, &asset),
        "./_moth/js/glue/module.js"
    );
}

#[test]
fn relative_url_path_one_level_deep() {
    let html = PathBuf::from("about/index.html");
    let asset = PathBuf::from("_moth/js/glue/module.js");
    assert_eq!(
        relative_url_path(&html, &asset),
        "../_moth/js/glue/module.js"
    );
}

#[test]
fn relative_url_path_two_levels_deep() {
    let html = PathBuf::from("a/b/index.html");
    let asset = PathBuf::from("_moth/js/runtime/moth-runtime.js");
    assert_eq!(
        relative_url_path(&html, &asset),
        "../../_moth/js/runtime/moth-runtime.js"
    );
}

#[test]
fn relative_url_path_shared_prefix() {
    let html = PathBuf::from("docs/index.html");
    let asset = PathBuf::from("docs/assets/file.js");
    assert_eq!(relative_url_path(&html, &asset), "./assets/file.js");
}

#[test]
fn generate_module_glue_returns_empty_when_no_external_exports() {
    let mut string_table = StringTable::new();
    let module = create_test_module(PathBuf::from("#page.moth"), &mut string_table);
    let referenced = HashSet::new();
    let registry = ExternalPackageRegistry::new();

    let result = generate_module_glue(
        &module,
        &referenced,
        &registry,
        &PathBuf::from("index.html"),
        false,
    )
    .expect("empty glue generation should succeed");

    assert!(result.glue_output_files.is_empty());
    assert!(result.bundle_import_preamble.is_none());
    assert!(result.import_map_html.is_none());
}

#[test]
fn generate_module_glue_empty_when_export_registered_but_not_referenced() {
    let mut string_table = StringTable::new();
    let mut module = create_test_module(PathBuf::from("#page.moth"), &mut string_table);
    module.link_facts.module_external_imports.push(
        crate::build_system::build::ModuleExternalImport {
            package_id: ExternalPackageId(0),
            runtime_asset: Some(
                crate::builder_surface::external_import_providers::provider::RuntimeAssetIdentity {
                    canonical_source_path: PathBuf::from("/project/lib.js"),
                    asset_kind: "js".to_owned(),
                },
            ),
            required_runtime_imports: Vec::new(),
        },
    );

    let (registry, _function_id, package_id) = create_registry_with_export("get_value", "getValue");
    module.link_facts.module_external_imports[0].package_id = package_id;

    // Export is registered but not referenced by emitted JS.
    let referenced = HashSet::new();

    let result = generate_module_glue(
        &module,
        &referenced,
        &registry,
        &PathBuf::from("index.html"),
        false,
    )
    .expect("glue generation should succeed");

    assert!(result.glue_output_files.is_empty());
    assert!(result.bundle_import_preamble.is_none());
    assert!(result.import_map_html.is_none());
}

#[test]
fn generate_module_glue_emits_glue_file_for_referenced_export() {
    let mut string_table = StringTable::new();
    let mut module = create_test_module(PathBuf::from("#page.moth"), &mut string_table);
    module.link_facts.module_external_imports.push(
        crate::build_system::build::ModuleExternalImport {
            package_id: ExternalPackageId(0),
            runtime_asset: Some(
                crate::builder_surface::external_import_providers::provider::RuntimeAssetIdentity {
                    canonical_source_path: PathBuf::from("/project/lib.js"),
                    asset_kind: "js".to_owned(),
                },
            ),
            required_runtime_imports: Vec::new(),
        },
    );

    let (registry, function_id, package_id) = create_registry_with_export("get_value", "getValue");
    module.link_facts.module_external_imports[0].package_id = package_id;
    let referenced = HashSet::from([function_id]);

    let result = generate_module_glue(
        &module,
        &referenced,
        &registry,
        &PathBuf::from("index.html"),
        false,
    )
    .expect("glue generation should succeed");

    assert_eq!(result.glue_output_files.len(), 1);
    let glue_file = &result.glue_output_files[0];
    assert!(
        glue_file
            .relative_output_path()
            .starts_with("_moth/js/glue/")
    );

    let FileKind::Js(source) = glue_file.file_kind() else {
        panic!("glue file must be JS");
    };
    assert!(
        source.contains("import { getValue as __moth_external_fn"),
        "missing aliased getValue import in:\n{}",
        source
    );
    assert!(source.contains("export function __moth_glue_fn"));
    assert!(source.contains("return __moth_external_fn"));

    assert!(result.bundle_import_preamble.is_some());
    let preamble = result.bundle_import_preamble.unwrap();
    assert!(preamble.starts_with("import { __moth_glue_fn"));
    assert!(preamble.contains("from \""));
}

#[test]
fn generate_module_glue_nested_html_output_path() {
    let mut string_table = StringTable::new();
    let mut module = create_test_module(PathBuf::from("#page.moth"), &mut string_table);
    module.link_facts.module_external_imports.push(
        crate::build_system::build::ModuleExternalImport {
            package_id: ExternalPackageId(0),
            runtime_asset: Some(
                crate::builder_surface::external_import_providers::provider::RuntimeAssetIdentity {
                    canonical_source_path: PathBuf::from("/project/lib.js"),
                    asset_kind: "js".to_owned(),
                },
            ),
            required_runtime_imports: Vec::new(),
        },
    );

    let (registry, function_id, package_id) = create_registry_with_export("get_value", "getValue");
    module.link_facts.module_external_imports[0].package_id = package_id;
    let referenced = HashSet::from([function_id]);

    let result = generate_module_glue(
        &module,
        &referenced,
        &registry,
        &PathBuf::from("a/b/index.html"),
        false,
    )
    .expect("glue generation should succeed");

    assert!(result.bundle_import_preamble.is_some());
    let preamble = result.bundle_import_preamble.unwrap();
    assert!(
        preamble.contains("from \"../../_moth/js/glue/module-"),
        "expected nested relative path in preamble, got:\n{preamble}"
    );
}

#[test]
fn generate_module_glue_asset_import_relative_to_glue_module() {
    let mut string_table = StringTable::new();
    let mut module = create_test_module(PathBuf::from("#page.moth"), &mut string_table);
    module.link_facts.module_external_imports.push(
        crate::build_system::build::ModuleExternalImport {
            package_id: ExternalPackageId(0),
            runtime_asset: Some(
                crate::builder_surface::external_import_providers::provider::RuntimeAssetIdentity {
                    canonical_source_path: PathBuf::from("/project/lib.js"),
                    asset_kind: "js".to_owned(),
                },
            ),
            required_runtime_imports: Vec::new(),
        },
    );

    let (registry, function_id, package_id) = create_registry_with_export("get_value", "getValue");
    module.link_facts.module_external_imports[0].package_id = package_id;
    let referenced = HashSet::from([function_id]);

    let result = generate_module_glue(
        &module,
        &referenced,
        &registry,
        &PathBuf::from("index.html"),
        false,
    )
    .expect("glue generation should succeed");

    let FileKind::Js(source) = result.glue_output_files[0].file_kind() else {
        panic!("glue file must be JS");
    };
    // Asset is at _moth/js/lib-{hash}.js; glue is at _moth/js/glue/module-{hash}.js.
    // Relative path from glue to asset should be ../lib-{hash}.js.
    assert!(
        source.contains("from \"../lib-"),
        "expected asset import relative to glue module, got:\n{source}"
    );
}

#[test]
fn generate_module_glue_fallible_wrapper_validates_result_shape() {
    let mut string_table = StringTable::new();
    let mut module = create_test_module(PathBuf::from("#page.moth"), &mut string_table);
    module.link_facts.module_external_imports.push(
        crate::build_system::build::ModuleExternalImport {
            package_id: ExternalPackageId(0),
            runtime_asset: Some(
                crate::builder_surface::external_import_providers::provider::RuntimeAssetIdentity {
                    canonical_source_path: PathBuf::from("/project/lib.js"),
                    asset_kind: "js".to_owned(),
                },
            ),
            required_runtime_imports: Vec::new(),
        },
    );

    let (registry, function_id, package_id) =
        create_registry_with_fallible_export("risky", "riskyOp");
    module.link_facts.module_external_imports[0].package_id = package_id;
    let referenced = HashSet::from([function_id]);

    let result = generate_module_glue(
        &module,
        &referenced,
        &registry,
        &PathBuf::from("index.html"),
        false,
    )
    .expect("fallible glue generation should succeed");

    let FileKind::Js(source) = result.glue_output_files[0].file_kind() else {
        panic!("glue file must be JS");
    };
    assert!(source.contains("try {"));
    assert!(source.contains("tag: \"ok\""));
    assert!(source.contains("tag: \"err\""));
    assert!(source.contains("catch (e)"));
}

#[test]
fn fallible_wrapper_handles_invalid_shape_differently_for_debug_and_release() {
    let debug_source = generate_fallible_wrapper("__moth_glue_fn1", "__moth_external_fn1", false);
    assert!(debug_source.contains("throw new Error("));
    assert!(debug_source.contains("Invalid result wrapper from external function"));

    let release_source = generate_fallible_wrapper("__moth_glue_fn1", "__moth_external_fn1", true);
    assert!(!release_source.contains("throw new Error("));
    assert!(
        release_source.contains("return { tag: \"err\", value: { b_fld0: \"Invalid result wrapper")
    );
    assert!(release_source.contains("b_fld1: 0"));
}

#[test]
fn fallible_wrapper_converts_external_errors_to_internal_error_fields() {
    let debug_source = generate_fallible_wrapper("__moth_glue_fn1", "__moth_external_fn1", false);

    assert!(
        debug_source.contains("moth_message_fld0: String(e.message || e)")
            && debug_source.contains("moth_code_fld1: 0"),
        "caught JS exceptions must become canonical Moth Error values"
    );
    assert!(
        debug_source.contains("moth_message_fld0: error.message || \"Unknown error\"")
            && debug_source
                .contains("moth_code_fld1: typeof error.code === \"number\" ? error.code : 0"),
        "external mothErr values must be translated into canonical Moth Error values"
    );
}

#[test]
fn infallible_wrapper_forwards_raw_arguments_and_return() {
    let source = generate_infallible_wrapper("__moth_glue_fn1", "__moth_external_fn1");
    assert!(source.contains("export function __moth_glue_fn1(...args)"));
    assert!(source.contains("return __moth_external_fn1(...args)"));
}

#[test]
fn emit_build_runtime_modules_dedupes_by_specifier() {
    let module_a = create_module_with_runtime_requirement();
    let module_b = create_module_with_runtime_requirement();

    let plan = HtmlExternalRuntimeEmissionPlan::from_modules(&[module_a, module_b]);
    let mut occupied = HashSet::new();
    let string_table = StringTable::new();
    let files = emit_build_runtime_modules(&plan, &mut occupied, &string_table)
        .expect("runtime module emission should succeed");

    // Only one runtime module emitted despite two modules requiring it.
    assert_eq!(files.len(), 1);
    assert!(files[0].relative_output_path().ends_with("moth-runtime.js"));
}

#[test]
fn emit_build_runtime_modules_rejects_unregistered_specifier() {
    let mut module = create_module_with_runtime_requirement();
    module.link_facts.module_external_imports[0].required_runtime_imports[0].module_name =
        "@moth/missing".to_owned();

    let plan = HtmlExternalRuntimeEmissionPlan::from_modules(&[module]);
    let mut occupied = HashSet::new();
    let string_table = StringTable::new();
    let error = match emit_build_runtime_modules(&plan, &mut occupied, &string_table) {
        Ok(_) => panic!("unregistered runtime module should fail"),
        Err(error) => error,
    };
    let (_, message, _) = error
        .first_infrastructure_error_for_tests()
        .expect("runtime module failure should be an infrastructure error");

    assert!(
        message.contains("@moth/missing"),
        "expected unregistered module name in error"
    );
}

#[test]
fn build_import_map_html_includes_moth_runtime() {
    let module = create_module_with_runtime_requirement();
    let html = build_import_map_html(&module, &PathBuf::from("index.html"));

    assert!(html.is_some());
    let map = html.unwrap();
    assert!(map.contains("<script type=\"importmap\">"));
    assert!(map.contains("@moth/runtime"));
    assert!(map.contains("./_moth/js/runtime/moth-runtime.js"));
}

#[test]
fn build_import_map_html_deduplicates_by_specifier() {
    let mut module = create_module_with_runtime_requirement();
    module.link_facts.module_external_imports.push(
        crate::build_system::build::ModuleExternalImport {
            package_id: ExternalPackageId(1),
            runtime_asset: None,
            required_runtime_imports: vec![
                crate::builder_surface::external_import_providers::provider::RequiredRuntimeImport {
                    module_name: "@moth/runtime".to_owned(),
                    imported_names: vec!["mothOk".to_owned()],
                },
            ],
        },
    );

    let html = build_import_map_html(&module, &PathBuf::from("index.html"));
    assert!(html.is_some());
    let map = html.unwrap();

    let occurrences = map.matches("@moth/runtime").count();
    assert_eq!(
        occurrences, 1,
        "expected exactly one @moth/runtime entry, got:\n{map}"
    );
}

// Test helpers

fn create_module_with_runtime_requirement() -> Module {
    let mut string_table = StringTable::new();
    let mut module = create_test_module(PathBuf::from("#page.moth"), &mut string_table);
    module.link_facts.module_external_imports.push(
        crate::build_system::build::ModuleExternalImport {
            package_id: ExternalPackageId(0),
            runtime_asset: None,
            required_runtime_imports: vec![
                crate::builder_surface::external_import_providers::provider::RequiredRuntimeImport {
                    module_name: "@moth/runtime".to_owned(),
                    imported_names: vec!["mothOk".to_owned(), "mothErr".to_owned()],
                },
            ],
        },
    );
    module
}

fn create_registry_with_export(
    name: &str,
    export_name: &str,
) -> (
    ExternalPackageRegistry,
    ExternalFunctionId,
    ExternalPackageId,
) {
    let mut registry = ExternalPackageRegistry::new();
    let package_id = registry
        .register_package(
            "test/pkg",
            crate::builder_surface::PackageOrigin::ProjectLocal,
        )
        .unwrap();
    let function_id = ExternalFunctionId::Synthetic(42);
    registry
        .register_function_in_package(
            package_id,
            function_id,
            ExternalFunctionDef {
                name: name.to_owned(),
                parameters: Vec::new(),
                returns: vec![ExternalReturnSlot::fresh(ExternalAbiType::I32)],
                error_return_type: None,
                lowerings: ExternalFunctionLowerings {
                    js: Some(ExternalJsLowering::ExternalModuleExport {
                        export_name: export_name.to_owned(),
                    }),
                    wasm: None,
                },
            },
        )
        .unwrap();
    (registry, function_id, package_id)
}

fn create_registry_with_fallible_export(
    name: &str,
    export_name: &str,
) -> (
    ExternalPackageRegistry,
    ExternalFunctionId,
    ExternalPackageId,
) {
    let mut registry = ExternalPackageRegistry::new();
    let package_id = registry
        .register_package(
            "test/pkg",
            crate::builder_surface::PackageOrigin::ProjectLocal,
        )
        .unwrap();
    let function_id = ExternalFunctionId::Synthetic(43);
    registry
        .register_function_in_package(
            package_id,
            function_id,
            ExternalFunctionDef {
                name: name.to_owned(),
                parameters: Vec::new(),
                returns: vec![ExternalReturnSlot::fresh(ExternalAbiType::I32)],
                error_return_type: Some(ExternalSignatureType::BuiltinError),
                lowerings: ExternalFunctionLowerings {
                    js: Some(ExternalJsLowering::ExternalModuleExport {
                        export_name: export_name.to_owned(),
                    }),
                    wasm: None,
                },
            },
        )
        .unwrap();
    (registry, function_id, package_id)
}
