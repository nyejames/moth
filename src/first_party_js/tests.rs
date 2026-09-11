use super::inventoried_javascript_sources;

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
            .any(|label| label.contains("inline-js:@core/math")),
        "math inline expressions must be inventoried: {labels:?}"
    );
}
