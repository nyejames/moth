//! Tests for shared HTML shell rendering.

use super::*;
use crate::projects::html_project::document_config::HtmlDocumentConfig;
use crate::projects::html_project::page_metadata::HtmlPageMetadata;
use crate::projects::html_project::tests::test_support::{
    assert_fragment_before_body_close, assert_has_basic_shell,
};
use std::path::Path;

fn render_shell(
    config: &HtmlDocumentConfig,
    page_metadata: &HtmlPageMetadata,
    logical_html_path: &str,
    project_name: &str,
    body_html: &str,
    script_html: &str,
) -> String {
    render_html_document_shell(
        config,
        page_metadata,
        Path::new(logical_html_path),
        project_name,
        body_html.to_owned(),
        script_html.to_owned(),
        None,
    )
    .expect("document shell should render for valid route inputs")
}

#[test]
fn renderer_outputs_full_document_shell() {
    let html = render_shell(
        &HtmlDocumentConfig::default(),
        &HtmlPageMetadata::default(),
        "index.html",
        "",
        "<h1>Hello</h1>\n",
        "<script>start()</script>\n",
    );

    assert_has_basic_shell(&html);
    assert!(html.contains("<html lang=\"en\">"));
}

#[test]
fn renderer_uses_route_title_fallback_before_project_name() {
    let html = render_shell(
        &HtmlDocumentConfig::default(),
        &HtmlPageMetadata::default(),
        "docs/basics/index.html",
        "Project",
        "",
        "",
    );

    assert!(html.contains("<title>Basics</title>"));
}

#[test]
fn renderer_keeps_script_inside_body() {
    let script = "<script>const message = `first\n    nested`;\nbootstrap(message)</script>\n";
    let html = render_shell(
        &HtmlDocumentConfig::default(),
        &HtmlPageMetadata::default(),
        "index.html",
        "",
        "<div>content</div>\n",
        script,
    );

    assert_fragment_before_body_close(
        &html,
        "<script>const message = `first\n    nested`;\nbootstrap(message)</script>",
    );
    assert!(html.contains(script));
}

#[test]
fn renderer_preserves_whitespace_sensitive_body_content() {
    let body = "<pre><code>first\n    nested\n</code></pre>";
    let html = render_shell(
        &HtmlDocumentConfig::default(),
        &HtmlPageMetadata::default(),
        "index.html",
        "",
        body,
        "",
    );

    assert!(html.contains(body));
    assert!(!html.contains("    <pre><code>first\n        nested"));
}

#[test]
fn renderer_adds_only_missing_fragment_separator_newline() {
    let ending_with_newline = render_shell(
        &HtmlDocumentConfig::default(),
        &HtmlPageMetadata::default(),
        "index.html",
        "",
        "<span>one</span>\n",
        "",
    );
    assert!(ending_with_newline.contains("<span>one</span>\n  </body>"));
    assert!(!ending_with_newline.contains("<span>one</span>\n\n  </body>"));

    let without_newline = render_shell(
        &HtmlDocumentConfig::default(),
        &HtmlPageMetadata::default(),
        "index.html",
        "",
        "<span>two</span>",
        "",
    );
    assert!(without_newline.contains("<span>two</span>\n  </body>"));
}

#[test]
fn renderer_injects_codeblock_scroll_styles() {
    let html = render_shell(
        &HtmlDocumentConfig::default(),
        &HtmlPageMetadata::default(),
        "index.html",
        "",
        "<h1>Hello</h1>\n",
        "",
    );

    let codeblock_rule = extract_css_rule(&html, ".codeblock");

    assert!(
        codeblock_rule.contains("overflow-x: auto"),
        "expected .codeblock to set overflow-x: auto, got: {codeblock_rule}"
    );
    assert!(
        codeblock_rule.contains("white-space: pre"),
        "expected .codeblock to set white-space: pre, got: {codeblock_rule}"
    );
}

/// Extracts the first CSS rule block that starts with `selector`.
///
/// WHAT: finds the selector in the CSS and returns the text between the following `{` and the
///       matching `}`.
/// WHY: lets tests assert properties within a specific rule without being fooled by the same
///      property appearing in unrelated rules.
fn extract_css_rule<'a>(css: &'a str, selector: &str) -> &'a str {
    let selector_start = css
        .find(selector)
        .unwrap_or_else(|| panic!("expected CSS to contain selector '{selector}'"));
    let block_start = css[selector_start..]
        .find('{')
        .map(|offset| selector_start + offset)
        .expect("expected opening brace after selector");
    let block_end = css[block_start..]
        .find('}')
        .map(|offset| block_start + offset)
        .expect("expected closing brace for selector block");

    &css[block_start..=block_end]
}

#[test]
fn renderer_injects_every_shared_code_role_selector() {
    let html = render_shell(
        &HtmlDocumentConfig::default(),
        &HtmlPageMetadata::default(),
        "index.html",
        "",
        "<h1>Hello</h1>\n",
        "",
    );

    let roles = [
        ("moth-code-comment", "moth-code-comment"),
        ("moth-code-keyword", "moth-code-keyword"),
        ("moth-code-literal", "moth-code-literal"),
        ("moth-code-string", "moth-code-string"),
        ("moth-code-number", "moth-code-number"),
        ("moth-code-operator", "moth-code-operator"),
        ("moth-code-nominal", "moth-code-nominal"),
        ("moth-code-type", "moth-code-type"),
        ("moth-code-delimiter", "moth-code-delimiter"),
        ("moth-code-function", "moth-code-function"),
        ("moth-code-directive", "moth-code-directive"),
        ("moth-code-contract", "moth-code-contract"),
    ];

    for (selector, variable) in roles {
        let rule = extract_css_rule(&html, &format!(".{selector}"));
        assert!(
            rule.contains(&format!("var(--{variable})")),
            "expected {selector} to use var(--{variable}), got: {rule}"
        );
    }
}

#[test]
fn renderer_no_longer_emits_old_code_role_names() {
    let html = render_shell(
        &HtmlDocumentConfig::default(),
        &HtmlPageMetadata::default(),
        "index.html",
        "",
        "<h1>Hello</h1>\n",
        "",
    );

    assert!(!html.contains("moth-code-struct"));
    assert!(!html.contains("moth-code-parenthesis"));
    assert!(!html.contains("var(--comment)"));
    assert!(!html.contains("var(--keyword)"));
    assert!(!html.contains("var(--struct)"));
    assert!(!html.contains("var(--parenthesis)"));
}
