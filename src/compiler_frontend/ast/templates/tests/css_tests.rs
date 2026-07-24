use super::*;

#[test]
fn valid_block_css_emits_no_warnings() {
    let warnings = validate_css_source(
        ".button { color: red; }\n@media (width > 600px) { .button { padding: 1rem; } }",
        CssFormatterMode::Block,
    );

    assert!(warnings.is_empty());
}

#[test]
fn valid_block_css_ignores_comments_inside_statements() {
    let warnings = validate_css_source(
        ":root { /* Default Background Colours */ --moth-bg-lightmode: #fff; /* Code block colours */ --comment-dark: #838c86; }",
        CssFormatterMode::Block,
    );

    assert!(warnings.is_empty());
}

#[test]
fn inline_css_rejects_selector_blocks() {
    let warnings = validate_css_source(".button { color: red; }", CssFormatterMode::Inline);
    assert!(
        warnings
            .iter()
            .any(|warning| warning.message.contains("only allow declarations"))
    );
}

#[test]
fn malformed_css_reports_balancing_and_declaration_shape() {
    let warnings = validate_css_source(".button { color red; ", CssFormatterMode::Block);
    assert!(
        warnings
            .iter()
            .any(|warning| warning.message.contains("Unclosed '{'"))
    );
    assert!(
        warnings
            .iter()
            .any(|warning| warning.message.contains("Expected 'property: value'"))
    );
}
