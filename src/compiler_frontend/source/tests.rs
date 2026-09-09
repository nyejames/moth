use super::{
    ExtendedSpanBuilder, LocalSpan, SourceDatabase, SourceDatabaseBuilder, SourceId, SourceKind,
    SourceProvenance, SourceRecord, SourceRegistrationIndex, SourceSlot, SpanCapacityReason,
    line_index::{LineIndex, LinePosition, line_start_offsets},
    record::ensure_source_snapshot_fits,
    span::{SourceSpan, SpanJoinError},
};

use std::cmp::Ordering;

use crate::builder_surface::{SourceFileKind, SourceFileKindRegistry};
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages, ErrorType};
use crate::compiler_frontend::compiler_messages::compiler_diagnostic::CompilerDiagnostic;
use crate::compiler_frontend::compiler_messages::render::dev_server::render_compiler_messages_html;
use crate::compiler_frontend::compiler_messages::source_location::{CharPosition, SourceLocation};
use crate::compiler_frontend::headers::parse_file_headers::FileFrontendPrepareFailure;
use crate::compiler_frontend::paths::path_resolution::ProjectPathResolver;
use crate::compiler_frontend::pipeline::CompilerFrontend;
use crate::compiler_frontend::source_packages::root_file::PreparedSourcePackageRoots;
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::TokenizerEntryMode;
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

    let first_error = CompilerError::file_error(
        &first_failed_path,
        "first source read failed",
        &mut string_table,
    );
    database
        .record_source_load_error(first_failed_id, first_error)
        .expect("first source load error should be retained");
    let second_error = CompilerError::file_error(
        &second_failed_path,
        "second source read failed",
        &mut string_table,
    );
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
    let mut string_table = StringTable::new();
    let logical_path = InternedPath::from_single_str("huge.moth", &mut string_table);
    let root_path = InternedPath::from_single_str("<compilation root>", &mut string_table);

    ensure_source_snapshot_fits(
        largest_accepted,
        SourceProvenance::AuthoredPhysical,
        &logical_path,
        Some(source_path.as_path()),
    )
    .expect("physical snapshot just under u32::MAX must be accepted");
    ensure_source_snapshot_fits(
        largest_accepted,
        SourceProvenance::CompilationRoot,
        &root_path,
        None,
    )
    .expect("synthetic snapshot just under u32::MAX must be accepted");

    let physical = ensure_source_snapshot_fits(
        limit,
        SourceProvenance::AuthoredPhysical,
        &logical_path,
        Some(source_path.as_path()),
    )
    .expect_err("physical snapshot of u32::MAX bytes must be rejected");
    let synthetic =
        ensure_source_snapshot_fits(limit, SourceProvenance::CompilationRoot, &root_path, None)
            .expect_err("synthetic snapshot of u32::MAX bytes must be rejected");

    assert_eq!(physical.error_type, ErrorType::File);
    assert_eq!(synthetic.error_type, ErrorType::Compiler);
    assert!(
        physical.msg.contains("huge.moth") && physical.msg.contains(&limit.to_string()),
        "the user-facing failure must name the oversized file and the limit: {}",
        physical.msg
    );
    // A renderer resolves the frame from the location's scope, not from the message text.
    assert_eq!(
        physical.location.scope, logical_path,
        "the user-facing failure must carry the source's own identity so it can be located"
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

    let load_error =
        CompilerError::file_error(&failed_path, "source read failed", &mut string_table);
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

#[test]
fn line_index_resolves_newline_shapes_and_zero_width_eof() {
    let crlf = "ab\r\n";
    let crlf_starts = line_start_offsets(crlf);
    let crlf_index = LineIndex::new(crlf, &crlf_starts);
    assert_eq!(crlf_index.line_count(), 1);
    assert_eq!(crlf_index.line_text(0), Some("ab"));
    for offset in [2, 3, 4] {
        assert_eq!(
            crlf_index.position(offset),
            Some(LinePosition { line: 0, column: 2 }),
            "CRLF offset {offset} should render at the visible line end"
        );
        assert_eq!(
            crlf_index.utf16_column(offset),
            Some(2),
            "UTF-16 conversion must use the same visible-line end"
        );
    }
    assert_eq!(crlf_index.line_of_offset(4), Some(0));

    let bare_cr = "ab\rc";
    let bare_cr_starts = line_start_offsets(bare_cr);
    let bare_cr_index = LineIndex::new(bare_cr, &bare_cr_starts);
    assert_eq!(bare_cr_index.line_count(), 2);
    assert_eq!(bare_cr_index.line_text(0), Some("ab"));
    assert_eq!(bare_cr_index.line_text(1), Some("c"));
    assert_eq!(
        bare_cr_index.position(2),
        Some(LinePosition { line: 0, column: 2 })
    );
    assert_eq!(
        bare_cr_index.position(3),
        Some(LinePosition { line: 1, column: 0 })
    );

    let trailing_bare_cr = "ab\r";
    let trailing_bare_cr_starts = line_start_offsets(trailing_bare_cr);
    let trailing_bare_cr_index = LineIndex::new(trailing_bare_cr, &trailing_bare_cr_starts);
    assert_eq!(trailing_bare_cr_index.line_count(), 1);
    assert_eq!(trailing_bare_cr_index.line_text(0), Some("ab"));
    assert_eq!(
        trailing_bare_cr_index.position(3),
        Some(LinePosition { line: 0, column: 2 })
    );

    let final_newline = "ab\n";
    let final_newline_starts = line_start_offsets(final_newline);
    let final_newline_index = LineIndex::new(final_newline, &final_newline_starts);
    assert_eq!(final_newline_index.line_count(), 1);
    assert_eq!(final_newline_index.line_text(0), Some("ab"));
    assert_eq!(
        final_newline_index.position(3),
        Some(LinePosition { line: 0, column: 2 })
    );
    assert_eq!(final_newline_index.line_of_offset(3), Some(0));

    let no_final_newline = "ab\ncd";
    let no_final_newline_starts = line_start_offsets(no_final_newline);
    let no_final_newline_index = LineIndex::new(no_final_newline, &no_final_newline_starts);
    assert_eq!(no_final_newline_index.line_count(), 2);
    assert_eq!(no_final_newline_index.line_text(1), Some("cd"));
    assert_eq!(
        no_final_newline_index.position(5),
        Some(LinePosition { line: 1, column: 2 })
    );

    let empty = "";
    let empty_starts = line_start_offsets(empty);
    let empty_index = LineIndex::new(empty, &empty_starts);
    assert_eq!(empty_index.line_count(), 0);
    assert_eq!(empty_index.line_text(0), None);
    assert_eq!(empty_index.line_of_offset(0), Some(0));
    assert_eq!(
        empty_index.position(0),
        Some(LinePosition { line: 0, column: 0 })
    );
    assert_eq!(empty_index.utf16_column(0), Some(0));
}

/// `str::lines()` agrees only for the LF and CRLF subset; bare CR is a tokenizer line break.
#[test]
fn line_text_matches_str_lines_for_lf_and_crlf() {
    let text = "lf\ncrlf\r\nempty\n\r\nlast";
    let line_starts = line_start_offsets(text);
    let line_index = LineIndex::new(text, &line_starts);
    let expected_lines: Vec<&str> = text.lines().collect();

    let actual_lines: Vec<&str> = (0..line_index.line_count())
        .map(|line| {
            line_index
                .line_text(line)
                .expect("counted line should have text")
        })
        .collect();
    assert_eq!(actual_lines, expected_lines);
}

#[test]
fn line_index_counts_scalar_and_utf16_columns_at_arbitrary_offsets() {
    let text = "aé😀z";
    let line_starts = line_start_offsets(text);
    let line_index = LineIndex::new(text, &line_starts);

    // `é` begins at byte 1 and occupies bytes 1..3; byte 2 is not a UTF-8 boundary.
    assert_eq!(
        line_index.position(2),
        Some(LinePosition { line: 0, column: 2 })
    );
    assert_eq!(
        line_index.position(5),
        Some(LinePosition { line: 0, column: 3 })
    );
    assert_eq!(
        line_index.utf16_column(5),
        Some(4),
        "the astral scalar occupies two UTF-16 code units"
    );
    assert_eq!(
        line_index.position(text.len() as u32),
        Some(LinePosition { line: 0, column: 4 })
    );
    assert_eq!(line_index.utf16_column(text.len() as u32), Some(5));
    assert_eq!(line_index.position(text.len() as u32 + 1), None);
    assert_eq!(line_index.utf16_column(text.len() as u32 + 1), None);
    assert_eq!(line_index.line_of_offset(text.len() as u32 + 1), None);
}

#[test]
fn line_index_keeps_long_lines_addressable_without_truncation() {
    let text = format!("{}\nend", "x".repeat(4096));
    let line_starts = line_start_offsets(&text);
    let line_index = LineIndex::new(&text, &line_starts);

    assert_eq!(line_index.line_count(), 2);
    assert_eq!(line_index.line_text(0).map(str::len), Some(4096));
    assert_eq!(
        line_index.position(4096),
        Some(LinePosition {
            line: 0,
            column: 4096
        })
    );
    assert_eq!(
        line_index.position(4097),
        Some(LinePosition { line: 1, column: 0 })
    );
    assert_eq!(
        line_index.position(text.len() as u32),
        Some(LinePosition { line: 1, column: 3 })
    );
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

fn database_with_retained_text(text: &str) -> (SourceDatabase, SourceId) {
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
    database
        .retain_text(source_id, text.to_owned())
        .expect("source text should be retained");
    (database, source_id)
}

fn compiler_bug_panic_message(panic: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = panic.downcast_ref::<String>() {
        return message.clone();
    }
    if let Some(message) = panic.downcast_ref::<&str>() {
        return (*message).to_owned();
    }
    panic!("compiler bug panic should carry a string message");
}

#[test]
fn installing_extended_spans_preserves_live_record_resolution() {
    let (mut database, source_id) = database_with_retained_text("source snapshot");
    let mut builder = ExtendedSpanBuilder::new();
    let inline = LocalSpan::exact(12, 8, &mut builder).expect("inline span");
    let extended = LocalSpan::exact(5_000, 2_000, &mut builder).expect("extended span");
    let empty_extended = LocalSpan::insertion_point(4 * 1024 * 1024, &mut builder)
        .expect("extended insertion point");

    let live_ranges = [
        inline.resolve_with(builder.resolver()),
        extended.resolve_with(builder.resolver()),
        empty_extended.resolve_with(builder.resolver()),
    ];
    let table = builder.freeze();
    database
        .install_extended_spans(source_id, table)
        .expect("the loaded source should install its table once");

    let record = database.source_record(source_id);
    assert_eq!(inline.resolve(record), live_ranges[0]);
    assert_eq!(extended.resolve(record), live_ranges[1]);
    assert_eq!(empty_extended.resolve(record), live_ranges[2]);
    assert!(!inline.is_empty(record));
    assert!(empty_extended.is_empty(record));
}

#[test]
fn repeated_source_preparation_preserves_earlier_extended_spans() {
    let text = format!("{}{}", "a".repeat(2_000), "b".repeat(2_000));
    let (database, source_id) = database_with_retained_text(&text);
    let mut sources = SourceDatabaseBuilder::new(database);
    let mut first = sources.split().1.take_span_builder(source_id);
    let first_span = LocalSpan::exact(0, 2_000, &mut first).expect("first extended span");
    sources.retain_span_builder(source_id, first);

    let (_, mut builders) = sources.split();
    let mut repeated = builders.take_span_builder(source_id);
    let repeated_span =
        LocalSpan::exact(2_000, 2_000, &mut repeated).expect("repeated extended span");
    builders.retain_span_builder(source_id, repeated);
    let database = sources
        .finish()
        .expect("all producers returned their builders");
    let first = first_span.resolve(database.source_record(source_id));
    let repeated = repeated_span.resolve(database.source_record(source_id));
    let snapshot = database
        .retained_text(source_id)
        .expect("retained snapshot");
    assert_eq!(
        &snapshot[first.start() as usize..first.end() as usize],
        "a".repeat(2_000)
    );
    assert_eq!(
        &snapshot[repeated.start() as usize..repeated.end() as usize],
        "b".repeat(2_000)
    );
}

#[test]
#[should_panic]
fn source_finalization_rejects_a_checked_out_span_builder() {
    let (database, source_id) = database_with_retained_text("source snapshot");
    let mut sources = SourceDatabaseBuilder::new(database);
    let _active_producer = sources.split().1.take_span_builder(source_id);
    let _ = sources.finish();
}

#[test]
fn installing_extended_spans_twice_is_a_compiler_bug() {
    let (mut database, source_id) = database_with_retained_text("source snapshot");
    database
        .install_extended_spans(source_id, ExtendedSpanBuilder::new().freeze())
        .expect("the first table installation should succeed");

    let error = database
        .install_extended_spans(source_id, ExtendedSpanBuilder::new().freeze())
        .expect_err("a source can install its table only once");

    assert_eq!(error.error_type, ErrorType::Compiler);
    assert!(
        error.msg.contains(&source_id.index().to_string()) && error.msg.contains("more than once"),
        "the duplicate installation should name the source and lifecycle violation: {}",
        error.msg
    );
}

#[test]
fn installing_extended_spans_for_a_source_that_never_loaded_is_a_compiler_bug() {
    let source_path = PathBuf::from("/project/pending.moth");
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

    let error = database
        .install_extended_spans(source_id, ExtendedSpanBuilder::new().freeze())
        .expect_err("an unloaded source cannot install an extended-span table");

    assert_eq!(error.error_type, ErrorType::Compiler);
    assert!(
        error.msg.contains(&source_id.index().to_string())
            && error.msg.contains("before its source was loaded"),
        "the unloaded-source failure should name the source and lifecycle violation: {}",
        error.msg
    );
}

#[test]
fn installing_extended_spans_for_the_compilation_root_is_a_compiler_bug() {
    let mut database = SourceDatabase::empty();
    let source_id = SourceId::from_index(0);

    let error = database
        .install_extended_spans(source_id, ExtendedSpanBuilder::new().freeze())
        .expect_err("the compilation root cannot install an extended-span table");

    assert_eq!(error.error_type, ErrorType::Compiler);
    assert!(
        error.msg.contains(&source_id.index().to_string())
            && error.msg.contains("compilation root"),
        "the root failure should name the source and lifecycle violation: {}",
        error.msg
    );
}

#[test]
fn inline_span_resolves_through_a_record_before_table_installation() {
    let (database, source_id) = database_with_retained_text("source snapshot");
    let mut builder = ExtendedSpanBuilder::new();
    let inline = LocalSpan::exact(12, 8, &mut builder).expect("inline span");
    assert!(builder.is_empty());

    let record = database.source_record(source_id);
    let range = inline.resolve(record);
    assert_eq!((range.start(), range.end()), (12, 20));
    assert!(!inline.is_empty(record));
}

#[test]
fn extended_span_without_an_installed_table_names_its_source_identity() {
    let (database, source_id) = database_with_retained_text("source snapshot");
    let mut builder = ExtendedSpanBuilder::new();
    let local = LocalSpan::exact(5_000, 2_000, &mut builder).expect("extended span");
    let span = SourceSpan::new(source_id, local);

    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        span.byte_range(&database);
    }))
    .expect_err("an extended span cannot resolve before its builder is installed");
    let message = compiler_bug_panic_message(panic);

    assert!(
        message.contains(&source_id.index().to_string())
            && message.contains("builder")
            && message.contains("never installed"),
        "the missing-table panic should name the source and lifecycle violation: {message}"
    );
}

