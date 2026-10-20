//! Rendering retained source and diagnostic identities after their producing owners end.

use crate::build_system::BuildProfile;
use crate::build_system::create_project_modules::{
    FrontendCompilationMode, compile_project_frontend_with_inputs,
};
use crate::builder_surface::{BuilderSurface, PackageOrigin};
use crate::compiler_frontend::build_config::BuildConfigInputSet;
use crate::compiler_frontend::compiler_messages::compiler_diagnostic::CompilerDiagnostic;
use crate::compiler_frontend::compiler_messages::compiler_errors::CompilerMessages;
use crate::compiler_frontend::compiler_messages::render::dev_server::render_compiler_messages_html;
use crate::compiler_frontend::compiler_messages::render::dev_server::render_diagnostics_html_with_context;
use crate::compiler_frontend::compiler_messages::render::terminal::{
    format_label_messages_with_context, format_label_messages_with_context_from_root,
    format_terminal_source_frame_for_test,
};
use crate::compiler_frontend::compiler_messages::render::terse::format_terse_diagnostic_with_context;
use crate::compiler_frontend::compiler_messages::render::{
    DiagnosticRenderContext, ResolvedDiagnosticLabel, diagnostic_type_name, primary_caret_padding,
    primary_underline_length, resolve_label_render_facts_from_root,
};
use crate::compiler_frontend::compiler_messages::{
    DiagnosticLabel, DiagnosticLabelMessage, DiagnosticLabelStyle, DiagnosticPayload,
    TypeMismatchContext,
};
use crate::compiler_frontend::source::{
    ExtendedSpanBuilder, FrozenIdentityContext, FrozenIdentityHandle, LocalSpan, SourceDatabase,
    SourceId, SourceSpan,
};
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::projects::settings::Config;
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

