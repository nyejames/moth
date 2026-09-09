use crate::compiler_frontend::compiler_messages::compiler_diagnostic::CompilerDiagnostic;
use crate::compiler_frontend::compiler_messages::compiler_errors::CompilerMessages;
use crate::compiler_frontend::compiler_messages::render::DiagnosticRenderContext;
use crate::compiler_frontend::compiler_messages::render::dev_server::render_compiler_messages_html;
use crate::compiler_frontend::compiler_messages::render::dev_server::render_diagnostics_html_with_context;
use crate::compiler_frontend::compiler_messages::render::primary_underline_length;
use crate::compiler_frontend::compiler_messages::render::terse::format_terse_diagnostic_with_context;
use crate::compiler_frontend::source::{
    ExtendedSpanBuilder, LocalSpan, SourceDatabase, SourceId, SourceSpan,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use std::fs;
use std::sync::Arc;

#[test]
fn rendered_source_frame_uses_retained_snapshot_after_disk_mutation() {
    let temporary_directory = tempfile::tempdir().expect("should create temporary directory");
    let source_path = temporary_directory.path().join("main.moth");
    fs::write(&source_path, "disk version\n").expect("should write initial source");

    let mut string_table = StringTable::new();
    let mut source_database = SourceDatabase::build(
        std::iter::once(source_path.as_path()),
        &source_path,
        None,
        &mut string_table,
    )
    .expect("source identity should build");
    let source_id = source_database
        .get_by_canonical_path(&source_path)
        .expect("source should be registered")
        .id;
    source_database
        .retain_text(source_id, "compiled snapshot\n".to_owned())
        .expect("compiled source should be retained");

    fs::write(&source_path, "newer disk version\n").expect("should mutate source on disk");

    let name = string_table.intern("undefined_thing");
    let mut span_builder = ExtendedSpanBuilder::new();
    let local_span =
        LocalSpan::exact(0, 6, &mut span_builder).expect("primary span should fit inline");
    let diagnostic =
        CompilerDiagnostic::unknown_value_name(name, Some(SourceSpan::new(source_id, local_span)));
    let mut messages = CompilerMessages::from_diagnostic(diagnostic, string_table);
    messages.set_source_database(Arc::new(source_database));

    let rendered = render_compiler_messages_html(&messages, temporary_directory.path());

    assert!(
        rendered.contains("compiled snapshot"),
        "the source frame should use the retained compiled snapshot: {rendered}"
    );
    assert!(
        !rendered.contains("newer disk version"),
        "the source frame must not reread the mutated source file: {rendered}"
    );
}

/// Build one database whose single source is `src/<relative_name>` with the given retained text.
fn retained_source_database(
    directory: &std::path::Path,
    relative_name: &str,
    text: &str,
    string_table: &mut StringTable,
) -> (SourceDatabase, SourceId) {
    let source_root = directory.join("src");
    fs::create_dir_all(&source_root).expect("should create source root");
    let source_path = source_root.join(relative_name);
    fs::write(&source_path, text).expect("should write source");

    let mut source_database = SourceDatabase::build(
        std::iter::once(source_path.as_path()),
        &source_root,
        None,
        string_table,
    )
    .expect("source identity should build");
    let source_id = source_database
        .get_by_canonical_path(&source_path)
        .expect("source should be registered")
        .id;
    source_database
        .retain_text(source_id, text.to_owned())
        .expect("compiled source should be retained");

    (source_database, source_id)
}
fn inline_span(source: SourceId, start: u32, length: u32) -> SourceSpan {
    let mut builder = ExtendedSpanBuilder::new();
    SourceSpan::new(
        source,
        LocalSpan::exact(start, length, &mut builder).expect("test span should fit inline"),
    )
}

#[test]
fn aggregated_diagnostics_keep_their_own_snapshot_across_prepend_and_append() {
    let alpha_directory = tempfile::tempdir().expect("should create alpha directory");
    let beta_directory = tempfile::tempdir().expect("should create beta directory");
    let mut string_table = StringTable::new();

    // Two independently rooted sources share a display path, so source identities—not paths—
    // distinguish their retained snapshots.
    let (alpha_database, alpha_source) = retained_source_database(
        alpha_directory.path(),
        "main.moth",
        "alpha_snapshot\n",
        &mut string_table,
    );
    let (beta_database, beta_source) = retained_source_database(
        beta_directory.path(),
        "main.moth",
        "beta_snapshot\n",
        &mut string_table,
    );

    let alpha_name = string_table.intern("alpha_value");
    let beta_name = string_table.intern("beta_value");
    let mut alpha_builder = ExtendedSpanBuilder::new();
    let alpha_span = LocalSpan::exact(0, 5, &mut alpha_builder).unwrap();
    let mut alpha_messages = CompilerMessages::from_diagnostic(
        CompilerDiagnostic::unknown_value_name(
            alpha_name,
            Some(SourceSpan::new(alpha_source, alpha_span)),
        ),
        string_table.clone(),
    );
    alpha_messages.set_source_database(Arc::new(alpha_database));
    let mut beta_builder = ExtendedSpanBuilder::new();
    let beta_span = LocalSpan::exact(0, 4, &mut beta_builder).unwrap();
    let mut beta_messages = CompilerMessages::from_diagnostic(
        CompilerDiagnostic::unknown_value_name(
            beta_name,
            Some(SourceSpan::new(beta_source, beta_span)),
        ),
        string_table,
    );
    beta_messages.set_source_database(Arc::new(beta_database));

    // Both aggregation directions shift the recorded ranges.
    alpha_messages.prepend_diagnostics_preserving_context(vec![
        CompilerDiagnostic::unreachable_match_arm(None),
    ]);
    alpha_messages.append_messages_preserving_context(beta_messages);

    let rendered = render_compiler_messages_html(&alpha_messages, alpha_directory.path());
    let alpha_frame = rendered
        .split("<article")
        .find(|article| article.contains("alpha_value"))
        .expect("the alpha diagnostic should render");
    let beta_frame = rendered
        .split("<article")
        .find(|article| article.contains("beta_value"))
        .expect("the beta diagnostic should render");

    assert!(
        alpha_frame.contains("alpha_snapshot") && !alpha_frame.contains("beta_snapshot"),
        "the alpha diagnostic must render its own snapshot: {alpha_frame}"
    );
    assert!(
        beta_frame.contains("beta_snapshot") && !beta_frame.contains("alpha_snapshot"),
        "the beta diagnostic must render its own snapshot: {beta_frame}"
    );
}
#[test]
fn rendered_source_lines_match_tokenizer_line_breaks() {
    let cases: &[(&str, &[&str])] = &[
        ("hello\r\nworld\r\n", &["hello", "world"]),
        ("a\rb", &["a", "b"]),
        ("trailing\r", &["trailing"]),
        ("mixed\r\n\rlone\n", &["mixed", "", "lone"]),
    ];

    for (text, expected_lines) in cases {
        let temporary_directory = tempfile::tempdir().expect("should create temporary directory");
        let mut string_table = StringTable::new();
        let (source_database, source_id) = retained_source_database(
            temporary_directory.path(),
            "main.moth",
            text,
            &mut string_table,
        );
        let line_name = string_table.intern("line");
        {
            let context = DiagnosticRenderContext::new(&string_table)
                .with_optional_source_database(Some(&source_database));

            let mut offset = 0u32;
            for (line_number, expected_line) in expected_lines.iter().enumerate() {
                let diagnostic = CompilerDiagnostic::unknown_value_name(
                    line_name,
                    Some(inline_span(source_id, offset, expected_line.len() as u32)),
                );
                let position = context
                    .primary_position(&diagnostic)
                    .expect("an exact source span should resolve to a position");
                assert_eq!(position.start.line, line_number as u32);
                assert_eq!(position.line, *expected_line);

                offset += expected_line.len() as u32;
                if let Some(next) = text.as_bytes().get(offset as usize) {
                    if *next == b'\r' {
                        offset += 1;
                        if text.as_bytes().get(offset as usize) == Some(&b'\n') {
                            offset += 1;
                        }
                    } else if *next == b'\n' {
                        offset += 1;
                    }
                }
            }
        }

        if text.ends_with('\r') || text.ends_with('\n') {
            let eof_name = string_table.intern("eof");
            let context = DiagnosticRenderContext::new(&string_table)
                .with_optional_source_database(Some(&source_database));
            let eof = CompilerDiagnostic::unknown_value_name(
                eof_name,
                Some(inline_span(source_id, text.len() as u32, 0)),
            );
            let position = context
                .primary_position(&eof)
                .expect("EOF after a terminator should resolve to the final authored line");
            assert_eq!(
                position.start.line,
                expected_lines.len() as u32 - 1,
                "{text:?}: EOF should stay on the final authored line"
            );
            assert_eq!(
                position.start.column,
                expected_lines
                    .last()
                    .map(|line| line.chars().count() as u32)
                    .unwrap_or(0),
                "{text:?}: EOF should resolve to the visible line end"
            );
            assert_eq!(
                position.line,
                expected_lines.last().copied().unwrap_or(""),
                "{text:?}: EOF should use the final authored line"
            );
        }
    }
}

#[test]
fn rendered_crlf_source_line_matches_str_lines() {
    let temporary_directory = tempfile::tempdir().expect("should create temporary directory");
    let mut string_table = StringTable::new();
    let text = "hello\r\nworld\r\n";
    let (source_database, source_id) = retained_source_database(
        temporary_directory.path(),
        "main.moth",
        text,
        &mut string_table,
    );

    let name = string_table.intern("undefined_thing");
    let diagnostic =
        CompilerDiagnostic::unknown_value_name(name, Some(inline_span(source_id, 0, 5)));
    let mut messages = CompilerMessages::from_diagnostic(diagnostic, string_table);
    messages.set_source_database(Arc::new(source_database));

    let rendered = render_compiler_messages_html(&messages, temporary_directory.path());
    assert!(
        rendered.contains(r#"<span class="source-line">hello</span>"#),
        "the source frame must render the CRLF line without a trailing CR: {rendered}"
    );
    assert!(
        !rendered.contains('\r'),
        "rendered HTML must not keep a CR from the line terminator: {rendered}"
    );
}

#[test]
fn renderers_resolve_multibyte_primary_span_to_scalar_columns() {
    let temporary_directory = tempfile::tempdir().expect("should create temporary directory");
    let mut string_table = StringTable::new();
    let (source_database, source_id) = retained_source_database(
        temporary_directory.path(),
        "main.moth",
        "éébad\n",
        &mut string_table,
    );

    let name = string_table.intern("bad");
    let diagnostic =
        CompilerDiagnostic::unknown_value_name(name, Some(inline_span(source_id, 4, 3)));

    let context = DiagnosticRenderContext::new(&string_table)
        .with_optional_source_database(Some(&source_database));
    let position = context
        .primary_position(&diagnostic)
        .expect("the primary span should resolve against the retained source");
    assert_eq!(position.start.line, 0);
    assert_eq!(position.start.column, 2);
    assert_eq!(position.end.line, 0);
    assert_eq!(position.end.column, 5);
    assert_eq!(primary_underline_length(&position, "éébad"), 3);

    let terse = format_terse_diagnostic_with_context(&diagnostic, context);
    assert!(
        terse.contains("|1:3|"),
        "terse output used stale columns: {terse}"
    );

    let rendered =
        render_diagnostics_html_with_context(&[diagnostic], temporary_directory.path(), context);
    assert!(
        rendered.contains(":1:3</a>"),
        "HTML output used stale columns: {rendered}"
    );
    assert!(
        rendered.contains(r#"class="source-caret">  ^^^</span>"#),
        "HTML caret must start at scalar column 2: {rendered}"
    );
}

#[test]
fn renderers_underline_half_open_reanchored_span_exactly() {
    let temporary_directory = tempfile::tempdir().expect("should create temporary directory");
    let mut string_table = StringTable::new();
    let (source_database, source_id) = retained_source_database(
        temporary_directory.path(),
        "main.moth",
        "xx café!\n",
        &mut string_table,
    );

    let name = string_table.intern("café");
    let diagnostic =
        CompilerDiagnostic::unknown_value_name(name, Some(inline_span(source_id, 3, 5)));

    let context = DiagnosticRenderContext::new(&string_table)
        .with_optional_source_database(Some(&source_database));
    let position = context
        .primary_position(&diagnostic)
        .expect("the primary span should resolve against the retained source");
    assert_eq!(position.start.column, 3);
    assert_eq!(position.end.column, 7);
    assert_eq!(primary_underline_length(&position, "xx café!"), 4);

    let rendered =
        render_diagnostics_html_with_context(&[diagnostic], temporary_directory.path(), context);
    assert!(
        rendered.contains(r#"class="source-caret">   ^^^^</span>"#),
        "half-open span must underline exactly four authored scalars: {rendered}"
    );
    assert!(
        !rendered.contains(r#"class="source-caret">   ^^^^^</span>"#),
        "the multibyte final scalar must not add an extra caret: {rendered}"
    );
}
#[test]
fn renderers_expand_a_preceding_tab_to_the_configured_stop_for_carets() {
    let temporary_directory = tempfile::tempdir().expect("should create temporary directory");
    let mut string_table = StringTable::new();
    let (source_database, source_id) = retained_source_database(
        temporary_directory.path(),
        "main.moth",
        "a\tb\n",
        &mut string_table,
    );

    let name = string_table.intern("b");
    let diagnostic =
        CompilerDiagnostic::unknown_value_name(name, Some(inline_span(source_id, 2, 1)));

    let context = DiagnosticRenderContext::new(&string_table)
        .with_optional_source_database(Some(&source_database));
    let position = context
        .primary_position(&diagnostic)
        .expect("the primary span should resolve against the retained source");
    assert_eq!(position.start.column, 2);
    assert_eq!(primary_underline_length(&position, "a\tb"), 1);

    let rendered =
        render_diagnostics_html_with_context(&[diagnostic], temporary_directory.path(), context);
    assert!(
        rendered.contains(r#"class="source-caret">        ^</span>"#),
        "caret must start after eight display cells: {rendered}"
    );
}

#[test]
fn renderers_count_a_wide_cjk_scalar_as_two_cells_for_carets() {
    let temporary_directory = tempfile::tempdir().expect("should create temporary directory");
    let mut string_table = StringTable::new();
    let (source_database, source_id) = retained_source_database(
        temporary_directory.path(),
        "main.moth",
        "あx\n",
        &mut string_table,
    );

    let name = string_table.intern("x");
    let diagnostic =
        CompilerDiagnostic::unknown_value_name(name, Some(inline_span(source_id, 3, 1)));

    let context = DiagnosticRenderContext::new(&string_table)
        .with_optional_source_database(Some(&source_database));
    let position = context
        .primary_position(&diagnostic)
        .expect("the primary span should resolve against the retained source");
    assert_eq!(position.start.column, 1);
    assert_eq!(primary_underline_length(&position, "あx"), 1);

    let rendered =
        render_diagnostics_html_with_context(&[diagnostic], temporary_directory.path(), context);
    assert!(
        rendered.contains(r#"class="source-caret">  ^</span>"#),
        "caret must start after two display cells: {rendered}"
    );
}
#[test]
fn renderers_count_a_combining_mark_as_zero_cells_for_carets() {
    let temporary_directory = tempfile::tempdir().expect("should create temporary directory");
    let mut string_table = StringTable::new();
    let (source_database, source_id) = retained_source_database(
        temporary_directory.path(),
        "main.moth",
        "éx\n",
        &mut string_table,
    );

    let name = string_table.intern("x");
    let diagnostic =
        CompilerDiagnostic::unknown_value_name(name, Some(inline_span(source_id, 3, 1)));

    let context = DiagnosticRenderContext::new(&string_table)
        .with_optional_source_database(Some(&source_database));
    let position = context
        .primary_position(&diagnostic)
        .expect("the primary span should resolve against the retained source");
    assert_eq!(position.start.column, 2);
    assert_eq!(primary_underline_length(&position, "éx"), 1);

    let rendered =
        render_diagnostics_html_with_context(&[diagnostic], temporary_directory.path(), context);
    assert!(
        rendered.contains(r#"class="source-caret"> ^</span>"#),
        "caret must start after one display cell: {rendered}"
    );
}

/// A compilation-root span names no physical source, so renderers omit a source frame even when
/// another retained snapshot is attached.
#[test]
fn compilation_root_primary_spans_render_without_a_physical_source_frame() {
    let temporary_directory = tempfile::tempdir().expect("should create temporary directory");
    let mut string_table = StringTable::new();
    let (source_database, _) = retained_source_database(
        temporary_directory.path(),
        "main.moth",
        "snapshot text\n",
        &mut string_table,
    );

    let name = string_table.intern("project_wide");
    let diagnostic = CompilerDiagnostic::unknown_value_name(
        name,
        Some(SourceSpan::new(
            SourceId::COMPILATION_ROOT,
            LocalSpan::source_start(),
        )),
    );

    let context = DiagnosticRenderContext::new(&string_table)
        .with_optional_source_database(Some(&source_database));

    assert!(
        context.primary_position(&diagnostic).is_none(),
        "the compilation root has no physical source position"
    );

    let terse = format_terse_diagnostic_with_context(&diagnostic, context);
    assert!(
        !terse.contains("main.moth"),
        "the root span must not resolve through the attached snapshot's path: {terse}"
    );

    let rendered =
        render_diagnostics_html_with_context(&[diagnostic], temporary_directory.path(), context);
    assert!(
        !rendered.contains("snapshot text"),
        "the compilation root must not render a physical source frame: {rendered}"
    );
    assert!(
        !rendered.contains("source-caret"),
        "the compilation root must not emit a caret row: {rendered}"
    );
}