#[test]
fn database_span_reads_reject_missing_source_records() {
    let source_path = PathBuf::from("/project/pending.moth");
    let mut string_table = StringTable::new();
    let database = SourceDatabase::build(
        std::iter::once(&source_path),
        &source_path,
        None,
        &mut string_table,
    )
    .expect("source identity should build");
    let pending_id = database
        .get_by_canonical_path(&source_path)
        .expect("source should be registered")
        .id;
    let mut builder = ExtendedSpanBuilder::new();
    let local = LocalSpan::exact(12, 8, &mut builder).expect("inline span");

    for source_id in [
        SourceId::from_index(0),
        pending_id,
        SourceId::from_index(99),
    ] {
        let span = SourceSpan::new(source_id, local);
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            span.byte_range(&database);
        }))
        .expect_err("a span must not resolve without its source record");
        let message = compiler_bug_panic_message(panic);

        assert!(
            message.contains(&source_id.index().to_string()) && message.contains("compiler bug"),
            "the missing source-record panic should name source {}: {message}",
            source_id.index()
        );
    }
}

#[test]
fn compilation_root_span_resolves_only_its_exact_empty_range() {
    let source_path = PathBuf::from("/project/main.moth");
    let mut string_table = StringTable::new();
    let database = SourceDatabase::build(
        std::iter::once(&source_path),
        &source_path,
        None,
        &mut string_table,
    )
    .expect("source identity should build");

    let root = SourceSpan::new(SourceId::COMPILATION_ROOT, LocalSpan::source_start());
    let range = root.byte_range(&database);
    assert_eq!((range.start(), range.end()), (0, 0));
    assert_eq!(root.start(&database), 0);
    assert_eq!(root.end(&database), 0);
    assert!(root.contains(root, &database));
}