#[test]
fn materialised_diagnostic_renders_owned_requester_and_donor_snapshots_after_service_exit() {
    let _guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let project_directory = tempfile::tempdir().expect("the project directory should exist");
    let donor_directory = tempfile::tempdir().expect("the donor directory should exist");
    let source_root = project_directory.path().join("src");
    fs::create_dir_all(&source_root).expect("the requester source root should exist");
    let requester_path = source_root.join("@page.moth");
    let donor_path = donor_directory.path().join("@mod.moth");
    const REQUESTER: &str = "@pkg outer\nvalue I32 = outer(1)\n";
    const DONOR: &str = "inner type U |text String, result U| -> U:\n    return result\n;\n\
        export:\n    outer type T |value T| -> I32:\n        return inner(value, 1)\n    ;\n;\n";
    fs::write(&requester_path, REQUESTER).expect("the requester should be written");
    fs::write(&donor_path, DONOR).expect("the donor should be written");
    fs::write(
        project_directory.path().join("config.moth"),
        "project #= (name = \"snapshots\", entry_root = \"src\")\nhtml #= ()\n",
    )
    .expect("the config should be written");
    let requester_path =
        fs::canonicalize(requester_path).expect("the requester should canonicalise");
    let donor_path = fs::canonicalize(donor_path).expect("the donor should canonicalise");

    // Return only the report. The complete module transaction, provider preparation, caller
    // configuration and mutable source owners end before any source or type is rendered.
    let messages = {
        let mut config = Config::new(project_directory.path().to_path_buf());
        let mut strings = StringTable::new();
        let mut surface = BuilderSurface::with_mandatory_core();
        surface
            .source_packages
            .register_filesystem_root(
                "pkg",
                donor_directory.path().to_path_buf(),
                PackageOrigin::Builder,
            )
            .expect("test source root should register");
        let mut source_files = None;
        let frontend = compile_project_frontend_with_inputs(
            &mut config,
            BuildProfile::Dev,
            None,
            &StyleDirectiveRegistry::built_ins(),
            &mut surface,
            &mut strings,
            &mut source_files,
            &BuildConfigInputSet::new(),
            FrontendCompilationMode::Canonical,
        )
        .expect("the generated source diagnosis should return its complete transaction");
        assert!(frontend.has_diagnosed_or_blocked());
        frontend.into_render_messages_with_frozen_identity(&mut strings, source_files.take(), None)
    };
    fs::write(&requester_path, "requester disk replacement\n")
        .expect("the requester should be replaced");
    fs::write(&donor_path, "donor disk replacement\n").expect("the donor should be replaced");

    let diagnostics = messages.diagnostics().collect::<Vec<_>>();
    assert_eq!(diagnostics.len(), 1);
    let diagnostic = diagnostics[0];
    let DiagnosticPayload::TypeMismatch {
        expected,
        found,
        context: mismatch_context,
    } = &diagnostic.payload
    else {
        panic!("the concrete donor call must retain a typed mismatch");
    };
    assert_eq!(*mismatch_context, TypeMismatchContext::FunctionArgument);
    let context = messages.diagnostic_render_context(0);
    assert!(context.type_environment.is_some());
    assert_eq!(diagnostic_type_name(*expected, context), "String");
    assert_eq!(diagnostic_type_name(*found, context), "I32");
    let primary = context
        .primary_position(diagnostic)
        .expect("the retained requester span should resolve");
    assert_eq!(primary.host_path, Some(requester_path.as_path()));
    assert_eq!(primary.line, "value I32 = outer(1)");
    assert_eq!(primary.start.line, 1);
    assert_eq!(diagnostic.labels.len(), 3);
    assert_eq!(
        diagnostic.labels[0].message,
        Some(DiagnosticLabelMessage::GenericInstantiationBodySite)
    );
    assert_eq!(
        diagnostic.labels[1].message,
        Some(DiagnosticLabelMessage::GenericInstantiationDeclarationSite)
    );
    let Some(DiagnosticLabelMessage::GenericInstantiationSubstitutions { substitutions }) =
        &diagnostic.labels[2].message
    else {
        panic!("the final label should retain concrete substitution provenance");
    };
    assert_eq!(substitutions.len(), 1);
    assert_eq!(
        diagnostic_type_name(substitutions[0].concrete_type_id, context),
        "I32"
    );
    for (label, expected_line) in diagnostic.labels.iter().zip([
        "        return inner(value, 1)",
        "    outer type T |value T| -> I32:",
    ]) {
        let donor = context
            .label_position(label)
            .expect("the retained donor label should resolve in its own domain");
        assert_eq!(donor.host_path, Some(donor_path.as_path()));
        assert_eq!(donor.line, expected_line);
        assert_eq!(
            label
                .span
                .expect("the donor label retains its source")
                .source(),
            diagnostic
                .primary_span
                .expect("the requester retains its source")
                .source(),
            "colliding source IDs still resolve through their explicit provenance"
        );
    }
    let rendered = render_compiler_messages_html(&messages, project_directory.path());
    assert!(
        rendered.contains(r#"<span class="source-line">value I32 = outer(1)</span>"#),
        "the source frame should render the exact compiled requester line: {rendered}"
    );
    assert!(rendered.contains("String") && rendered.contains("I32"));
    assert!(!rendered.contains("disk replacement"));
    assert_eq!(rendered.matches("source-frame").count(), 1);
    let label_facts =
        resolve_label_render_facts_from_root(diagnostic, context, project_directory.path());
    assert_eq!(
        label_facts
            .iter()
            .map(|label| label.message.as_str())
            .collect::<Vec<_>>(),
        [
            "generic body operation failed here",
            "generic function declared here",
            "generic substitution: T = I32",
        ]
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
fn empty_retained_file_eof_position_preserves_path_coordinates_and_excerpt() {
    let temporary_directory = tempfile::tempdir().expect("should create temporary directory");
    let mut string_table = StringTable::new();
    let (source_database, source_id) = retained_source_database(
        temporary_directory.path(),
        "main.moth",
        "",
        &mut string_table,
    );
    let name = string_table.intern("eof");
    let diagnostic =
        CompilerDiagnostic::unknown_value_name(name, Some(inline_span(source_id, 0, 0)));

    {
        let context = DiagnosticRenderContext::new(&string_table)
            .with_optional_source_database(Some(&source_database));
        let position = context
            .primary_position(&diagnostic)
            .expect("empty-file EOF should resolve in the mutable source database");
        assert_eq!(position.source, source_id);
        assert_eq!(position.path, std::path::PathBuf::from("src/main.moth"));
        assert_eq!(position.start.line, 0);
        assert_eq!(position.start.column, 0);
        assert_eq!(position.end.line, 0);
        assert_eq!(position.end.column, 0);
        assert_eq!(position.line, "");

        let invalid_source = CompilerDiagnostic::unknown_value_name(
            name,
            Some(SourceSpan::new(
                SourceId::from_index(99),
                LocalSpan::source_start(),
            )),
        );
        assert!(
            context.primary_position(&invalid_source).is_none(),
            "an unknown source identity must remain unresolved"
        );
        let invalid_range =
            CompilerDiagnostic::unknown_value_name(name, Some(inline_span(source_id, 1, 0)));
        assert!(
            context.primary_position(&invalid_range).is_none(),
            "an out-of-range empty-file offset must remain unresolved"
        );
    }

    let frozen_identity = FrozenIdentityContext::from_parts(string_table, source_database);
    let frozen_context = DiagnosticRenderContext::new(frozen_identity.strings())
        .with_frozen_identity(&frozen_identity);
    let position = frozen_context
        .primary_position(&diagnostic)
        .expect("empty-file EOF should resolve in the frozen source database");
    assert_eq!(position.source, source_id);
    assert_eq!(position.path, std::path::PathBuf::from("src/main.moth"));
    assert_eq!(position.start.line, 0);
    assert_eq!(position.start.column, 0);
    assert_eq!(position.end.line, 0);
    assert_eq!(position.end.column, 0);
    assert_eq!(position.line, "");

    let invalid_source = CompilerDiagnostic::unknown_value_name(
        name,
        Some(SourceSpan::new(
            SourceId::from_index(99),
            LocalSpan::source_start(),
        )),
    );
    assert!(
        frozen_context.primary_position(&invalid_source).is_none(),
        "an unknown source identity must remain unresolved in the frozen database"
    );
    let invalid_range =
        CompilerDiagnostic::unknown_value_name(name, Some(inline_span(source_id, 1, 0)));
    assert!(
        frozen_context.primary_position(&invalid_range).is_none(),
        "an out-of-range empty-file offset must remain unresolved in the frozen database"
    );
}

#[test]
fn spanless_message_label_renders_without_a_source_position() {
    let mut string_table = StringTable::new();
    let label_text = string_table.intern("message-only label");
    let diagnostic = CompilerDiagnostic::unreachable_match_arm(None).with_labels(vec![
        DiagnosticLabel::secondary(None, Some(DiagnosticLabelMessage::RenderedText(label_text))),
        DiagnosticLabel::secondary(
            Some(SourceSpan::new(
                SourceId::from_index(99),
                LocalSpan::source_start(),
            )),
            Some(DiagnosticLabelMessage::PreviousDeclaration),
        ),
    ]);

    let rendered = format_label_messages_with_context(
        &diagnostic,
        DiagnosticRenderContext::new(&string_table),
    );

    assert_eq!(
        rendered,
        vec!["info: - message-only label"],
        "spanless labels render their message while unresolved spans remain omitted"
    );
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
fn terminal_source_and_caret_rows_share_gutter_and_tab_geometry() {
    let temporary_directory = tempfile::tempdir().expect("should create temporary directory");
    let mut string_table = StringTable::new();
    let target_lines = [0usize, 9, 99, 999];
    let mut source = String::new();
    let mut target_offsets = Vec::new();
    for line in 0..1000 {
        if target_lines.contains(&line) {
            target_offsets.push(source.len());
            source.push_str("\tb\n");
        } else {
            source.push_str("noop\n");
        }
    }

    let (source_database, source_id) = retained_source_database(
        temporary_directory.path(),
        "main.moth",
        &source,
        &mut string_table,
    );
    let name = string_table.intern("b");
    let context = DiagnosticRenderContext::new(&string_table)
        .with_optional_source_database(Some(&source_database));

    for (line, offset) in target_lines.into_iter().zip(target_offsets) {
        let diagnostic = CompilerDiagnostic::unknown_value_name(
            name,
            Some(inline_span(source_id, (offset + 1) as u32, 1)),
        );
        let frame = format_terminal_source_frame_for_test(&diagnostic, context)
            .expect("the terminal source frame should resolve");
        let rows = frame.lines().collect::<Vec<_>>();
        assert_eq!(
            rows.len(),
            3,
            "line {line} should emit marker, source and caret rows"
        );
        let gutter_width = (line + 1).to_string().len().max(3);
        let expected_source_label = format!("{:>width$} | ", line + 1, width = gutter_width);
        assert!(
            rows[1].starts_with(&expected_source_label),
            "line {} should use the expected source gutter width: {frame}",
            line + 1
        );
        let source_b = rows[1]
            .find('b')
            .expect("the expanded source row should retain the diagnosed scalar");
        let caret = rows[2]
            .find('^')
            .expect("the caret row should retain the diagnosed underline");
        assert_eq!(
            source_b,
            caret,
            "line {} should align the caret with the expanded leading tab",
            line + 1
        );
        assert_eq!(
            rows[2].find('^').expect("caret should be present"),
            rows[1]
                .find(" | ")
                .expect("source row should have a gutter")
                + 3
                + 8,
            "line {} should expand a leading tab from the source-column origin",
            line + 1
        );
    }
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

#[test]
fn renderers_measure_a_zwj_emoji_sequence_as_two_display_cells_for_carets() {
    let temporary_directory = tempfile::tempdir().expect("temporary directory should exist");
    let mut string_table = StringTable::new();
    let (source_database, source_id) = retained_source_database(
        temporary_directory.path(),
        "main.moth",
        "👩‍💻x\n",
        &mut string_table,
    );

    let name = string_table.intern("x");
    let diagnostic =
        CompilerDiagnostic::unknown_value_name(name, Some(inline_span(source_id, 11, 1)));
    let context = DiagnosticRenderContext::new(&string_table)
        .with_optional_source_database(Some(&source_database));
    let position = context
        .primary_position(&diagnostic)
        .expect("the primary span should resolve against the retained source");

    assert_eq!(position.start.column, 3);
    assert_eq!(primary_caret_padding(&position, "👩‍💻x"), 2);
    assert_eq!(primary_underline_length(&position, "👩‍💻x"), 1);
}

#[test]
fn renderers_measure_a_variation_selector_sequence_as_two_display_cells_for_carets() {
    let temporary_directory = tempfile::tempdir().expect("temporary directory should exist");
    let mut string_table = StringTable::new();
    let (source_database, source_id) = retained_source_database(
        temporary_directory.path(),
        "main.moth",
        "❤️x\n",
        &mut string_table,
    );

    let name = string_table.intern("x");
    let diagnostic =
        CompilerDiagnostic::unknown_value_name(name, Some(inline_span(source_id, 6, 1)));
    let context = DiagnosticRenderContext::new(&string_table)
        .with_optional_source_database(Some(&source_database));
    let position = context
        .primary_position(&diagnostic)
        .expect("the primary span should resolve against the retained source");

    assert_eq!(position.start.column, 2);
    assert_eq!(primary_caret_padding(&position, "❤️x"), 2);
    assert_eq!(primary_underline_length(&position, "❤️x"), 1);
}

#[test]
fn renderers_include_related_donor_path_and_message_in_terminal_and_html() {
    let primary_directory = tempfile::tempdir().expect("primary directory should exist");
    let donor_directory = tempfile::tempdir().expect("donor directory should exist");
    let mut primary_strings = StringTable::new();
    let (primary_database, primary_source) = retained_source_database(
        primary_directory.path(),
        "caller.moth",
        "missing\n",
        &mut primary_strings,
    );
    let missing_name = primary_strings.intern("missing");
    let primary_identity = Arc::new(FrozenIdentityContext::from_parts(
        primary_strings,
        primary_database,
    ));

    let mut donor_strings = StringTable::new();
    let (donor_database, donor_source) = retained_source_database(
        donor_directory.path(),
        "donor.moth",
        "generic body\n",
        &mut donor_strings,
    );
    let donor_identity = Arc::new(FrozenIdentityContext::from_parts(
        donor_strings,
        donor_database,
    ));
    let donor_handle = FrozenIdentityHandle::new();
    donor_handle
        .install(Arc::clone(&donor_identity))
        .expect("donor identity should install exactly once");

    let diagnostic = CompilerDiagnostic::unknown_value_name(
        missing_name,
        Some(inline_span(primary_source, 0, 7)),
    )
    .with_labels(vec![DiagnosticLabel::secondary_with_frozen_identity(
        Some(inline_span(donor_source, 0, 7)),
        Some(DiagnosticLabelMessage::GenericInstantiationBodySite),
        donor_handle,
    )]);
    let context = DiagnosticRenderContext::new(primary_identity.strings())
        .with_frozen_identity(&primary_identity);
    let label_facts =
        resolve_label_render_facts_from_root(&diagnostic, context, primary_directory.path());
    assert_eq!(
        label_facts,
        vec![ResolvedDiagnosticLabel {
            style: DiagnosticLabelStyle::Secondary,
            path: Some("src/donor.moth".to_owned()),
            line: Some(1),
            column: Some(1),
            message: "generic body operation failed here".to_owned(),
        }],
        "shared label resolution should expose facts without terminal presentation text",
    );

    let terminal_labels = format_label_messages_with_context_from_root(
        &diagnostic,
        context,
        primary_directory.path(),
    );
    assert_eq!(
        terminal_labels,
        vec!["info: src/donor.moth:1:1 - generic body operation failed here"],
        "terminal related-site output should retain the resolved donor path and label message",
    );

    let html = render_diagnostics_html_with_context(
        std::slice::from_ref(&diagnostic),
        primary_directory.path(),
        context,
    );
    assert!(
        html.contains(
            r#"class="diagnostic-label">info: src/donor.moth:1:1 - generic body operation failed here</p>"#,
        ),
        "HTML should render the related-site path and message without dropping the label: {html}",
    );
    assert_eq!(
        html.matches("source-frame").count(),
        1,
        "secondary labels should not duplicate the primary source geometry: {html}",
    );
}

/// A span-only secondary label is a valid structured fact (label message code 0). The shared
/// resolver and both renderers must surface its own resolved location, and neither may append an
/// empty message separator.
#[test]
fn span_only_secondary_label_surfaces_its_own_location() {
    let temporary_directory = tempfile::tempdir().expect("should create temporary directory");
    let mut string_table = StringTable::new();
    let (source_database, source_id) = retained_source_database(
        temporary_directory.path(),
        "main.moth",
        "base\nvalue\n",
        &mut string_table,
    );

    let name = string_table.intern("value");
    let diagnostic =
        CompilerDiagnostic::unknown_value_name(name, Some(inline_span(source_id, 5, 5)))
            .with_labels(vec![DiagnosticLabel::secondary(
                Some(inline_span(source_id, 0, 4)),
                None,
            )]);
    let context = DiagnosticRenderContext::new(&string_table)
        .with_optional_source_database(Some(&source_database));

    let label_facts =
        resolve_label_render_facts_from_root(&diagnostic, context, temporary_directory.path());
    assert_eq!(
        label_facts,
        vec![ResolvedDiagnosticLabel {
            style: DiagnosticLabelStyle::Secondary,
            path: Some("src/main.moth".to_owned()),
            line: Some(1),
            column: Some(1),
            message: String::new(),
        }],
        "a span-only label must keep its resolved location with an empty message",
    );

    let terminal_labels = format_label_messages_with_context_from_root(
        &diagnostic,
        context,
        temporary_directory.path(),
    );
    assert_eq!(
        terminal_labels,
        vec!["info: 1:1"],
        "the terminal label line must show the distinct location without an empty message suffix",
    );

    let html = render_diagnostics_html_with_context(
        std::slice::from_ref(&diagnostic),
        temporary_directory.path(),
        context,
    );
    assert!(
        html.contains("src/main.moth:2:1"),
        "the primary location must still render: {html}",
    );
    assert_eq!(
        html.matches(r#"class="diagnostic-label""#).count(),
        1,
        "the span-only label must render exactly once: {html}",
    );
    assert!(
        html.contains(r#"class="diagnostic-label">info: 1:1</p>"#),
        "HTML must show the span-only label location without an empty message suffix: {html}",
    );
}

#[test]
fn html_escapes_dynamic_relative_source_paths() {
    let temporary_directory = tempfile::tempdir().expect("temporary directory should exist");
    let mut string_table = StringTable::new();
    let (source_database, source_id) = retained_source_database(
        temporary_directory.path(),
        "bad&.moth",
        "value\n",
        &mut string_table,
    );
    let name = string_table.intern("value");
    let diagnostic =
        CompilerDiagnostic::unknown_value_name(name, Some(inline_span(source_id, 0, 5)));
    let context = DiagnosticRenderContext::new(&string_table)
        .with_optional_source_database(Some(&source_database));

    let html = render_diagnostics_html_with_context(
        std::slice::from_ref(&diagnostic),
        temporary_directory.path(),
        context,
    );
    assert!(
        html.contains("src/bad&amp;.moth:1:1"),
        "relative source paths must be HTML escaped in location text: {html}",
    );
    assert!(
        !html.contains("src/bad&.moth:1:1"),
        "raw relative source path markup must never be emitted: {html}",
    );
}

#[test]
fn html_escapes_related_label_messages_without_terminal_presentation_text() {
    let temporary_directory = tempfile::tempdir().expect("temporary directory should exist");
    let mut string_table = StringTable::new();
    let (source_database, source_id) = retained_source_database(
        temporary_directory.path(),
        "main.moth",
        "value\n",
        &mut string_table,
    );
    let name = string_table.intern("value");
    let label_text = string_table.intern("<label> &");
    let diagnostic =
        CompilerDiagnostic::unknown_value_name(name, Some(inline_span(source_id, 0, 5)))
            .with_labels(vec![DiagnosticLabel::secondary(
                None,
                Some(DiagnosticLabelMessage::RenderedText(label_text)),
            )]);
    let context = DiagnosticRenderContext::new(&string_table)
        .with_optional_source_database(Some(&source_database));

    let html = render_diagnostics_html_with_context(
        std::slice::from_ref(&diagnostic),
        temporary_directory.path(),
        context,
    );

    assert!(
        html.contains(r#"class="diagnostic-label">info: - &lt;label&gt; &amp;</p>"#),
        "label message text must be escaped independently in HTML: {html}",
    );
    assert!(
        !html.contains(r#"class="diagnostic-label">info: - <label> &</p>"#),
        "raw label message markup must never be emitted: {html}",
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
