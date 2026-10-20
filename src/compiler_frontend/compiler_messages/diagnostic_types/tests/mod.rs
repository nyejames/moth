//! Unit tests for the type-display snapshot store and its builder.
//!
//! WHAT: fixtures shared by the snapshot and builder test modules.
//! WHY: both modules build live environments and compare snapshot spellings with the live formatter.

mod type_display_snapshot_builder_tests;
mod type_display_snapshot_tests;

use crate::compiler_frontend::compiler_messages::diagnostic_types::TypeDisplayId;
use crate::compiler_frontend::compiler_messages::diagnostic_types::type_display_snapshot::TypeDisplaySnapshot;
use crate::compiler_frontend::compiler_messages::diagnostic_types::type_display_snapshot_builder::TypeDisplaySnapshotBuilder;
use crate::compiler_frontend::datatypes::definitions::{
    ChoiceTypeDefinition, ChoiceVariantDefinition, StructTypeDefinition,
};
use crate::compiler_frontend::datatypes::display::display_type;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::{NominalTypeId, TypeId};
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerBuilder, PathTable};
use crate::compiler_frontend::symbols::string_interning::StringTable;

/// Interns one portable path spelling and returns its id.
fn test_path(paths: &mut PathInternerBuilder, strings: &mut StringTable, spelling: &str) -> PathId {
    paths
        .try_intern_portable_path(spelling, strings)
        .expect("test path fits")
}

/// Returns a path table holding only the root, so every path name uses its fallback.
fn empty_path_table() -> PathTable {
    PathInternerBuilder::new().freeze()
}

/// Builds a field-less struct definition. `register_nominal_struct` assigns the real nominal id.
fn struct_definition(path: PathId, const_record: bool) -> StructTypeDefinition {
    StructTypeDefinition {
        id: NominalTypeId(0),
        path,
        fields: Box::new([]),
        generic_parameters: None,
        const_record,
    }
}

/// Builds a choice definition with the given variants. `register_nominal_choice` assigns the id.
fn choice_definition(
    path: PathId,
    variants: Box<[ChoiceVariantDefinition]>,
) -> ChoiceTypeDefinition {
    ChoiceTypeDefinition {
        id: NominalTypeId(0),
        path,
        variants,
        generic_parameters: None,
    }
}

/// Builds one snapshot holding every requested type and returns the display ids in request order.
fn build_snapshot(
    environment: &TypeEnvironment,
    type_ids: &[TypeId],
) -> (TypeDisplaySnapshot, Vec<TypeDisplayId>) {
    let mut builder = TypeDisplaySnapshotBuilder::new(Some(environment));
    let display_ids = type_ids
        .iter()
        .map(|&type_id| builder.display_id(type_id).expect("test snapshot fits"))
        .collect();
    (builder.finish(), display_ids)
}

/// Asserts that the live formatter and the snapshot both render each case as its expected spelling.
fn assert_snapshot_spellings(
    environment: &TypeEnvironment,
    strings: &StringTable,
    paths: &PathTable,
    cases: &[(TypeId, &str)],
) {
    let type_ids: Vec<TypeId> = cases.iter().map(|(type_id, _)| *type_id).collect();
    let (snapshot, display_ids) = build_snapshot(environment, &type_ids);
    for ((type_id, expected), display_id) in cases.iter().zip(display_ids) {
        assert_eq!(
            display_type(*type_id, environment, strings, paths),
            *expected,
            "live spelling of {type_id:?}"
        );
        assert_eq!(
            snapshot.display(display_id, strings, paths),
            *expected,
            "snapshot spelling of {type_id:?}"
        );
    }
}