#[test]
fn non_empty_compilation_root_spans_fail_loudly_as_compiler_bugs() {
    let database = SourceDatabase::empty();
    let mut builder = ExtendedSpanBuilder::new();

    for local in [
        LocalSpan::exact(4, 3, &mut builder).expect("non-empty inline root span"),
        LocalSpan::insertion_point(7, &mut builder).expect("non-zero empty root span"),
        LocalSpan::exact(5_000, 2_000, &mut builder).expect("extended root span"),
    ] {
        let span = SourceSpan::new(SourceId::COMPILATION_ROOT, local);
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            span.byte_range(&database);
        }))
        .expect_err("a non-empty compilation-root span must not resolve silently");
        let message = compiler_bug_panic_message(panic);

        assert!(
            message.contains("compilation root")
                && message.contains(&SourceId::COMPILATION_ROOT.index().to_string())
                && message.contains("compiler bug"),
            "the root range rejection should name the identity as a compiler bug: {message}"
        );
    }

    // The rejection must come from the span contract itself, not from a record lookup accident.
    let non_empty = SourceSpan::new(
        SourceId::COMPILATION_ROOT,
        LocalSpan::exact(4, 3, &mut builder).expect("non-empty inline root span"),
    );
    let overlap_panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        non_empty.overlaps(non_empty, &database);
    }))
    .expect_err("a non-empty root span must fail the same way through overlaps");
    let message = compiler_bug_panic_message(overlap_panic);
    assert!(
        message.contains("compilation root") && message.contains("compiler bug"),
        "overlaps must route through the same root span contract: {message}"
    );
}

