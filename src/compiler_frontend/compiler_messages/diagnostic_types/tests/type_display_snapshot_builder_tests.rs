//! Builder tests for the frozen type-display snapshot.
//!
//! WHAT: proves that the builder returns stable display ids, stays finite on recursive
//! definitions and distinguishes unknown children from unresolved roots.
//! WHY: diagnostics preserve `TypeId(N)` for unresolved roots while unknown nested types remain
//! visibly unknown; recursive types must not hang rendering.
//! OWNERSHIP: tests only. Each test builds its own environment, string table and path table.

use super::{choice_definition, empty_path_table, struct_definition, test_path};
use crate::compiler_frontend::compiler_messages::diagnostic_types::type_display_snapshot_builder::TypeDisplaySnapshotBuilder;
use crate::compiler_frontend::datatypes::definitions::{
    ChoiceVariantDefinition, ChoiceVariantPayloadDefinition, FieldDefinition,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::{GenericParameterId, TypeId};
use crate::compiler_frontend::symbols::path_interner::PathInternerBuilder;
use crate::compiler_frontend::symbols::string_interning::StringTable;

#[test]
fn builder_returns_one_display_id_per_type_id() {
    let environment = TypeEnvironment::new();
    let string_type = environment.builtins().string;
    let mut builder = TypeDisplaySnapshotBuilder::new(Some(&environment));
    let first = builder.display_id(string_type).expect("test snapshot fits");
    let second = builder.display_id(string_type).expect("test snapshot fits");
    let snapshot = builder.finish();
    assert_eq!(first, second);
    assert_eq!(snapshot.records.len(), 1);
}

#[test]
fn builder_keeps_equal_spellings_from_distinct_type_ids_distinct() {
    let mut strings = StringTable::new();
    let t_name = strings.intern("T");
    let mut environment = TypeEnvironment::new();
    let first_parameter = environment.intern_generic_parameter(GenericParameterId(0), t_name);
    let second_parameter = environment.intern_generic_parameter(GenericParameterId(1), t_name);
    assert_ne!(
        first_parameter, second_parameter,
        "fixture must hold two distinct TypeIds"
    );
    let paths = empty_path_table();
    let mut builder = TypeDisplaySnapshotBuilder::new(Some(&environment));
    let first = builder
        .display_id(first_parameter)
        .expect("test snapshot fits");
    let second = builder
        .display_id(second_parameter)
        .expect("test snapshot fits");
    let snapshot = builder.finish();
    assert_ne!(first, second);
    assert_eq!(snapshot.display(first, &strings, &paths), "T");
    assert_eq!(snapshot.display(second, &strings, &paths), "T");
}

#[test]
fn builder_terminates_on_recursive_struct_field_types() {
    let mut strings = StringTable::new();
    let mut paths = PathInternerBuilder::new();
    let node_path = test_path(&mut paths, &mut strings, "Node");
    let next_path = test_path(&mut paths, &mut strings, "next");
    let mut environment = TypeEnvironment::new();
    let (_, node) = environment.register_nominal_struct(struct_definition(node_path, false));
    environment.update_struct_fields(
        node,
        Box::new([FieldDefinition {
            name: next_path,
            type_id: node,
            span: None,
        }]),
    );
    let table = paths.freeze();
    let mut builder = TypeDisplaySnapshotBuilder::new(Some(&environment));
    let display_id = builder.display_id(node).expect("test snapshot fits");
    let snapshot = builder.finish();
    assert_eq!(snapshot.records.len(), 1);
    assert_eq!(snapshot.display(display_id, &strings, &table), "Node");
}

#[test]
fn builder_terminates_on_recursive_choice_payload_types() {
    let mut strings = StringTable::new();
    let mut paths = PathInternerBuilder::new();
    let status_path = test_path(&mut paths, &mut strings, "Status");
    let field_path = test_path(&mut paths, &mut strings, "next");
    let ready = strings.intern("Ready");
    let mut environment = TypeEnvironment::new();
    let (_, status) =
        environment.register_nominal_choice(choice_definition(status_path, Box::new([])));
    environment.update_choice_variants(
        status,
        Box::new([ChoiceVariantDefinition {
            name: ready,
            tag: 0,
            payload: ChoiceVariantPayloadDefinition::Record {
                fields: Box::new([FieldDefinition {
                    name: field_path,
                    type_id: status,
                    span: None,
                }]),
            },
            span: None,
        }]),
    );
    let table = paths.freeze();
    let mut builder = TypeDisplaySnapshotBuilder::new(Some(&environment));
    let display_id = builder.display_id(status).expect("test snapshot fits");
    let snapshot = builder.finish();
    assert_eq!(snapshot.records.len(), 1);
    assert_eq!(
        snapshot.display(display_id, &strings, &table),
        "Status::{Ready(...)}"
    );
}

#[test]
fn builder_distinguishes_unknown_children_from_unresolved_roots() {
    let strings = StringTable::new();
    let paths = empty_path_table();
    let environment = TypeEnvironment::new();
    let type_id = TypeId(9_999);
    let mut builder = TypeDisplaySnapshotBuilder::new(Some(&environment));
    let unknown_child = builder.display_id(type_id).expect("test snapshot fits");
    let unresolved_root = builder
        .display_top_level(type_id)
        .expect("test snapshot fits");
    assert_eq!(
        builder.display_id(type_id).expect("test snapshot fits"),
        unknown_child
    );
    assert_eq!(
        builder
            .display_top_level(type_id)
            .expect("test snapshot fits"),
        unresolved_root
    );
    assert_ne!(unknown_child, unresolved_root);

    let snapshot = builder.finish();
    assert_eq!(snapshot.records.len(), 2);
    assert_eq!(
        snapshot.display(unknown_child, &strings, &paths),
        "<unknown type>"
    );
    assert_eq!(
        snapshot.display_for_render(unresolved_root, &strings, Some(&paths)),
        "TypeId(9999)"
    );
}
