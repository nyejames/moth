use super::{ExtendedSpanBuilder, LocalSpan, SourceDatabase, SourceSpan};
use crate::compiler_frontend::paths::module_roots::ModuleRootTable;

use crate::builder_surface::SourceFileKindRegistry;
use crate::compiler_frontend::compiler_errors::CompilerMessages;
use crate::compiler_frontend::compiler_messages::compiler_diagnostic::CompilerDiagnostic;
use crate::compiler_frontend::compiler_messages::render::dev_server::render_compiler_messages_html;
use crate::compiler_frontend::paths::path_resolution::ProjectPathResolver;
use crate::compiler_frontend::source_packages::root_file::PreparedSourcePackageRoots;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use std::fs;
use std::sync::Arc;

#[test]
fn source_span_selects_exact_source_when_logical_paths_are_ambiguous() {
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
    let resolver = ProjectPathResolver::new_with_module_roots(
        project_root,
        entry_root,
        PreparedSourcePackageRoots::default(),
        &source_file_kinds,
        ModuleRootTable::empty(),
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

    // Logical-path lookup remains ambiguous even after both snapshots load, so it must not choose
    // another source's text. The exact source span below carries the intended source identity.
    assert!(
        database
            .unique_record_for_logical_path(&root_logical_path)
            .is_none()
    );
    database
        .retain_text(entry_id, "entry source snapshot\n".to_owned())
        .expect("entry source snapshot should be retained");
    let name = string_table.intern("undefined_thing");
    let mut span_builder = ExtendedSpanBuilder::new();
    let local_span =
        LocalSpan::exact(0, 6, &mut span_builder).expect("primary span should fit inline");
    let diagnostic =
        CompilerDiagnostic::unknown_value_name(name, Some(SourceSpan::new(entry_id, local_span)));
    let mut messages = CompilerMessages::from_diagnostic(diagnostic, string_table);
    messages.set_source_database(Arc::new(database));

    let rendered = render_compiler_messages_html(&messages, temporary_directory.path());
    assert!(
        !rendered.contains("root config snapshot"),
        "the exact source span must not render the colliding root source: {rendered}"
    );
    assert!(
        rendered.contains("entry source snapshot"),
        "the exact source span must render its retained source: {rendered}"
    );
}