#[test]
fn database_span_operations_match_live_resolvers_and_reject_cross_source_pairs() {
    let first_path = PathBuf::from("/project/first.moth");

    let second_path = PathBuf::from("/project/second.moth");
    let mut string_table = StringTable::new();
    let mut database = SourceDatabase::build(
        [&first_path, &second_path],
        &first_path,
        None,
        &mut string_table,
    )
    .expect("source identities should build");
    let first_id = database
        .get_by_canonical_path(&first_path)
        .expect("first source should be registered")
        .id;
    let second_id = database
        .get_by_canonical_path(&second_path)
        .expect("second source should be registered")
        .id;
    database
        .retain_text(first_id, "first source".to_owned())
        .expect("first source text should be retained");
    database
        .retain_text(second_id, "second source".to_owned())
        .expect("second source text should be retained");

    let mut first_builder = ExtendedSpanBuilder::new();
    let outer_local = LocalSpan::exact(5_000, 5_000, &mut first_builder).expect("outer span");
    let inner_local = LocalSpan::exact(6_000, 10, &mut first_builder).expect("inner span");
    let first_outer = SourceSpan::new(first_id, outer_local);
    let first_inner = SourceSpan::new(first_id, inner_local);
    let live_outer_range = outer_local.resolve_with(first_builder.resolver());
    let live_inner_range = inner_local.resolve_with(first_builder.resolver());
    let live_overlap = first_outer.overlaps_with(first_inner, first_builder.resolver_for(first_id));
    let live_containment =
        first_outer.contains_with(first_inner, first_builder.resolver_for(first_id));
    let first_table = first_builder.freeze();

    let mut second_builder = ExtendedSpanBuilder::new();
    let coincident_local =
        LocalSpan::exact(5_000, 5_000, &mut second_builder).expect("coincident span");
    let second_coincident = SourceSpan::new(second_id, coincident_local);
    let second_table = second_builder.freeze();

    database
        .install_extended_spans(first_id, first_table)
        .expect("first source should install its table");
    database
        .install_extended_spans(second_id, second_table)
        .expect("second source should install its table");

    assert_eq!(first_outer.byte_range(&database), live_outer_range);
    assert_eq!(first_inner.byte_range(&database), live_inner_range);
    assert_eq!(first_outer.start(&database), live_outer_range.start());
    assert_eq!(first_outer.end(&database), live_outer_range.end());
    assert_eq!(first_outer.overlaps(first_inner, &database), live_overlap);
    assert_eq!(
        first_outer.contains(first_inner, &database),
        live_containment
    );

    let first_coincident = SourceSpan::new(first_id, outer_local);
    assert_eq!(
        first_coincident.byte_range(&database),
        second_coincident.byte_range(&database)
    );
    assert!(!first_coincident.overlaps(second_coincident, &database));
    assert!(!first_coincident.contains(second_coincident, &database));
    assert!(!second_coincident.contains(first_coincident, &database));
    let absent_source = SourceSpan::new(SourceId::from_index(99), outer_local);
    let absent_cross_source = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        absent_source.overlaps(first_coincident, &database)
    }));
    assert!(
        absent_cross_source.is_ok(),
        "cross-source overlap must reject before source lookup"
    );
    assert!(!absent_cross_source.unwrap());
}

#[test]
fn local_and_source_spans_use_the_non_zero_option_niche() {
    assert_eq!(size_of::<LocalSpan>(), 4);
    assert_eq!(size_of::<Option<LocalSpan>>(), 4);
    assert_eq!(size_of::<SourceSpan>(), 8);
    assert_eq!(size_of::<Option<SourceSpan>>(), 8);
}

#[test]
fn largest_inline_length_encodes_inline_and_one_more_spills() {
    let mut builder = ExtendedSpanBuilder::new();
    let inline_span = LocalSpan::exact(0, 1022, &mut builder).expect("max inline length");
    let inline_range = inline_span.resolve_with(builder.resolver());

    assert!(builder.is_empty());
    assert_eq!(inline_range.start(), 0);
    assert_eq!(inline_range.end(), 1022);

    let extended_span =
        LocalSpan::exact(0, 1023, &mut builder).expect("one past max inline length");
    let extended_range = extended_span.resolve_with(builder.resolver());

    assert_eq!(builder.len(), 1);
    assert_eq!(extended_range.start(), 0);
    assert_eq!(extended_range.end(), 1023);
}

#[test]
fn largest_inline_start_encodes_inline_and_one_more_spills() {
    let mut builder = ExtendedSpanBuilder::new();
    let inline_start = 4 * 1024 * 1024 - 1;
    let inline_span = LocalSpan::exact(inline_start, 0, &mut builder).expect("max inline start");
    let inline_range = inline_span.resolve_with(builder.resolver());

    assert!(builder.is_empty());
    assert_eq!(inline_range.start(), inline_start);
    assert_eq!(inline_range.end(), inline_start);

    let spilled_start = 4 * 1024 * 1024;
    let extended_span =
        LocalSpan::exact(spilled_start, 0, &mut builder).expect("one past max inline start");
    let extended_range = extended_span.resolve_with(builder.resolver());

    assert_eq!(builder.len(), 1);
    assert_eq!(extended_range.start(), spilled_start);
    assert_eq!(extended_range.end(), spilled_start);
}

#[test]
fn largest_inline_start_and_length_encode_inline_without_spilling() {
    let mut builder = ExtendedSpanBuilder::new();
    let start = 4 * 1024 * 1024 - 1;
    let span = LocalSpan::exact(start, 1022, &mut builder)
        .expect("max inline start packed with max inline length");
    let range = span.resolve_with(builder.resolver());

    assert!(builder.is_empty());
    assert_eq!(range.start(), start);
    assert_eq!(range.end(), start + 1022);
}

#[test]
fn zero_length_span_at_offset_zero_round_trips() {
    let mut builder = ExtendedSpanBuilder::new();
    let span = LocalSpan::insertion_point(0, &mut builder).expect("empty span at byte 0");
    let range = span.resolve_with(builder.resolver());

    assert!(builder.is_empty());
    assert!(span.is_empty_with(builder.resolver()));
    assert_eq!(range.start(), 0);
    assert_eq!(range.end(), 0);
}

#[test]
fn zero_length_span_past_inline_start_limit_round_trips() {
    let mut builder = ExtendedSpanBuilder::new();
    let offset = 4 * 1024 * 1024;
    let span =
        LocalSpan::insertion_point(offset, &mut builder).expect("empty span past inline start");
    let range = span.resolve_with(builder.resolver());

    assert_eq!(builder.len(), 1);
    assert!(span.is_empty_with(builder.resolver()));
    assert_eq!(range.start(), offset);
    assert_eq!(range.end(), offset);
}

