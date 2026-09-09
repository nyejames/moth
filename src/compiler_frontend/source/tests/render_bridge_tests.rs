use super::SourceDatabase;

use crate::builder_surface::SourceFileKindRegistry;
use crate::compiler_frontend::compiler_errors::CompilerMessages;
use crate::compiler_frontend::compiler_messages::compiler_diagnostic::CompilerDiagnostic;
use crate::compiler_frontend::compiler_messages::render::dev_server::render_compiler_messages_html;
use crate::compiler_frontend::compiler_messages::source_location::{CharPosition, SourceLocation};
use crate::compiler_frontend::paths::path_resolution::ProjectPathResolver;
use crate::compiler_frontend::source_packages::root_file::PreparedSourcePackageRoots;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use std::fs;
use std::sync::Arc;

#[test]
fn ambiguous_project_config_logical_path_omits_source_frame_instead_of_guessing() {
    let temporary_directory = tempfile::tempdir().expect("should create temporary directory");
    let project_root =
        fs::canonicalize(temporary_directory.path()).expect("project root should canonicalize");
    let entry_root = project_root.join("src");
    fs::create_dir_all(&entry_root).expect("should create project entry root");

    // Config registration roots the file at its own directory, while ordinary project sources
    // strip the configured entry root. With an entry root of `src`, both records intentionally
    // intern to `config.moth` in this one database.
    let root_config = project_root.join("config.moth");
    let entry_config = entry_root.join("config.moth");
    fs::write(&root_config, "root config snapshot\n").expect("should write root config");
    fs::write(&entry_config, "entry source snapshot\n").expect("should write entry source");
    let root_config = fs::canonicalize(root_config).expect("root config should canonicalize");
    let entry_config = fs::canonicalize(entry_config).expect("entry config should canonicalize");

    let source_file_kinds = SourceFileKindRegistry::default();
    let resolver = ProjectPathResolver::new(
        project_root,
        entry_root,
        PreparedSourcePackageRoots::empty(),
        &source_file_kinds,
    )
    .expect("project path resolver should build");
    let mut string_table = StringTable::new();
    let mut database = SourceDatabase::build(
        [root_config.as_path(), entry_config.as_path()],
        &entry_config,
        Some(&resolver),
        &mut string_table,
    )
    .expect("source identities should build");
    let root_id = database
        .get_by_canonical_path(&root_config)
        .expect("root config should be registered")
        .id;
    let entry_id = database
        .get_by_canonical_path(&entry_config)
        .expect("entry config should be registered")
        .id;
    let root_logical_path = database.legacy_logical_path(root_id);
    let entry_logical_path = database.legacy_logical_path(entry_id);
    assert_eq!(
        root_logical_path, entry_logical_path,
        "the project config and entry-root source should share config.moth's logical path"
    );

    database
        .retain_text(root_id, "root config snapshot\n".to_owned())
        .expect("root config snapshot should be retained");

    // Ambiguity is a property of the identities, not of how many happen to be loaded: one loaded
    // candidate among colliding records must not become the answer by default.
    assert!(
        database
            .unique_record_for_logical_path(&root_logical_path)
            .is_none()
    );
    database
        .retain_text(entry_id, "entry source snapshot\n".to_owned())
        .expect("entry source snapshot should be retained");
    let name = string_table.intern("undefined_thing");
    let location = SourceLocation::new(
        entry_logical_path,
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
    messages.set_source_database(Arc::new(database));

    let rendered = render_compiler_messages_html(&messages, temporary_directory.path());
    assert!(
        !rendered.contains("root config snapshot"),
        "an ambiguous logical path must never render the root config text: {rendered}"
    );
    assert!(
        !rendered.contains("entry source snapshot"),
        "an ambiguous logical path must omit the frame rather than guess: {rendered}"
    );
}
