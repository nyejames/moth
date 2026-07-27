//! Tests for `HtmlExternalRuntimeEmissionPlan`.

use crate::build_system::build::ModuleExternalImport;
use crate::builder_surface::external_import_providers::provider::{
    RequiredRuntimeImport, RuntimeAssetIdentity,
};
use crate::compiler_frontend::external_packages::ExternalPackageId;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::projects::html_project::external_js::runtime_emission_plan::HtmlExternalRuntimeEmissionPlan;
use crate::projects::html_project::tests::test_support::create_test_module;
use std::path::PathBuf;

#[test]
fn plan_collects_js_assets_by_canonical_path() {
    let mut string_table = StringTable::new();
    let mut module = create_test_module(PathBuf::from("#page.moth"), &mut string_table);
    module.link_facts.external_import_candidates = vec![ModuleExternalImport {
        package_id: ExternalPackageId(1),
        runtime_asset: Some(RuntimeAssetIdentity {
            canonical_source_path: PathBuf::from("/project/lib.js"),
            asset_kind: "js".to_owned(),
        }),
        required_runtime_imports: vec![],
    }];

    let plan = HtmlExternalRuntimeEmissionPlan::from_import_sets([module
        .link_facts
        .external_import_candidates
        .as_slice()]);

    assert_eq!(plan.js_assets().len(), 1);
    assert!(
        plan.js_assets()
            .contains_key(&PathBuf::from("/project/lib.js"))
    );
}

#[test]
fn plan_ignores_non_js_assets() {
    let mut string_table = StringTable::new();
    let mut module = create_test_module(PathBuf::from("#page.moth"), &mut string_table);
    module.link_facts.external_import_candidates = vec![ModuleExternalImport {
        package_id: ExternalPackageId(1),
        runtime_asset: Some(RuntimeAssetIdentity {
            canonical_source_path: PathBuf::from("/project/lib.css"),
            asset_kind: "css".to_owned(),
        }),
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
    let mut module = create_test_module(PathBuf::from("#page.moth"), &mut string_table);
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
    let mut module_a = create_test_module(PathBuf::from("#page.moth"), &mut string_table);
    module_a.link_facts.external_import_candidates = vec![ModuleExternalImport {
        package_id: ExternalPackageId(1),
        runtime_asset: Some(RuntimeAssetIdentity {
            canonical_source_path: PathBuf::from("/project/lib.js"),
            asset_kind: "js".to_owned(),
        }),
        required_runtime_imports: vec![],
    }];

    let mut module_b = create_test_module(PathBuf::from("docs/#page.moth"), &mut string_table);
    module_b.link_facts.external_import_candidates = vec![ModuleExternalImport {
        package_id: ExternalPackageId(2),
        runtime_asset: Some(RuntimeAssetIdentity {
            canonical_source_path: PathBuf::from("/project/lib.js"),
            asset_kind: "js".to_owned(),
        }),
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
    let mut module_a = create_test_module(PathBuf::from("#page.moth"), &mut string_table);
    module_a.link_facts.external_import_candidates = vec![ModuleExternalImport {
        package_id: ExternalPackageId(1),
        runtime_asset: None,
        required_runtime_imports: vec![RequiredRuntimeImport {
            module_name: "@moth/runtime".to_owned(),
            imported_names: vec!["mothOk".to_owned()],
        }],
    }];

    let mut module_b = create_test_module(PathBuf::from("docs/#page.moth"), &mut string_table);
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
