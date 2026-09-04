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
fn source_id_round_trips_database_record_indices() {
    let paths = [
        PathBuf::from("/project/00.moth"),
        PathBuf::from("/project/01.moth"),
        PathBuf::from("/project/02.moth"),
        PathBuf::from("/project/03.moth"),
        PathBuf::from("/project/04.moth"),
    ];
    let mut string_table = StringTable::new();
    let database = SourceDatabase::build(
        paths.iter(),
        Path::new("/project/00.moth"),
        None,
        &mut string_table,
    )
    .expect("source identities should build");

    for index in [0, 2, 4] {
        let id = SourceId::from_index(index);
        let record = database.get(id).expect("database record should exist");
        assert_eq!(record.id, id);
        assert_eq!(id.index(), index);
    }
}
