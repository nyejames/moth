use super::{
    FirstPartyJavascriptImportFindingKind, inventoried_javascript_sources,
    javascript_import_findings,
};

#[test]
fn inventory_includes_runtime_helpers_and_inline_templates() {
    let labels: Vec<_> = inventoried_javascript_sources()
        .into_iter()
        .map(|source| source.label)
        .collect();

    assert!(
        labels
            .iter()
            .any(|label| label == "runtime-module:@moth/runtime"),
        "runtime module must be inventoried: {labels:?}"
    );
    assert!(
        labels
            .iter()
            .any(|label| label == "core-collections-js-helper:__moth_fixed_collection"),
        "fixed collection helper must be inventoried: {labels:?}"
    );
    assert!(
        labels
            .iter()
            .any(|label| label == "core-js-helper:__moth_text_length"),
        "text helper must be inventoried: {labels:?}"
    );
    assert!(
        labels
            .iter()
            .any(|label| label == "core-js-helper:__moth_random_int_bigint"),
        "BigInt random helper must be inventoried: {labels:?}"
    );
    assert!(
        labels
            .iter()
            .any(|label| label.contains("inline-js:@core/math")),
        "math inline expressions must be inventoried: {labels:?}"
    );
}

#[test]
fn constant_initializers_cannot_hide_third_party_module_loading() {
    for initializer in ["require(\"third-party\")", "load(import(\"third-party\"))"] {
        let source = format!("export const dependency = {initializer};");
        let findings = javascript_import_findings(&source);
        assert_eq!(
            findings
                .iter()
                .map(|finding| finding.kind)
                .collect::<Vec<_>>(),
            vec![FirstPartyJavascriptImportFindingKind::UnapprovedModuleImport],
            "module-loading initializer {initializer:?} must violate first-party policy"
        );
    }
}

#[test]
fn constant_initializer_lookalikes_do_not_load_modules() {
    let source = r#"
export const dependency = "require('third-party') import('third-party')";
export const mode = /* require("third-party") import("third-party") */ 4;
export const template = `require("third-party") import("third-party")`;
export const pattern = /import("third-party")/;
"#;
    assert!(javascript_import_findings(source).is_empty());
}
