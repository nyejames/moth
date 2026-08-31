//! Tests for `HtmlExternalRuntimeEmissionPlan`.

use crate::builder_surface::external_import_providers::provider::RequiredRuntimeImport;
use crate::compiler_frontend::external_packages::ExternalPackageId;
use crate::compiler_frontend::module_compilation::ModuleExternalImport;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::projects::html_project::external_js::runtime_emission_plan::HtmlExternalRuntimeEmissionPlan;
use crate::projects::html_project::tests::test_support::{
    create_test_module, js_runtime_asset_import, non_js_runtime_asset_import,
};
use std::path::{Path, PathBuf};

#[test]
fn plan_collects_js_assets_by_provider_origin() {
    let mut string_table = StringTable::new();
    let mut module = create_test_module(PathBuf::from("@page.moth"), &mut string_table);
    let asset = js_runtime_asset_import(Path::new("lib.js"), PathBuf::from("/project/lib.js"));
    let asset_origin = asset.origin.clone();
    module.link_facts.external_import_candidates = vec![ModuleExternalImport {
        package_id: ExternalPackageId(1),
        runtime_asset: Some(asset),
        required_runtime_imports: vec![],
    }];

    let plan = HtmlExternalRuntimeEmissionPlan::from_import_sets([module
        .link_facts
        .external_import_candidates
        .as_slice()]);

    assert_eq!(plan.js_assets().len(), 1);
    assert!(plan.js_assets().contains_key(&asset_origin));
}

#[test]
fn plan_ignores_non_js_assets() {
    let mut string_table = StringTable::new();
    let mut module = create_test_module(PathBuf::from("@page.moth"), &mut string_table);
    module.link_facts.external_import_candidates = vec![ModuleExternalImport {
        package_id: ExternalPackageId(1),
        runtime_asset: Some(non_js_runtime_asset_import(
            "css",
            PathBuf::from("/project/lib.css"),
        )),
        required_runtime_imports: vec![],
    }];

    let plan = HtmlExternalRuntimeEmissionPlan::from_import_sets([module
        .link_facts
        .external_import_candidates
        .as_slice()]);

    assert!(plan.js_assets().is_empty());
}

#[test]
fn plan_collects_runtime_module_specifiers() {
    let mut string_table = StringTable::new();
    let mut module = create_test_module(PathBuf::from("@page.moth"), &mut string_table);
    module.link_facts.external_import_candidates = vec![ModuleExternalImport {
        package_id: ExternalPackageId(1),
        runtime_asset: None,
        required_runtime_imports: vec![RequiredRuntimeImport {
            module_name: "@moth/runtime".to_owned(),
            imported_names: vec!["mothOk".to_owned()],
        }],
    }];

    let plan = HtmlExternalRuntimeEmissionPlan::from_import_sets([module
        .link_facts
        .external_import_candidates
        .as_slice()]);

    assert_eq!(plan.runtime_module_specifiers().len(), 1);
    assert!(plan.runtime_module_specifiers().contains("@moth/runtime"));
}

#[test]
fn plan_dedupes_js_assets_across_modules() {
    let mut string_table = StringTable::new();
    let mut module_a = create_test_module(PathBuf::from("@page.moth"), &mut string_table);
    module_a.link_facts.external_import_candidates = vec![ModuleExternalImport {
        package_id: ExternalPackageId(1),
        runtime_asset: Some(js_runtime_asset_import(
            Path::new("lib.js"),
            PathBuf::from("/project/lib.js"),
        )),
        required_runtime_imports: vec![],
    }];

    let mut module_b = create_test_module(PathBuf::from("docs/@page.moth"), &mut string_table);
    module_b.link_facts.external_import_candidates = vec![ModuleExternalImport {
        package_id: ExternalPackageId(2),
        runtime_asset: Some(js_runtime_asset_import(
            Path::new("lib.js"),
            PathBuf::from("/project/lib.js"),
        )),
        required_runtime_imports: vec![],
    }];

    let plan = HtmlExternalRuntimeEmissionPlan::from_import_sets([
        module_a.link_facts.external_import_candidates.as_slice(),
        module_b.link_facts.external_import_candidates.as_slice(),
    ]);

    assert_eq!(plan.js_assets().len(), 1);
}

#[test]
fn plan_dedupes_runtime_specifiers_across_modules() {
    let mut string_table = StringTable::new();
    let mut module_a = create_test_module(PathBuf::from("@page.moth"), &mut string_table);
    module_a.link_facts.external_import_candidates = vec![ModuleExternalImport {
        package_id: ExternalPackageId(1),
        runtime_asset: None,
        required_runtime_imports: vec![RequiredRuntimeImport {
            module_name: "@moth/runtime".to_owned(),
            imported_names: vec!["mothOk".to_owned()],
        }],
    }];

    let mut module_b = create_test_module(PathBuf::from("docs/@page.moth"), &mut string_table);
    module_b.link_facts.external_import_candidates = vec![ModuleExternalImport {
        package_id: ExternalPackageId(2),
        runtime_asset: None,
        required_runtime_imports: vec![RequiredRuntimeImport {
            module_name: "@moth/runtime".to_owned(),
            imported_names: vec!["mothErr".to_owned()],
        }],
    }];

    let plan = HtmlExternalRuntimeEmissionPlan::from_import_sets([
        module_a.link_facts.external_import_candidates.as_slice(),
        module_b.link_facts.external_import_candidates.as_slice(),
    ]);

    assert_eq!(plan.runtime_module_specifiers().len(), 1);
}

#[test]
fn plan_empty_modules_produces_empty_plan() {
    let plan = HtmlExternalRuntimeEmissionPlan::from_import_sets(std::iter::empty::<
        &[ModuleExternalImport],
    >());

    assert!(plan.js_assets().is_empty());
    assert!(plan.runtime_module_specifiers().is_empty());
}
