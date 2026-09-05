use crate::compiler_frontend::compiler_messages::compiler_diagnostic::CompilerDiagnostic;
use crate::compiler_frontend::compiler_messages::compiler_errors::CompilerMessages;
use crate::compiler_frontend::compiler_messages::render::dev_server::render_compiler_messages_html;
use crate::compiler_frontend::compiler_messages::source_location::{CharPosition, SourceLocation};
use crate::compiler_frontend::source::SourceDatabase;
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
    let logical_path = source_database
        .get(source_id)
        .expect("source record should be addressable")
        .logical_path
        .clone();

    fs::write(&source_path, "newer disk version\n").expect("should mutate source on disk");

    let name = string_table.intern("undefined_thing");
    let location = SourceLocation::new(
        logical_path,
        CharPosition {
            line_number: 0,
            char_column: 0,
        },
        CharPosition {
            line_number: 0,
            char_column: 6,
        },
    );
    let diagnostic = CompilerDiagnostic::unknown_value_name(name, location);
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
///
/// Both callers share the `src` root name so their sources intern to one logical path.
fn retained_source_database(
    directory: &std::path::Path,
    relative_name: &str,
    text: &str,
    string_table: &mut StringTable,
) -> (SourceDatabase, SourceLocation) {
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
    let logical_path = source_database
        .get(source_id)
        .expect("source record should be addressable")
        .logical_path
        .clone();

    let location = SourceLocation::new(
        logical_path,
        CharPosition {
            line_number: 0,
            char_column: 0,
        },
        CharPosition {
            line_number: 0,
            char_column: 5,
        },
    );
    (source_database, location)
}

#[test]
fn aggregated_diagnostics_keep_their_own_snapshot_across_prepend_and_append() {
    let alpha_directory = tempfile::tempdir().expect("should create alpha directory");
    let beta_directory = tempfile::tempdir().expect("should create beta directory");
    let mut string_table = StringTable::new();

    // Two independently rooted sources interning to the same logical path, so only the recorded
    // association distinguishes them.
    let (alpha_database, alpha_location) = retained_source_database(
        alpha_directory.path(),
        "main.moth",
        "alpha_snapshot\n",
        &mut string_table,
    );
    let (beta_database, beta_location) = retained_source_database(
        beta_directory.path(),
        "main.moth",
        "beta_snapshot\n",
        &mut string_table,
    );
    assert_eq!(
        alpha_location.scope, beta_location.scope,
        "the two sources must share one logical path for this regression to be meaningful"
    );

    let alpha_name = string_table.intern("alpha_value");
    let beta_name = string_table.intern("beta_value");
    let mut alpha_messages = CompilerMessages::from_diagnostic(
        CompilerDiagnostic::unknown_value_name(alpha_name, alpha_location),
        string_table.clone(),
    );
    alpha_messages.set_source_database(Arc::new(alpha_database));
    let mut beta_messages = CompilerMessages::from_diagnostic(
        CompilerDiagnostic::unknown_value_name(beta_name, beta_location),
        string_table,
    );
    beta_messages.set_source_database(Arc::new(beta_database));

    // Both aggregation directions shift the recorded ranges.
    alpha_messages.prepend_diagnostics_preserving_context(vec![
        CompilerDiagnostic::unreachable_match_arm(SourceLocation::default()),
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
