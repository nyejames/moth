use super::{
    SourceDatabase, SourceId, SourceKind, SourceProvenance, SourceRegistrationIndex,
    line_index::LinePosition, record::ensure_source_snapshot_fits,
};

use crate::builder_surface::SourceFileKind;
use crate::compiler_frontend::compiler_errors::{CompilerError, ErrorType};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use std::mem::{align_of, size_of};
use std::path::{Path, PathBuf};

use super::test_support::database_with_retained_text;

#[test]
fn source_id_uses_the_non_zero_option_niche() {
    assert_eq!(size_of::<SourceId>(), 4);
    assert_eq!(align_of::<SourceId>(), 4);
    assert_eq!(size_of::<Option<SourceId>>(), 4);
}

#[test]
fn source_id_try_from_index_spans_the_full_compact_domain() {
    assert_eq!(
        SourceId::try_from_index(0),
        Some(SourceId::COMPILATION_ROOT)
    );

    let last_index = u32::MAX as usize - 1;
    assert_eq!(
        SourceId::try_from_index(last_index).map(SourceId::index),
        Some(last_index)
    );

    // One past the last usable identity is authored table exhaustion, reported fallibly
    // instead of wrapping the identity or panicking.
    assert_eq!(SourceId::try_from_index(u32::MAX as usize), None);
    assert_eq!(
        SourceId::try_from_index(u32::MAX as usize + 1),
        None,
        "indexes beyond the u32 domain must be rejected without a lossy cast"
    );
}

#[test]
fn registered_physical_sources_carry_the_kind_named_by_their_extension() {
    let moth_path = PathBuf::from("/project/main.moth");
    let template_path = PathBuf::from("/project/page.mtf");
    let markdown_path = PathBuf::from("/project/notes.md");
    let mut string_table = StringTable::new();
    let database = SourceDatabase::build(
        [&moth_path, &template_path, &markdown_path],
        &moth_path,
        None,
        &mut string_table,
    )
    .expect("recognized sources should register");

    assert_eq!(
        database
            .get_by_canonical_path(&moth_path)
            .and_then(|record| record.kind),
        Some(SourceKind::Compiler(SourceFileKind::Moth))
    );
    assert_eq!(
        database
            .get_by_canonical_path(&template_path)
            .and_then(|record| record.kind),
        Some(SourceKind::Compiler(SourceFileKind::MothTemplate))
    );
    assert_eq!(
        database
            .get_by_canonical_path(&markdown_path)
            .and_then(|record| record.kind),
        Some(SourceKind::Compiler(SourceFileKind::PlainMarkdown))
    );
}

#[test]
fn registered_provider_owned_physical_sources_keep_their_identity() {
    let path = PathBuf::from("/project/drawing.js");
    let mut string_table = StringTable::new();
    let database = SourceDatabase::build(std::iter::once(&path), &path, None, &mut string_table)
        .expect("provider-owned physical sources should register");
    let record = database
        .get_by_canonical_path(&path)
        .expect("provider-owned identity should be retained");

    assert_eq!(record.kind, Some(SourceKind::ProviderOwned));
    assert_eq!(record.id, SourceId::from_index(1));
    assert_eq!(record.canonical_os_path.as_deref(), Some(path.as_path()));
}

#[test]
fn registration_index_sorted_by_logical_path_keeps_authored_kind_when_canonical_extension_disagrees()
 {
    let path = PathBuf::from("/project/payload.bin");
    let mut string_table = StringTable::new();
    let registration_index = SourceRegistrationIndex::from_rows(std::iter::once((
        path.as_path(),
        SourceKind::Compiler(SourceFileKind::MothTemplate),
    )));
    let database = SourceDatabase::from_registration_index_sorted_by_logical_path(
        &registration_index,
        &path,
        None,
        &mut string_table,
    )
    .expect("authored-kind registration should succeed");
    let record = database
        .get_by_canonical_path(&path)
        .expect("registered source should retain its identity");

    assert_eq!(
        record.kind,
        Some(SourceKind::Compiler(SourceFileKind::MothTemplate)),
        "authored kind must survive a canonical extension that would derive ProviderOwned"
    );
}

