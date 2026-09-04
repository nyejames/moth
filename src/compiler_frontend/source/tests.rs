use super::{SourceDatabase, SourceId};
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
        assert_eq!(record.id, SourceId::from_index(index));
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
        .filter(|record| record.canonical_os_path.starts_with("/project/alpha"))
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
    let mut database = SourceDatabase::from_ordered_canonical_files(
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
        .filter(|record| record.canonical_os_path == shared_source)
        .count();

    assert_eq!(module_a_view, module_b_view);
    assert_eq!(shared_record_count, 1);
}
