//! Tests for dev-server error page rendering helpers.

use super::{
    escape_html, format_compiler_messages, render_compiler_error_page, render_runtime_error_page,
};
use crate::compiler_frontend::compiler_errors::CompilerMessages;
use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, InvalidConfigReason};
use crate::compiler_frontend::source::{
    ExtendedSpanBuilder, LocalSpan, SourceDatabase, SourceSpan,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use std::fs;
use std::sync::Arc;

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
        None,
    );
    let messages = CompilerMessages::from_diagnostic(diagnostic, StringTable::new());

    let formatted = format_compiler_messages(&messages);

    assert!(formatted.contains("MOTH-CONFIG-0001"));
    assert!(formatted.contains("Unsupported value"));
}

#[test]
fn compiler_error_page_links_to_project_relative_resolved_source_path() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let source_file = root.join("src/docs/guide.moth");
    fs::create_dir_all(
        source_file
            .parent()
            .expect("source file should have a parent directory"),
    )
    .expect("should create source dir");
    fs::write(&source_file, "broken()\n").expect("should write source file");

    let mut string_table = StringTable::new();
    let mut source_database = SourceDatabase::build(
        std::iter::once(source_file.as_path()),
        &root.join("main.moth"),
        None,
        &mut string_table,
    )
    .expect("source identity should build");
    let source_id = source_database
        .get_by_canonical_path(&source_file)
        .expect("source should be registered")
        .id;
    source_database
        .retain_text(source_id, "broken()\n".to_owned())
        .expect("source text should be retained");
    let mut span_builder = ExtendedSpanBuilder::new();
    let local_span = LocalSpan::exact(4, 3, &mut span_builder).expect("span should fit inline");
    let source_span = SourceSpan::new(source_id, local_span);
    let diagnostic = CompilerDiagnostic::invalid_config_reason(
        None,
        InvalidConfigReason::UnsupportedScalarValue,
        Some(source_span),
    );
    let mut messages = CompilerMessages::from_diagnostic(diagnostic, string_table);
    messages.set_source_database(Arc::new(source_database));

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
}
