//! Tests for dev-server error page rendering helpers.

use super::{
    escape_html, format_compiler_messages, render_compiler_error_page, render_runtime_error_page,
};
use crate::compiler_frontend::compiler_errors::{CompilationOutcome, CompilationResult};
use crate::compiler_frontend::compiler_messages::diagnostics::boundary_accumulator::BoundaryAccumulator;
use crate::compiler_frontend::compiler_messages::diagnostics::families::project_config;
use crate::compiler_frontend::compiler_messages::diagnostics::freeze::InstallMode;
use crate::compiler_frontend::compiler_messages::prepared_batch::PreparedDiagnosticBatch;
use crate::compiler_frontend::compiler_messages::{
    DiagnosticBag, DiagnosticDiscoveryStops, DiagnosticDraft, InvalidConfigReason,
};
use crate::compiler_frontend::source::{
    ExtendedSpanBuilder, LocalSpan, SourceDatabase, SourceSpan,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use std::sync::Arc;

fn diagnostic_result(
    diagnostic: DiagnosticDraft,
    string_table: StringTable,
    source_database: Option<Arc<SourceDatabase>>,
) -> CompilationResult<()> {
    let batch = PreparedDiagnosticBatch::capture_from_bag(
        DiagnosticBag::from(diagnostic),
        string_table,
        None,
        None,
    );
    let mut accumulator = BoundaryAccumulator::new();
    accumulator.append_prepared(batch, source_database.map(|database| (database, None)));
    accumulator
        .finish(InstallMode::Frozen)
        .map(CompilationOutcome::Diagnosed)
}

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
    assert!(page.contains("EventSource(\"/preview/__moth/events\")"));
}

#[test]
fn formatted_compiler_messages_include_typed_diagnostics() {
    let diagnostic = project_config::invalid_config_reason(
        None,
        InvalidConfigReason::UnsupportedScalarValue,
        None,
    );
    let failure = diagnostic_result(diagnostic, StringTable::new(), None);
    let formatted = format_compiler_messages(&failure);

    assert!(formatted.contains("MOTH-CONFIG-0001"));
    assert!(formatted.contains("Unsupported value"));
}

#[test]
fn compiler_error_page_links_to_project_relative_resolved_source_path() {
    // A synthetic absolute root keeps the expected href literal on each platform. Source identity
    // construction and rendering do not read the file.
    #[cfg(unix)]
    let root = std::path::PathBuf::from("/moth-test/project");
    #[cfg(windows)]
    let root = std::path::PathBuf::from(r"C:\moth-test\project");
    let source_file = root.join("src/docs/guide #100% & 'ü'.moth");

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
    let diagnostic = project_config::invalid_config_reason(
        None,
        InvalidConfigReason::UnsupportedScalarValue,
        Some(source_span),
    );
    let failure = diagnostic_result(diagnostic, string_table, Some(Arc::new(source_database)));
    let page =
        render_compiler_error_page(&failure, &root, "/docs", 7, DiagnosticDiscoveryStops::NONE);

    // The browser card should not visibly show MOTH-* codes but should
    // carry them as data attributes for debugging.
    assert!(page.contains("color-scheme: dark"));
    assert!(page.contains("data-diagnostic-code=\"MOTH-CONFIG-0001\""));
    assert!(!page.contains(">MOTH-CONFIG-0001<"));
    assert!(page.contains("--> src/docs/guide #100% &amp; &#39;ü&#39;.moth:1:5"));
    assert!(page.contains("Unsupported value"));
    assert!(!page.contains("start.header"));

    assert!(page.contains("source-caret"));

    // The href is a URL: each path segment is percent-encoded before the completed attribute is
    // HTML-escaped, so the source name's `#`, `%`, `&`, `'`, space and Unicode stay in the path.
    #[cfg(unix)]
    let expected_href =
        "href=\"file:///moth-test/project/src/docs/guide%20%23100%25%20%26%20%27%C3%BC%27.moth\"";
    #[cfg(windows)]
    let expected_href = "href=\"file:///C:/moth-test/project/src/docs/guide%20%23100%25%20%26%20%27%C3%BC%27.moth\"";
    assert!(page.contains(expected_href), "{page}");
    assert!(
        !page.contains("guide #100% &amp; &#39;ü&#39;.moth\""),
        "{page}"
    );

    // SSE client is injected.
    assert!(page.contains("EventSource(\"/docs/__moth/events\")"));
}

/// A host path with bytes that are not UTF-8 cannot be represented faithfully as a file URL, so
/// the source frame keeps the portable label instead of linking to a different path.
#[cfg(unix)]
#[test]
fn compiler_error_page_omits_source_link_for_non_utf8_host_path() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    use std::path::PathBuf;

    let root = PathBuf::from("/moth-test").join(OsString::from_vec(b"non-utf8-\xFF".to_vec()));
    let source_file = root.join("src/docs/guide.moth");

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
    let diagnostic = project_config::invalid_config_reason(
        None,
        InvalidConfigReason::UnsupportedScalarValue,
        Some(SourceSpan::new(source_id, local_span)),
    );
    let failure = diagnostic_result(diagnostic, string_table, Some(Arc::new(source_database)));
    let page =
        render_compiler_error_page(&failure, &root, "/docs", 7, DiagnosticDiscoveryStops::NONE);

    assert!(
        page.contains(r#"<span class="source-location">--> src/docs/guide.moth:1:5</span>"#),
        "{page}"
    );
    assert!(!page.contains("href=\"file://"), "{page}");
}

#[test]
fn compiler_error_page_renders_one_combined_stop_line_for_stopped_discovery() {
    let root = tempfile::tempdir().expect("should create temp dir");
    let diagnostic = project_config::invalid_config_reason(
        None,
        InvalidConfigReason::UnsupportedScalarValue,
        None,
    );
    let failure = diagnostic_result(diagnostic, StringTable::new(), None);

    let mut discovery_stops = DiagnosticDiscoveryStops::MODULE_ERROR_ALLOWANCE;
    discovery_stops.insert(DiagnosticDiscoveryStops::AST_RECOVERY_WORK_GUARD);
    let expected_stop_line = discovery_stops
        .stop_line()
        .expect("two stop reasons must combine into one stop line");

    let stopped_page =
        render_compiler_error_page(&failure, root.path(), "/docs", 7, discovery_stops);
    assert_eq!(
        stopped_page
            .matches("Diagnostic discovery stopped early")
            .count(),
        1,
        "multi-reason discovery must render exactly one stop line"
    );
    assert!(
        stopped_page.contains(&escape_html(&expected_stop_line)),
        "the single stop line must name every reason"
    );

    let complete_page = render_compiler_error_page(
        &failure,
        root.path(),
        "/docs",
        7,
        DiagnosticDiscoveryStops::NONE,
    );
    assert!(
        !complete_page.contains("Diagnostic discovery stopped early"),
        "complete discovery must not render a stop explanation"
    );
}