#[test]
fn last_usable_extended_index_encodes_and_one_past_it_is_capacity_error() {
    let mut builder = ExtendedSpanBuilder::new();
    let last_usable_index = 4_194_302_u32;

    for _ in 0..=last_usable_index {
        LocalSpan::exact(0, 1023, &mut builder)
            .expect("every index through the last usable extended slot must encode");
    }

    assert_eq!(builder.len(), last_usable_index as usize + 1);

    let error = LocalSpan::exact(0, 1023, &mut builder)
        .expect_err("one past the last usable extended index must fail");

    assert_eq!(error.start(), 0);
    assert_eq!(error.length(), 1023);
    assert_eq!(error.reason(), SpanCapacityReason::ExtendedTableFull);

    // The real exhausted table must keep token construction failures in preparation's
    // infrastructure lane. The lexer diagnoses the unterminated literal before token
    // construction, then capture hits the exhausted table and aborts terminally: the
    // diagnosis is replaced by the capacity failure carrying the offending exact range,
    // so no span is silently dropped.
    let source = format!("\"{}\"", "x".repeat(1023));
    let path = Path::new("capacity.moth");
    let mut strings = StringTable::new();
    let malformed_path = Path::new("unterminated.moth");
    let malformed_source = source[..source.len() - 1].to_owned();
    let mut sources =
        SourceDatabase::build([path, malformed_path], path, None, &mut strings).unwrap();
    let source_id = sources.get_by_canonical_path(path).unwrap().id;
    let malformed_id = sources.get_by_canonical_path(malformed_path).unwrap().id;
    sources.retain_text(source_id, source.clone()).unwrap();
    sources
        .retain_text(malformed_id, malformed_source.clone())
        .unwrap();
    let failure = CompilerFrontend::tokenize_source(
        &sources,
        &StyleDirectiveRegistry::built_ins(),
        &source,
        path,
        TokenizerEntryMode::SourceFile,
        &mut strings,
        &mut builder,
    )
    .expect_err("minting a long token must report exhausted source storage");
    let FileFrontendPrepareFailure::Infrastructure(error) = failure else {
        panic!("token storage failure must not become an authored-source diagnosis");
    };
    assert_eq!(error.error_type, ErrorType::File);

    assert_eq!(builder.len(), last_usable_index as usize + 1);
    let mut malformed_builder = ExtendedSpanBuilder::new();
    for _ in 0..=last_usable_index {
        LocalSpan::exact(0, 1023, &mut malformed_builder).unwrap();
    }
    let failure = CompilerFrontend::tokenize_source(
        &sources,
        &StyleDirectiveRegistry::built_ins(),
        &malformed_source,
        malformed_path,
        TokenizerEntryMode::SourceFile,
        &mut strings,
        &mut malformed_builder,
    )
    .expect_err("capture must abort terminally when the source's span table is exhausted");
    // The unterminated literal is diagnosed before token construction; capture then hits the
    // exhausted table and aborts terminally. The diagnosis is not published spanless: the
    // infrastructure failure carries the offending exact range instead.
    let FileFrontendPrepareFailure::Infrastructure(error) = failure else {
        panic!("capture exhaustion must surface the terminal capacity failure");
    };
    assert_eq!(error.error_type, ErrorType::File);
    assert_eq!(error.location.start_byte, 0);
    assert_eq!(error.location.end_byte, malformed_source.len() as u32);
    assert_eq!(malformed_builder.len(), last_usable_index as usize + 1);
}

#[test]
fn join_spills_to_extended_when_cover_exceeds_inline_length() {
    let mut builder = ExtendedSpanBuilder::new();
    let left = LocalSpan::exact(0, 600, &mut builder).expect("left");
    let right = LocalSpan::exact(500, 600, &mut builder).expect("right");
    let joined = left.join(right, &mut builder).expect("extended join");
    let range = joined.resolve_with(builder.resolver());

    assert_eq!(builder.len(), 1);
    assert_eq!(range.start(), 0);
    assert_eq!(range.end(), 1100);
}

#[test]
fn join_of_adjacent_spans_is_contiguous_cover() {
    let mut builder = ExtendedSpanBuilder::new();
    let left = LocalSpan::exact(0, 5, &mut builder).expect("left");
    let right = LocalSpan::exact(5, 5, &mut builder).expect("right");
    let joined = left.join(right, &mut builder).expect("adjacent join");
    let range = joined.resolve_with(builder.resolver());

    assert_eq!(range.start(), 0);
    assert_eq!(range.end(), 10);
}

#[test]
fn join_of_overlapping_spans_is_smallest_cover() {
    let mut builder = ExtendedSpanBuilder::new();
    let left = LocalSpan::exact(0, 8, &mut builder).expect("left");
    let right = LocalSpan::exact(4, 8, &mut builder).expect("right");
    let joined = left.join(right, &mut builder).expect("overlapping join");
    let range = joined.resolve_with(builder.resolver());

    assert_eq!(range.start(), 0);
    assert_eq!(range.end(), 12);
}

#[test]
fn cross_source_join_is_rejected() {
    let mut builder = ExtendedSpanBuilder::new();
    let left_local = LocalSpan::exact(0, 4, &mut builder).expect("left");
    let right_local = LocalSpan::exact(0, 4, &mut builder).expect("right");
    let left = SourceSpan::new(SourceId::from_index(1), left_local);
    let right = SourceSpan::new(SourceId::from_index(2), right_local);

    match left.join(right, &mut builder) {
        Err(SpanJoinError::DifferentSources {
            left: left_source,
            right: right_source,
        }) => {
            assert_eq!(left_source, SourceId::from_index(1));
            assert_eq!(right_source, SourceId::from_index(2));
        }
        other => panic!("expected a different-sources join error, got {other:?}"),
    }
}

/// A same-source join keeps the identity and covers both operands, extended rows included.
#[test]
fn same_source_join_keeps_the_identity_and_covers_an_extended_operand() {
    let mut builder = ExtendedSpanBuilder::new();
    let source = SourceId::from_index(3);
    let inline = SourceSpan::new(
        source,
        LocalSpan::exact(10, 4, &mut builder).expect("inline"),
    );
    let extended = SourceSpan::new(
        source,
        LocalSpan::exact(9_000, 5_000, &mut builder).expect("extended"),
    );

    let joined = extended
        .join(inline, &mut builder)
        .expect("same-source join");
    let resolver = builder.resolver_for(source);
    let range = joined.resolve_with(resolver);

    assert_eq!(joined.source(), source);
    assert_eq!((range.start(), range.end()), (10, 14_000));
    assert!(joined.contains_with(inline, resolver));
    assert!(joined.contains_with(extended, resolver));
}

#[test]
fn cross_source_overlap_and_containment_are_false_when_byte_ranges_coincide() {
    let mut builder = ExtendedSpanBuilder::new();
    let local = LocalSpan::exact(0, 10, &mut builder).expect("shared byte range");
    let left = SourceSpan::new(SourceId::from_index(1), local);
    let right = SourceSpan::new(SourceId::from_index(2), local);
    let resolver = builder.resolver();

    assert!(!left.overlaps_with(right, resolver));
    assert!(!left.contains_with(right, resolver));
    assert!(!right.contains_with(left, resolver));
}

