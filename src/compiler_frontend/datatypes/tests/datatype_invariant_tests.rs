//! Legacy tests for the old `DataType` enum.
//!
//! These tests preserve old implementation-detail assertions that still have
//! active callers while the type system migrates to `TypeId + TypeEnvironment`.

use crate::compiler_frontend::datatypes::{DataType, builtin_type_ids};
use crate::compiler_frontend::symbols::path_interner::PathInternerBuilder;
use crate::compiler_frontend::symbols::string_interning::StringTable;

#[test]
fn choice_equality_is_purely_nominal() {
    let mut table = StringTable::new();
    let mut path_builder = PathInternerBuilder::new();
    let path_a = path_builder
        .try_intern_portable_path("Status", &mut table)
        .expect("test path fits");
    let path_b = path_builder
        .try_intern_portable_path("OtherStatus", &mut table)
        .expect("test path fits");

    let status_a = DataType::Choices {
        nominal_path: path_a,
        type_id: builtin_type_ids::NONE,
        generic_instance_key: None,
    };
    let status_b = DataType::Choices {
        nominal_path: path_a,
        type_id: builtin_type_ids::NONE,
        generic_instance_key: None,
    };
    let other = DataType::Choices {
        nominal_path: path_b,
        type_id: builtin_type_ids::NONE,
        generic_instance_key: None,
    };

    assert_eq!(
        status_a, status_b,
        "same nominal path should make choices equal regardless of variant shape"
    );
    assert_ne!(
        status_a, other,
        "different nominal paths should make choices unequal even with identical variants"
    );
}
