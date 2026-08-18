//! Tests for dev-server error page rendering helpers.

use super::{
    escape_html, format_compiler_messages, render_compiler_error_page, render_runtime_error_page,
};
use crate::compiler_frontend::compiler_errors::{CompilerMessages, SourceLocation};
use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, InvalidConfigReason};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::CharPosition;
use crate::compiler_tests::test_support::unused_temp_path;
use std::fs;

#[test]
fn escape_html_rewrites_special_characters() {
    let escaped = escape_html(r#"<tag attr="x">Tom & Jerry's</tag>"#);
    assert_eq!(
        escaped,
        "&lt;tag attr=&quot;x&quot;&gt;Tom &amp; Jerry&#39;s&lt;/tag&gt;"
    );
}

#[test]
fn rendered_runtime_page_includes_version_error_text_and_dark_mode() {
    let page = render_runtime_error_page("Title", "something broke", "/preview", 14);
    assert!(page.contains("Build Version: 14"));
    assert!(page.contains("something broke"));
    assert!(page.contains("Timestamp (unix):"));
    assert!(page.contains("color-scheme: dark"));
    assert!(page.contains("EventSource('/preview/__moth/events')"));
}

#[test]
fn formatted_compiler_messages_include_typed_diagnostics() {
    let diagnostic = CompilerDiagnostic::invalid_config_reason(
        None,
        InvalidConfigReason::UnsupportedScalarValue,
        SourceLocation::default(),
    );
    let messages = CompilerMessages::from_diagnostic(diagnostic, StringTable::new());

    let formatted = format_compiler_messages(&messages);

    assert!(formatted.contains("MOTH-CONFIG-0001"));
    assert!(formatted.contains("Unsupported value"));
}

#[test]
fn compiler_error_page_links_to_project_relative_resolved_source_path() {
    let root = unused_temp_path("relative_path");
    let source_file = root.join("src/docs/guide.moth");
    fs::create_dir_all(
        source_file
            .parent()
            .expect("source file should have a parent directory"),
    )
    .expect("should create source dir");
    fs::write(&source_file, "broken()\n").expect("should write source file");

    let header_scope = source_file.join("start.header");
    let mut string_table = StringTable::new();
    let diagnostic = CompilerDiagnostic::invalid_config_reason(
        None,
        InvalidConfigReason::UnsupportedScalarValue,
        SourceLocation::new(
            InternedPath::try_from_filesystem_path(&header_scope, &mut string_table)
                .expect("test path should be UTF-8"),
            CharPosition {
                line_number: 0,
                char_column: 4,
            },
            CharPosition {
                line_number: 0,
                char_column: 7,
            },
        ),
    );
    let messages = CompilerMessages::from_diagnostic(diagnostic, string_table);

    let page = render_compiler_error_page(&messages, &root, "/docs", 7);

    // The browser card should not visibly show MOTH-* codes but should
    // carry them as data attributes for debugging.
    assert!(page.contains("color-scheme: dark"));
    assert!(page.contains("data-diagnostic-code=\"MOTH-CONFIG-0001\""));
    assert!(!page.contains(">MOTH-CONFIG-0001<"));
    assert!(page.contains("guide.moth"));
    assert!(page.contains("--> src/docs/guide.moth:1:5"));
    assert!(page.contains("Unsupported value"));
    assert!(!page.contains("start.header"));

    // Source frame with underline carets.
    assert!(page.contains("source-caret"));

    // Simple file:// link to the resolved source path.
    assert!(page.contains("file://"));

    // SSE client is injected.
    assert!(page.contains("EventSource('/docs/__moth/events')"));

    fs::remove_dir_all(&root).expect("should remove temp dir");
}