/// A global span must never resolve through another source's extended table.
///
/// The foreign table here is deliberately long enough to serve the span's extended index: the
/// rejection must come from the source-identity check, not from an index-bounds accident. The
/// wrong pairing is a compiler bug even when the row it would return happens to exist.
#[test]
fn source_span_rejects_resolution_through_another_sources_resolver() {
    let mut first_builder = ExtendedSpanBuilder::new();
    let first_span = SourceSpan::new(
        SourceId::from_index(1),
        LocalSpan::exact(5_000, 2_000, &mut first_builder).expect("first extended span"),
    );

    let mut second_builder = ExtendedSpanBuilder::new();
    let _second_row = LocalSpan::exact(7, 3, &mut second_builder).expect("second source's own row");
    let foreign_resolver = second_builder.resolver_for(SourceId::from_index(2));
    let bare_resolver = second_builder.resolver();

    let foreign_panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        first_span.resolve_with(foreign_resolver);
    }))
    .expect_err("a global span must not resolve through another source's resolver");
    let foreign_message = compiler_bug_panic_message(foreign_panic);
    assert!(
        foreign_message.contains(&SourceId::from_index(1).index().to_string())
            && foreign_message.contains(&SourceId::from_index(2).index().to_string())
            && foreign_message.contains("compiler bug"),
        "the wrong-source rejection should name both identities as a compiler bug: {foreign_message}"
    );

    let bare_panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        first_span.resolve_with(bare_resolver);
    }))
    .expect_err("a global span must not resolve through an unqualified resolver");
    let bare_message = compiler_bug_panic_message(bare_panic);
    assert!(
        bare_message.contains("unqualified") && bare_message.contains("compiler bug"),
        "the unqualified rejection should require resolver_for(source): {bare_message}"
    );

    let own_range = first_span.resolve_with(first_builder.resolver_for(SourceId::from_index(1)));
    assert_eq!(
        (own_range.start(), own_range.end()),
        (5_000, 7_000),
        "the same-source qualified resolver must keep resolving its own table"
    );

    let empty_builder = ExtendedSpanBuilder::new();
    let second = SourceSpan::new(SourceId::from_index(2), LocalSpan::source_start());
    let empty_foreign = empty_builder.resolver_for(SourceId::from_index(3));
    assert!(
        !first_span.overlaps_with(second, empty_foreign),
        "cross-source operands still answer false without resolving"
    );
    assert!(
        !first_span.contains_with(second, empty_foreign),
        "cross-source operands still answer false without resolving"
    );
    assert_eq!(
        first_span.source_order_with(second, empty_foreign),
        Ordering::Less,
        "cross-source operands still order by identity without resolving"
    );
}

#[test]
fn source_order_sorts_by_source_then_start_then_end() {
    let mut builder = ExtendedSpanBuilder::new();
    let early_short = LocalSpan::exact(0, 5, &mut builder).expect("early short");
    let early_long = LocalSpan::exact(0, 10, &mut builder).expect("early long");
    let later = LocalSpan::exact(10, 10, &mut builder).expect("later");
    let other_source = LocalSpan::exact(0, 1, &mut builder).expect("other source");

    let mut spans = [
        SourceSpan::new(SourceId::from_index(2), other_source),
        SourceSpan::new(SourceId::from_index(1), later),
        SourceSpan::new(SourceId::from_index(1), early_long),
        SourceSpan::new(SourceId::from_index(1), early_short),
    ];
    let first = builder.resolver_for(SourceId::from_index(1));
    let second = builder.resolver_for(SourceId::from_index(2));
    spans.sort_by(|left, right| left.source_order_with(*right, first));

    let resolve = |span: SourceSpan| {
        if span.source() == SourceId::from_index(1) {
            span.resolve_with(first)
        } else {
            span.resolve_with(second)
        }
    };

    assert_eq!(
        spans.map(|span| (span.source(), resolve(span))),
        [
            (SourceId::from_index(1), early_short.resolve_with(first)),
            (SourceId::from_index(1), early_long.resolve_with(first)),
            (SourceId::from_index(1), later.resolve_with(first)),
            (SourceId::from_index(2), other_source.resolve_with(second)),
        ]
    );
}

#[test]
fn spans_in_one_source_overlap_only_where_they_share_a_byte() {
    let mut builder = ExtendedSpanBuilder::new();
    let source = SourceId::from_index(1);
    let span = |start, length, builder: &mut ExtendedSpanBuilder| {
        SourceSpan::new(
            source,
            LocalSpan::exact(start, length, builder).expect("test span"),
        )
    };

    let first = span(0, 10, &mut builder);
    let straddling = span(5, 10, &mut builder);
    let adjacent = span(10, 5, &mut builder);
    let nested = span(2, 3, &mut builder);
    let resolver = builder.resolver_for(source);

    assert!(first.overlaps_with(straddling, resolver));
    assert!(straddling.overlaps_with(first, resolver));
    assert!(first.overlaps_with(nested, resolver));
    assert!(first.overlaps_with(first, resolver));
    assert!(
        !first.overlaps_with(adjacent, resolver),
        "a range ending where the next begins shares no byte with it"
    );
}

/// An insertion point sits inside a range without sharing a byte with it.
///
/// Overlap is the non-emptiness of the intersection, so an empty span overlaps nothing at all.
/// Containment is the question consumers actually ask of an insertion point, and it answers yes.
#[test]
fn an_empty_span_is_contained_but_overlaps_nothing() {
    let mut builder = ExtendedSpanBuilder::new();
    let source = SourceId::from_index(1);
    let range = SourceSpan::new(
        source,
        LocalSpan::exact(0, 10, &mut builder).expect("range"),
    );
    let interior = SourceSpan::new(
        source,
        LocalSpan::insertion_point(5, &mut builder).expect("interior insertion point"),
    );
    let on_start = SourceSpan::new(
        source,
        LocalSpan::insertion_point(0, &mut builder).expect("insertion point on the start"),
    );
    let on_end = SourceSpan::new(
        source,
        LocalSpan::insertion_point(10, &mut builder).expect("insertion point on the end"),
    );
    let resolver = builder.resolver_for(source);

    for point in [interior, on_start, on_end] {
        assert!(!range.overlaps_with(point, resolver));
        assert!(!point.overlaps_with(range, resolver));
        assert!(range.contains_with(point, resolver));
    }

    assert!(!interior.overlaps_with(interior, resolver));
    assert!(!range.is_empty_with(resolver));
    assert!(interior.is_empty_with(resolver));
}

#[test]
fn containment_is_directional_and_reflexive_within_one_source() {
    let mut builder = ExtendedSpanBuilder::new();
    let source = SourceId::from_index(1);
    let outer = SourceSpan::new(
        source,
        LocalSpan::exact(0, 10, &mut builder).expect("outer"),
    );
    let inner = SourceSpan::new(source, LocalSpan::exact(2, 3, &mut builder).expect("inner"));
    let straddling = SourceSpan::new(
        source,
        LocalSpan::exact(5, 10, &mut builder).expect("straddling"),
    );
    let resolver = builder.resolver_for(source);

    assert!(outer.contains_with(inner, resolver));
    assert!(!inner.contains_with(outer, resolver));
    assert!(outer.contains_with(outer, resolver));
    assert!(!outer.contains_with(straddling, resolver));
}