#[test]
fn append_ordered_registration_index_keeps_authored_kind_when_canonical_extension_disagrees() {
    let path = PathBuf::from("/project/payload.bin");
    let mut string_table = StringTable::new();
    let registration_index = SourceRegistrationIndex::from_rows(std::iter::once((
        path.as_path(),
        SourceKind::Compiler(SourceFileKind::MothTemplate),
    )));
    let mut database = SourceDatabase::empty();
    database
        .append_ordered_registration_index(&registration_index, &path, None, &mut string_table)
        .expect("authored-kind registration should succeed");
    let record = database
        .get_by_canonical_path(&path)
        .expect("indexed source should retain its identity");

    assert_eq!(
        record.kind,
        Some(SourceKind::Compiler(SourceFileKind::MothTemplate)),
        "authored kind must survive a canonical extension that would derive ProviderOwned"
    );
}

#[test]
fn insert_keeps_authored_kind_when_canonical_extension_disagrees() {
    let path = PathBuf::from("/project/payload.bin");
    let mut string_table = StringTable::new();
    let mut database = SourceDatabase::empty();
    database
        .insert(
            path.clone(),
            SourceKind::Compiler(SourceFileKind::MothTemplate),
            &path,
            None,
            &mut string_table,
        )
        .expect("authored-kind insert should succeed");
    let record = database
        .get_by_canonical_path(&path)
        .expect("inserted source should retain its identity");

    assert_eq!(
        record.kind,
        Some(SourceKind::Compiler(SourceFileKind::MothTemplate)),
        "authored kind must survive a canonical extension that would derive ProviderOwned"
    );
}

