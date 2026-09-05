use super::{SourceDatabase, SourceId, SourceProvenance};
use crate::builder_surface::SourceFileKindRegistry;
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages, ErrorType};
use crate::compiler_frontend::compiler_messages::compiler_diagnostic::CompilerDiagnostic;
use crate::compiler_frontend::compiler_messages::render::dev_server::render_compiler_messages_html;
use crate::compiler_frontend::compiler_messages::source_location::{CharPosition, SourceLocation};
use crate::compiler_frontend::paths::path_resolution::ProjectPathResolver;
use crate::compiler_frontend::source_packages::root_file::PreparedSourcePackageRoots;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use std::fs;
use std::mem::{align_of, size_of};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[test]
fn source_id_uses_the_non_zero_option_niche() {
    assert_eq!(size_of::<SourceId>(), 4);
    assert_eq!(align_of::<SourceId>(), 4);
    assert_eq!(size_of::<Option<SourceId>>(), 4);
}

#[test]
fn source_database_reserves_compilation_root_before_physical_sources() {
    let physical_path = PathBuf::from("/project/main.moth");
    let mut string_table = StringTable::new();
    let database = SourceDatabase::build(
        std::iter::once(&physical_path),
        &physical_path,
        None,
        &mut string_table,
    )
    .expect("source identities should build");

    let root_id = SourceId::from_index(0);
    assert!(
        database.get(root_id).is_none(),
        "the reserved compilation root is not a physical source record"
    );

    let physical_records = database.iter().collect::<Vec<_>>();
    assert_eq!(physical_records.len(), 1);
    let physical = physical_records[0];
    assert_eq!(
        physical.id,
        SourceId::from_index(1),
        "physical sources begin after the reserved root"
    );
    assert_eq!(physical.provenance, SourceProvenance::AuthoredPhysical);
    assert_eq!(
        database.get(physical.id).map(|record| record.id),
        Some(physical.id),
        "a physical identity must address its own record"
    );
}

#[test]
fn appended_sources_also_begin_after_the_reserved_root() {
    // `empty` backs the single-file and template temporary databases, which register by insertion
    // rather than from an ordered inventory. Their first source must still not claim the root ID.
    let mut database = SourceDatabase::empty();
    let mut string_table = StringTable::new();
    let canonical_path = PathBuf::from("/project/main.moth");

    let id = database
        .insert(
            canonical_path.clone(),
            &canonical_path,
            None,
            &mut string_table,
        )
        .expect("an appended source should register");

    assert_eq!(id, SourceId::from_index(1));
    assert_eq!(
        database
            .get_by_canonical_path(&canonical_path)
            .map(|record| record.id),
        Some(id)
    );
}

#[test]
fn source_database_build_orders_records_by_portable_logical_path() {
    let input_paths = [
        PathBuf::from("/project/zeta.moth"),
        PathBuf::from("/project/alpha.moth"),
        PathBuf::from("/project/middle.moth"),
    ];
    let entry_path = Path::new("/project/alpha.moth");
    let mut first_strings = StringTable::new();
    let first = SourceDatabase::build(input_paths.iter(), entry_path, None, &mut first_strings)
        .expect("source identities should build");

    let reversed_paths = input_paths.iter().rev().collect::<Vec<_>>();
    let mut second_strings = StringTable::new();
    let second = SourceDatabase::build(reversed_paths, entry_path, None, &mut second_strings)
        .expect("source identities should build in reverse input order");

    let first_logical_paths = first
        .iter()
        .map(|record| record.logical_path.to_path_buf(&first_strings))
        .collect::<Vec<_>>();
    let second_logical_paths = second
        .iter()
        .map(|record| record.logical_path.to_path_buf(&second_strings))
        .collect::<Vec<_>>();
    assert_eq!(first_logical_paths, second_logical_paths);
    assert_eq!(
        first_logical_paths,
        vec![
            PathBuf::from("alpha.moth"),
            PathBuf::from("middle.moth"),
            PathBuf::from("zeta.moth"),
        ]
    );

    for (index, record) in first.iter().enumerate() {
        assert_eq!(record.id, SourceId::from_index(index + 1));
    }
}