/// The cover is the minimum start and the maximum end, whichever operand each comes from.
///
/// A cover that still fits inline must not consume an extended row.
#[test]
fn join_covers_both_operands_whatever_order_they_arrive_in() {
    let mut builder = ExtendedSpanBuilder::new();
    let early = LocalSpan::exact(10, 5, &mut builder).expect("early");
    let late = LocalSpan::exact(40, 6, &mut builder).expect("late");

    let forward = early.join(late, &mut builder).expect("forward join");
    let reversed = late.join(early, &mut builder).expect("reversed join");
    let resolver = builder.resolver();

    assert!(builder.is_empty());
    assert_eq!(
        forward.resolve_with(resolver),
        reversed.resolve_with(resolver)
    );
    assert_eq!(
        (
            forward.resolve_with(resolver).start(),
            forward.resolve_with(resolver).end()
        ),
        (10, 46)
    );
}

/// Joining a nested span must not widen the outer one.
#[test]
fn join_of_a_nested_span_returns_the_outer_cover() {
    let mut builder = ExtendedSpanBuilder::new();
    let outer = LocalSpan::exact(10, 20, &mut builder).expect("outer");
    let inner = LocalSpan::exact(15, 2, &mut builder).expect("inner");

    let joined = outer.join(inner, &mut builder).expect("nested join");
    let range = joined.resolve_with(builder.resolver());

    assert_eq!((range.start(), range.end()), (10, 30));
}

/// Each extended row must be reachable through its own span.
///
/// A codec that appended correctly but always encoded index zero would resolve every extended
/// span to the first row, which no boundary or capacity test can see.
#[test]
fn distinct_extended_rows_resolve_through_their_own_spans() {
    let mut builder = ExtendedSpanBuilder::new();
    let ranges = [(0_u32, 1023_u32), (7, 5000), (4_194_304, 0), (12, 1023)];
    let spans = ranges.map(|(start, length)| {
        LocalSpan::exact(start, length, &mut builder).expect("extended span")
    });

    assert_eq!(
        builder.len(),
        ranges.len(),
        "every range must take its own row"
    );

    let resolver = builder.resolver();
    let resolved = spans.map(|span| {
        let range = span.resolve_with(resolver);
        (range.start(), range.end() - range.start())
    });

    assert_eq!(resolved, ranges);
}

fn next_generated_span_value(state: &mut u32) -> u32 {
    *state ^= *state << 13;
    *state ^= *state >> 17;
    *state ^= *state << 5;
    *state
}

#[test]
fn generated_spans_round_trip_through_live_and_frozen_resolvers() {
    use super::span_encoding::{
        DecodedSpan, INLINE_MAX_LENGTH, INLINE_START_LIMIT, decode_logical,
    };

    let boundary_starts = [
        0,
        1,
        INLINE_START_LIMIT - 1,
        INLINE_START_LIMIT,
        INLINE_START_LIMIT + 1,
    ];
    let boundary_lengths = [
        0,
        1,
        INLINE_MAX_LENGTH - 1,
        INLINE_MAX_LENGTH,
        INLINE_MAX_LENGTH + 1,
    ];
    let mut ranges = Vec::new();

    for start in boundary_starts {
        for length in boundary_lengths {
            ranges.push((start, length));
        }
    }
    for start in [u32::MAX - 1, u32::MAX] {
        ranges.push((start, 0));
    }

    let mut generator_state = 0x9e37_79b9;
    let extended_start_width = u64::from(u32::MAX - INLINE_START_LIMIT) + 1;

    for sample in 0..2_048 {
        let raw_start = next_generated_span_value(&mut generator_state);
        let start = match sample % 4 {
            0 => raw_start % INLINE_START_LIMIT,
            1 => {
                (u64::from(INLINE_START_LIMIT) + u64::from(raw_start) % extended_start_width) as u32
            }
            2 => raw_start,
            _ => u32::MAX - raw_start % 65_536,
        };
        let maximum_length = u64::from(u32::MAX - start);
        let raw_length = next_generated_span_value(&mut generator_state);
        let length = match sample % 8 {
            0 => raw_length % (INLINE_MAX_LENGTH + 1),
            1 if maximum_length >= u64::from(INLINE_MAX_LENGTH + 1) => {
                let minimum_extended_length = u64::from(INLINE_MAX_LENGTH + 1);
                (minimum_extended_length
                    + u64::from(raw_length) % (maximum_length - minimum_extended_length + 1))
                    as u32
            }
            _ => (u64::from(raw_length) % (maximum_length + 1)) as u32,
        };
        ranges.push((start, length));
    }

    let mut builder = ExtendedSpanBuilder::new();
    let mut spans = Vec::with_capacity(ranges.len());
    let mut inline_count = 0usize;
    let mut extended_count = 0usize;

    for &(start, length) in &ranges {
        let span = LocalSpan::exact(start, length, &mut builder)
            .expect("generated range must have a representable end");
        let expected_inline = start < INLINE_START_LIMIT && length <= INLINE_MAX_LENGTH;
        let actual_inline = matches!(
            decode_logical(span.logical_word()),
            DecodedSpan::Inline { .. }
        );

        assert_eq!(
            actual_inline,
            expected_inline,
            "range {start}..{} selected the wrong encoding regime",
            start + length
        );
        if actual_inline {
            inline_count += 1;
        } else {
            extended_count += 1;
        }
        spans.push(span);
    }

    assert!(
        inline_count >= 100 && extended_count >= 100,
        "generated sweep must exercise both regimes (inline={inline_count}, extended={extended_count})"
    );

    let live_ranges: Vec<_> = spans
        .iter()
        .map(|&span| span.resolve_with(builder.resolver()))
        .collect();
    let frozen_table = builder.freeze();

    for ((start, length), (span, live_range)) in ranges
        .iter()
        .copied()
        .zip(spans.iter().copied().zip(live_ranges))
    {
        let frozen_range = span.resolve_with(frozen_table.resolver());
        let expected_end = start + length;

        assert_eq!(
            (live_range.start(), live_range.end()),
            (start, expected_end),
            "live resolver changed generated range {start}..{expected_end}"
        );
        assert_eq!(
            (frozen_range.start(), frozen_range.end()),
            (start, expected_end),
            "frozen resolver changed generated range {start}..{expected_end}"
        );
        assert_eq!(
            live_range, frozen_range,
            "live and frozen resolvers disagree for generated range {start}..{expected_end}"
        );
    }
}