#[test]
fn source_database_keeps_each_load_failure_with_its_source_identity() {
    let first_failed_path = PathBuf::from("/project/first-failed.moth");
    let second_failed_path = PathBuf::from("/project/second-failed.moth");
    let loaded_path = PathBuf::from("/project/loaded.moth");
    let pending_path = PathBuf::from("/project/pending.moth");
    let mut string_table = StringTable::new();
    let mut database = SourceDatabase::build(
        [
            &first_failed_path,
            &second_failed_path,
            &loaded_path,
            &pending_path,
        ],
        &first_failed_path,
        None,
        &mut string_table,
    )
    .expect("source identities should build");

    let first_failed_id = database
        .get_by_canonical_path(&first_failed_path)
        .expect("first failed source should be registered")
        .id;
    let second_failed_id = database
        .get_by_canonical_path(&second_failed_path)
        .expect("second failed source should be registered")
        .id;
    let loaded_id = database
        .get_by_canonical_path(&loaded_path)
        .expect("loaded source should be registered")
        .id;
    let pending_id = database
        .get_by_canonical_path(&pending_path)
        .expect("pending source should be registered")
        .id;

    let first_error = CompilerError::file_error(&first_failed_path, "first source read failed");
    database
        .record_source_load_error(first_failed_id, first_error)
        .expect("first source load error should be retained");
    let second_error = CompilerError::file_error(&second_failed_path, "second source read failed");
    database
        .record_source_load_error(second_failed_id, second_error)
        .expect("second source load error should be retained");
    database
        .retain_text(loaded_id, "loaded source".to_owned())
        .expect("loaded source text should be retained");

    assert_eq!(
        database
            .source_load_error(first_failed_id)
            .map(|error| error.msg.as_str()),
        Some("first source read failed"),
        "the first failed source must resolve its own structured error"
    );
    assert_eq!(
        database
            .source_load_error(second_failed_id)
            .map(|error| error.msg.as_str()),
        Some("second source read failed"),
        "the second failed source must resolve its own structured error"
    );
    assert!(
        database.source_load_error(loaded_id).is_none(),
        "a loaded source has no load failure"
    );
    assert!(
        database.source_load_error(pending_id).is_none(),
        "a pending source has no load failure"
    );

    assert!(database.retained_text(first_failed_id).is_none());
    assert!(database.retained_text(second_failed_id).is_none());
    assert!(database.line_index(first_failed_id).is_none());
    assert!(database.line_index(second_failed_id).is_none());
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
            SourceKind::Compiler(SourceFileKind::Moth),
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

    let first_logical_paths = render_source_logical_paths(&first, &first_strings);
    let second_logical_paths = render_source_logical_paths(&second, &second_strings);
    assert_eq!(first_logical_paths, second_logical_paths);
    assert_eq!(
        first_logical_paths,
        vec![
            PathBuf::from("alpha.moth"),
            PathBuf::from("middle.moth"),
            PathBuf::from("zeta.moth"),
        ]
    );

    for path in &input_paths {
        let first_slot = first
            .get_by_canonical_path(path)
            .expect("registered source");
        let second_slot = second
            .get_by_canonical_path(path)
            .expect("registered source");
        assert_eq!(first_slot.id, second_slot.id);
        assert_eq!(first_slot.logical_path, second_slot.logical_path);
    }

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

    let path_table = database.paths();
    let mut scratch = Vec::new();
    let alpha_logical_paths = database
        .iter()
        .filter(|record| {
            record
                .canonical_os_path
                .as_deref()
                .is_some_and(|path| path.starts_with("/project/alpha"))
        })
        .map(|record| path_table.render_portable(record.logical_path, &string_table, &mut scratch))
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
            SourceKind::Compiler(SourceFileKind::Moth),
            &module_a_source,
            None,
            &mut string_table,
        )
        .expect("module A should resolve the shared source");
    let module_b_view = database
        .insert(
            shared_source.clone(),
            SourceKind::Compiler(SourceFileKind::Moth),
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
            SourceKind::Compiler(SourceFileKind::Moth),
            &first_entry_path,
            None,
            &mut string_table,
        )
        .expect("the first source registration should succeed");
    let error = database
        .insert(
            canonical_path,
            SourceKind::Compiler(SourceFileKind::Moth),
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
fn conflicting_kind_for_canonical_source_is_rejected() {
    let canonical_path = PathBuf::from("/project/src/shared.moth");
    let entry_path = PathBuf::from("/project/entry.moth");
    let mut string_table = StringTable::new();
    let mut database = SourceDatabase::empty();
    let moth = SourceKind::Compiler(SourceFileKind::Moth);
    let moth_template = SourceKind::Compiler(SourceFileKind::MothTemplate);

    let first_id = database
        .insert(
            canonical_path.clone(),
            moth,
            &entry_path,
            None,
            &mut string_table,
        )
        .expect("the first source registration should succeed");
    let repeated_id = database
        .insert(
            canonical_path.clone(),
            moth,
            &entry_path,
            None,
            &mut string_table,
        )
        .expect("the same canonical path and kind should reuse the existing identity");
    let error = database
        .insert(
            canonical_path.clone(),
            moth_template,
            &entry_path,
            None,
            &mut string_table,
        )
        .expect_err("a canonical source cannot change authored kind");

    assert_eq!(first_id, repeated_id);
    assert!(
        error.msg.contains("/project/src/shared.moth")
            && error.msg.contains(&format!("{moth:?}"))
            && error.msg.contains(&format!("{moth_template:?}")),
        "the conflict should name the canonical path and both kinds: {}",
        error.msg
    );
    assert_eq!(error.error_type, ErrorType::Compiler);
    assert_eq!(
        database
            .get_by_canonical_path(&canonical_path)
            .map(|record| (record.id, record.kind)),
        Some((first_id, Some(moth))),
        "the rejected registration must leave the first identity and kind in place"
    );
}

#[test]
fn source_snapshot_size_bound_rejects_u32_max_by_provenance() {
    let limit = u32::MAX as usize;
    let largest_accepted = limit - 1;
    let source_path = PathBuf::from("/project/huge.moth");

    ensure_source_snapshot_fits(
        largest_accepted,
        SourceProvenance::AuthoredPhysical,
        Some(source_path.as_path()),
    )
    .expect("physical snapshot just under u32::MAX must be accepted");
    ensure_source_snapshot_fits(largest_accepted, SourceProvenance::CompilationRoot, None)
        .expect("synthetic snapshot just under u32::MAX must be accepted");

    let physical = ensure_source_snapshot_fits(
        limit,
        SourceProvenance::AuthoredPhysical,
        Some(source_path.as_path()),
    )
    .expect_err("physical snapshot of u32::MAX bytes must be rejected");
    let physical_without_path =
        ensure_source_snapshot_fits(limit, SourceProvenance::AuthoredPhysical, None)
            .expect_err("a pathless physical snapshot must be rejected");
    let synthetic = ensure_source_snapshot_fits(limit, SourceProvenance::CompilationRoot, None)
        .expect_err("synthetic snapshot of u32::MAX bytes must be rejected");

    assert_eq!(physical.error_type, ErrorType::File);
    assert_eq!(physical.source_span, None);
    assert_eq!(physical.host_path.as_deref(), Some(source_path.as_path()));
    assert_eq!(physical_without_path.error_type, ErrorType::File);
    assert_eq!(physical_without_path.source_span, None);
    assert_eq!(physical_without_path.host_path, None);
    assert_eq!(synthetic.error_type, ErrorType::Compiler);
    assert_eq!(synthetic.source_span, None);
    assert_eq!(synthetic.host_path, None);
    assert!(
        physical.msg.contains("huge.moth") && physical.msg.contains(&limit.to_string()),
        "the user-facing failure must name the oversized file and the limit: {}",
        physical.msg
    );
    assert!(
        synthetic.msg.contains(&limit.to_string()),
        "the compiler-bug failure must name the limit: {}",
        synthetic.msg
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
    let empty_line_index = database
        .line_index(empty_id)
        .expect("loaded empty source should expose its line index");
    assert_eq!(
        empty_line_index.line_count(),
        0,
        "an empty snapshot has no table entries, matching str::lines()"
    );
    assert_eq!(
        empty_line_index.position(0),
        Some(LinePosition { line: 0, column: 0 }),
        "empty EOF still resolves as line 0, column 0"
    );
    assert!(database.source_load_error(empty_id).is_none());

    let load_error = CompilerError::file_error(&failed_path, "source read failed");
    database
        .record_source_load_error(failed_id, load_error)
        .expect("source load error should be retained");
    assert!(database.retained_text(failed_id).is_none());
    assert!(database.source_load_error(failed_id).is_some());
    assert!(
        database.line_index(failed_id).is_none(),
        "an unreadable source has no line table"
    );

    // Every further write is refused and leaves the recorded slot untouched, so a stage can
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

    let repeated_failure = CompilerError::file_error(&failed_path, "source read failed again");
    database
        .record_source_load_error(failed_id, repeated_failure)
        .expect_err("an unreadable source cannot record a second failure");
    assert!(database.source_load_error(failed_id).is_some());

    // The reserved compilation root is addressable but never writable.
    database
        .retain_text(SourceId::from_index(0), "root snapshot".to_owned())
        .expect_err("the compilation root cannot retain source text");

    let load_after_snapshot =
        CompilerError::file_error(&empty_path, "source read failed after snapshot");
    database
        .record_source_load_error(empty_id, load_after_snapshot)
        .expect_err("a loaded source cannot receive a second load status");
    assert_eq!(database.retained_text(empty_id), Some(""));
    assert!(database.source_load_error(empty_id).is_none());
}

#[test]
fn retaining_source_text_moves_the_original_allocation_into_the_loaded_record() {
    let source_path = PathBuf::from("/project/main.moth");
    let mut string_table = StringTable::new();
    let mut database = SourceDatabase::build(
        std::iter::once(&source_path),
        &source_path,
        None,
        &mut string_table,
    )
    .expect("source identity should build");
    let source_id = database
        .get_by_canonical_path(&source_path)
        .expect("source should be registered")
        .id;
    let text = "x".repeat(4096);
    let original_ptr = text.as_ptr();

    database
        .retain_text(source_id, text)
        .expect("source text should be retained");

    let retained = database
        .retained_text(source_id)
        .expect("loaded source should retain text");
    assert_eq!(retained.as_ptr(), original_ptr);
    assert_eq!(retained, "x".repeat(4096));
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
    let logical_path = database.legacy_logical_path(source_id);

    database
        .retain_text(source_id, "compiled snapshot\n".to_owned())
        .expect("source text should be retained");

    assert_eq!(
        database
            .unique_record_for_logical_path(&logical_path)
            .and_then(|slot| database.retained_text(slot.id)),
        Some("compiled snapshot\n"),
    );
}

#[test]
fn loaded_source_line_ranges_reconstruct_the_snapshot() {
    // Each case names the exact lines the table must produce, including their terminators.
    // Reconstruction alone cannot catch a misplaced interior boundary, because any partition
    // of the snapshot concatenates back to it.
    let cases: &[(&str, &str, &[&str])] = &[
        ("empty text", "", &[]),
        ("text with no newline", "hello", &["hello"]),
        ("text ending in a newline", "hello\n", &["hello\n"]),
        (
            "text ending without a newline",
            "hello\nworld",
            &["hello\n", "world"],
        ),
        (
            "consecutive blank lines",
            "a\n\n\nb",
            &["a\n", "\n", "\n", "b"],
        ),
        (
            "CRLF line endings",
            "hello\r\nworld\r\n",
            &["hello\r\n", "world\r\n"],
        ),
        (
            "multi-byte UTF-8 adjacent to a newline",
            "α\nβ",
            &["α\n", "β"],
        ),
        ("bare CR line ending", "a\rb", &["a\r", "b"]),
        ("a single newline", "\n", &["\n"]),
        ("a trailing bare CR", "a\r", &["a\r"]),
    ];

    for (label, text, expected_lines) in cases {
        let (database, source_id) = database_with_retained_text(text);
        let source_text = database
            .retained_text(source_id)
            .expect("loaded source should retain text");
        assert_eq!(source_text, *text, "{label}: retained text");
        let line_index = database
            .line_index(source_id)
            .expect("loaded source should expose its line index");
        assert_eq!(
            line_index.line_count() as usize,
            expected_lines.len(),
            "{label}: line count"
        );

        let lines: Vec<&str> = (0..line_index.line_count())
            .map(|line_number| {
                let range = line_index
                    .line_byte_range(line_number)
                    .expect("every counted line should resolve");
                &source_text[range.start as usize..range.end as usize]
            })
            .collect();
        assert_eq!(lines, *expected_lines, "{label}: line boundaries");
        assert_eq!(lines.concat(), *text, "{label}: reconstructed snapshot");
        assert!(
            line_index
                .line_byte_range(line_index.line_count())
                .is_none(),
            "{label}: past-the-end line should not resolve"
        );
    }
}

fn render_source_logical_paths(
    database: &SourceDatabase,
    string_table: &StringTable,
) -> Vec<PathBuf> {
    let path_table = database.paths();
    let mut scratch = Vec::new();
    database
        .iter()
        .map(|record| {
            PathBuf::from(path_table.render_portable(
                record.logical_path,
                string_table,
                &mut scratch,
            ))
        })
        .collect()
}
