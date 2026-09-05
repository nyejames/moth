use super::{SourceDatabase, SourceId, SourceProvenance};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use std::mem::{align_of, size_of};
use std::path::{Path, PathBuf};

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
}