/// The `Option<LocalSpan>` niche depends on the codec never producing the all-ones logical word.
///
/// Nothing at the `LocalSpan` level can observe that word: `store_logical` panics before a span
/// exists, so the invariant lives in the codec's admissible domain. The inline extreme is a
/// compile-time assertion in `span_encoding`; this pins the extended bound and the reason for it.
#[test]
fn the_first_rejected_extended_index_is_the_one_that_would_reserve_the_logical_word() {
    use super::span_encoding::{
        DecodedSpan, LENGTH_BITS, LENGTH_SENTINEL, MAX_EXTENDED_INDEX, decode_logical,
        encode_extended_index,
    };

    let last_usable =
        encode_extended_index(MAX_EXTENDED_INDEX).expect("the last usable index must encode");

    assert!(last_usable < u32::MAX);
    assert!(matches!(
        decode_logical(last_usable),
        DecodedSpan::Extended { index } if index == MAX_EXTENDED_INDEX
    ));

    assert_eq!(
        encode_extended_index(MAX_EXTENDED_INDEX + 1),
        None,
        "the index past the last usable one must leave the codec's domain"
    );
    assert_eq!(
        ((MAX_EXTENDED_INDEX + 1) << LENGTH_BITS) | LENGTH_SENTINEL,
        u32::MAX,
        "that index is rejected because its encoding would be the reserved word"
    );
}

#[test]
fn exact_reports_end_overflow_at_u32_boundary_without_wrapping() {
    use super::span_encoding::{
        DecodedSpan, INLINE_MAX_LENGTH, INLINE_START_LIMIT, decode_logical,
    };

    let mut builder = ExtendedSpanBuilder::new();
    let inline = LocalSpan::exact(INLINE_START_LIMIT - 1, INLINE_MAX_LENGTH, &mut builder)
        .expect("the largest inline range should be representable");
    assert!(matches!(
        decode_logical(inline.logical_word()),
        DecodedSpan::Inline { .. }
    ));

    let extended = LocalSpan::exact(INLINE_START_LIMIT - 1, INLINE_MAX_LENGTH + 1, &mut builder)
        .expect("one past the inline length limit should spill to an extended row");
    assert!(matches!(
        decode_logical(extended.logical_word()),
        DecodedSpan::Extended { .. }
    ));

    // Inline starts and lengths are too small to overflow u32; the end boundary itself is
    // therefore exercised by extended ranges, while the preceding cases prove both exact paths.
    for start in [INLINE_START_LIMIT - 1, INLINE_START_LIMIT, u32::MAX - 1] {
        let maximum_length = u32::MAX - start;
        let span = LocalSpan::exact(start, maximum_length, &mut builder)
            .expect("a range ending at u32::MAX should be representable");
        let resolved = span.resolve_with(builder.resolver());
        assert_eq!((resolved.start(), resolved.end()), (start, u32::MAX));

        let error = LocalSpan::exact(start, maximum_length + 1, &mut builder)
            .expect_err("a range ending past u32::MAX must be rejected");
        assert_eq!(error.start(), start);
        assert_eq!(error.length(), maximum_length + 1);
        assert_eq!(error.reason(), SpanCapacityReason::EndUnrepresentable);
    }
}

#[derive(Clone, Copy)]
struct ReferenceLineRange {
    start: usize,
    visible_end: usize,
    end: usize,
}

fn reference_line_ranges(source: &str) -> Vec<ReferenceLineRange> {
    let mut ranges = Vec::new();
    let mut line_start = 0usize;
    let mut char_indices = source.char_indices().peekable();

    while let Some((offset, scalar)) = char_indices.next() {
        let Some(end) = (match scalar {
            '\n' => Some(offset + 1),
            '\r' => {
                if let Some(&(next_offset, '\n')) = char_indices.peek() {
                    char_indices.next();
                    Some(next_offset + 1)
                } else {
                    Some(offset + 1)
                }
            }
            _ => None,
        }) else {
            continue;
        };

        ranges.push(ReferenceLineRange {
            start: line_start,
            visible_end: offset,
            end,
        });
        line_start = end;
    }

    if ranges.is_empty() || line_start < source.len() {
        ranges.push(ReferenceLineRange {
            start: line_start,
            visible_end: source.len(),
            end: source.len(),
        });
    }

    ranges
}

fn reference_position<'a>(
    source: &'a str,
    ranges: &[ReferenceLineRange],
    offset: usize,
) -> (u32, u32, u32, &'a str) {
    let (line_number, line_range) = ranges
        .iter()
        .enumerate()
        .find(|(index, line_range)| {
            offset < line_range.end || (*index + 1 == ranges.len() && offset == line_range.end)
        })
        .expect("every source boundary must belong to one authored line");
    let line_text = &source[line_range.start..line_range.visible_end];
    let authored_offset = offset.saturating_sub(line_range.start).min(line_text.len());
    let mut scalar_column = 0u32;
    let mut utf16_column = 0u32;

    for (byte_offset, scalar) in line_text.char_indices() {
        if byte_offset >= authored_offset {
            break;
        }
        scalar_column += 1;
        utf16_column += scalar.len_utf16() as u32;
    }

    (line_number as u32, scalar_column, utf16_column, line_text)
}

#[test]
fn line_index_matches_an_independent_unicode_and_newline_oracle_at_every_boundary() {
    let source = "ascii é€😀\n\r\n\r\n\nx\rfinal é€😀";
    let reference_ranges = reference_line_ranges(source);
    let line_starts = line_start_offsets(source);
    let line_index = LineIndex::new(source, &line_starts);

    assert_eq!(
        reference_ranges.len(),
        6,
        "the authored fixture must retain its three empty terminator-only lines"
    );
    assert_eq!(line_index.line_count() as usize, reference_ranges.len());

    let offsets = source
        .char_indices()
        .map(|(offset, _)| offset)
        .chain(std::iter::once(source.len()));
    for offset in offsets {
        let (line, scalar_column, utf16_column, line_text) =
            reference_position(source, &reference_ranges, offset);

        assert_eq!(
            line_index.position(offset as u32),
            Some(LinePosition {
                line,
                column: scalar_column,
            }),
            "scalar position disagreed at source byte {offset}"
        );
        assert_eq!(
            line_index.line_of_offset(offset as u32),
            Some(line),
            "line lookup disagreed at source byte {offset}"
        );
        assert_eq!(
            line_index.utf16_column(offset as u32),
            Some(utf16_column),
            "UTF-16 position disagreed at source byte {offset}"
        );
        assert_eq!(
            line_index.line_text(line),
            Some(line_text),
            "visible line text disagreed at source byte {offset}"
        );
    }
}