#[test]
fn source_ids_are_distinct_and_same_module_sources_follow_portable_order() {
    let paths = [
        PathBuf::from("/project/alpha/z.moth"),
        PathBuf::from("/project/alpha/a.moth"),
        PathBuf::from("/project/beta/root.moth"),
    ];
    let mut string_table = StringTable::new();
    let database = SourceDatabase::build(
        paths.iter(),
        Path::new("/project/alpha/a.moth"),
        None,
        &mut string_table,
    )
    .expect("source identities should build");

    let alpha_a = database
        .get_by_canonical_path(&paths[1])
        .expect("alpha/a source should be registered")
        .id;
    let alpha_z = database
        .get_by_canonical_path(&paths[0])
        .expect("alpha/z source should be registered")
        .id;
    let beta_root = database
        .get_by_canonical_path(&paths[2])
        .expect("beta/root source should be registered")
        .id;

    assert_ne!(alpha_a, alpha_z);
    assert_ne!(alpha_a, beta_root);
    assert_ne!(alpha_z, beta_root);

    let alpha_logical_paths = database
        .iter()
        .filter(|record| {
            record
                .canonical_os_path
                .as_deref()
                .is_some_and(|path| path.starts_with("/project/alpha"))
        })
        .map(|record| record.logical_path.to_portable_string(&string_table))
        .collect::<Vec<_>>();
    assert_eq!(alpha_logical_paths, vec!["a.moth", "z.moth"]);
}

#[test]
fn one_canonical_source_reachable_from_two_modules_has_one_record() {
    let module_a_source = PathBuf::from("/project/alpha.moth");
    let module_b_source = PathBuf::from("/project/beta.moth");
    let shared_source = PathBuf::from("/project/shared.moth");
    let boundary_paths = [
        module_a_source.clone(),
        module_b_source.clone(),
        shared_source.clone(),
    ];
    let mut string_table = StringTable::new();
    let mut database = SourceDatabase::build(
        boundary_paths.iter(),
        &module_a_source,
        None,
        &mut string_table,
    )
    .expect("boundary source identities should build");

    let module_a_view = database
        .insert(
            shared_source.clone(),
            &module_a_source,
            None,
            &mut string_table,
        )
        .expect("module A should resolve the shared source");
    let module_b_view = database
        .insert(
            shared_source.clone(),
            &module_b_source,
            None,
            &mut string_table,
        )
        .expect("module B should resolve the shared source");
    let shared_record_count = database
        .iter()
        .filter(|record| record.canonical_os_path.as_deref() == Some(shared_source.as_path()))
        .count();

    assert_eq!(module_a_view, module_b_view);
    assert_eq!(shared_record_count, 1);
}

#[test]
fn conflicting_logical_identity_for_canonical_source_is_rejected() {
    let canonical_path = PathBuf::from("/project/src/shared.moth");
    let first_entry_path = PathBuf::from("/project/entry.moth");
    let conflicting_entry_path = PathBuf::from("/project/src/entry.moth");
    let mut string_table = StringTable::new();
    let mut database = SourceDatabase::empty();

    database
        .insert(
            canonical_path.clone(),
            &first_entry_path,
            None,
            &mut string_table,
        )
        .expect("the first source registration should succeed");
    let error = database
        .insert(
            canonical_path,
            &conflicting_entry_path,
            None,
            &mut string_table,
        )
        .expect_err("a canonical source cannot change logical identity");

    // Both spellings are suffixes of the canonical path, so a substring test cannot tell them
    // apart; the occurrence count proves the message names the stored and requested identities
    // as well as the file it is rejecting.
    assert!(
        error.msg.contains("/project/src/shared.moth")
            && error.msg.matches("shared.moth").count() >= 3,
        "the conflict should name the canonical path and both logical spellings: {}",
        error.msg
    );
    assert_eq!(error.error_type, ErrorType::Compiler);
    assert_eq!(
        database
            .get_by_canonical_path(&PathBuf::from("/project/src/shared.moth"))
            .map(|record| record.id),
        Some(SourceId::from_index(1)),
        "the rejected registration must leave the first identity in place"
    );
}

#[test]
fn source_database_distinguishes_empty_text_from_unloaded_and_failed_slots() {
    let empty_path = PathBuf::from("/project/empty.moth");
    let failed_path = PathBuf::from("/project/failed.moth");
    let mut string_table = StringTable::new();
    let mut database = SourceDatabase::build(
        [&empty_path, &failed_path],
        &empty_path,
        None,
        &mut string_table,
    )
    .expect("source identities should build");
    let empty_id = database
        .get_by_canonical_path(&empty_path)
        .expect("empty source should be registered")
        .id;
    let failed_id = database
        .get_by_canonical_path(&failed_path)
        .expect("failed source should be registered")
        .id;

    assert!(database.retained_text(SourceId::from_index(0)).is_none());
    assert!(database.retained_text(empty_id).is_none());
    assert!(database.source_load_error(empty_id).is_none());

    database
        .retain_text(empty_id, String::new())
        .expect("empty source text should be retained");
    assert_eq!(database.retained_text(empty_id), Some(""));
    assert!(database.source_load_error(empty_id).is_none());

    let load_error =
        CompilerError::file_error(&failed_path, "source read failed", &mut string_table);
    database
        .record_source_load_error(failed_id, load_error)
        .expect("source load error should be retained");
    assert!(database.retained_text(failed_id).is_none());
    assert!(database.source_load_error(failed_id).is_some());

    // Every further write is refused and leaves the recorded state untouched, so a stage can
    // never observe a snapshot the compiler did not compile.
    let repeated_snapshot_error = database
        .retain_text(empty_id, "second snapshot".to_owned())
        .expect_err("a source snapshot can only be retained once");
    assert_eq!(repeated_snapshot_error.error_type, ErrorType::Compiler);
    assert_eq!(database.retained_text(empty_id), Some(""));

    database
        .retain_text(failed_id, "unreadable snapshot".to_owned())
        .expect_err("a failed source cannot later receive a snapshot");
    assert!(database.retained_text(failed_id).is_none());
    assert!(database.source_load_error(failed_id).is_some());

    let repeated_failure =
        CompilerError::file_error(&failed_path, "source read failed again", &mut string_table);
    database
        .record_source_load_error(failed_id, repeated_failure)
        .expect_err("an unreadable source cannot record a second failure");
    assert!(database.source_load_error(failed_id).is_some());

    // The reserved compilation root is addressable but never writable.
    database
        .retain_text(SourceId::from_index(0), "root snapshot".to_owned())
        .expect_err("the compilation root cannot retain source text");

    let load_after_snapshot = CompilerError::file_error(
        &empty_path,
        "source read failed after snapshot",
        &mut string_table,
    );
    database
        .record_source_load_error(empty_id, load_after_snapshot)
        .expect_err("a loaded source cannot receive a second load status");
    assert_eq!(database.retained_text(empty_id), Some(""));
    assert!(database.source_load_error(empty_id).is_none());
}

#[test]
fn source_database_resolves_retained_text_by_logical_path() {
    let source_path = PathBuf::from("/project/main.moth");
    let mut string_table = StringTable::new();
    let mut database = SourceDatabase::build(
        std::iter::once(&source_path),
        &source_path,
        None,
        &mut string_table,
    )
    .expect("source identities should build");
    let source_id = database
        .get_by_canonical_path(&source_path)
        .expect("source should be registered")
        .id;
    let logical_path = database
        .get(source_id)
        .expect("source record should be addressable")
        .logical_path
        .clone();

    database
        .retain_text(source_id, "compiled snapshot\n".to_owned())
        .expect("source text should be retained");

    assert_eq!(
        database.retained_text_for_logical_path(&logical_path),
        Some("compiled snapshot\n"),
    );
}
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
    let root_logical_path = database
        .get(root_id)
        .expect("root config record should be addressable")
        .logical_path
        .clone();
    let entry_logical_path = database
        .get(entry_id)
        .expect("entry config record should be addressable")
        .logical_path
        .clone();
    assert_eq!(
        root_logical_path, entry_logical_path,
        "the project config and entry-root source should share config.moth's logical path"
    );

    database
        .retain_text(root_id, "root config snapshot\n".to_owned())
        .expect("root config snapshot should be retained");

    // Ambiguity is a property of the identities, not of how many happen to be loaded: one loaded
    // candidate among colliding records must not become the answer by default.
    assert_eq!(
        database.retained_text_for_logical_path(&root_logical_path),
        None
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
