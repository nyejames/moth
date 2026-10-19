//! Spelling tests for the frozen type-display snapshot.
//!
//! WHAT: proves that a snapshot renders each type exactly as the live formatter does, that its
//! string and path remaps follow their owning tables and that arena lengths are checked
//! against `u32`.
//! WHY: diagnostics render from the snapshot after the live environment is gone, so its spellings
//! must never drift from the live formatter (D15 keeps user-visible output byte-identical).
//! OWNERSHIP: tests only. Each test builds its own environment, string table and path table.

use super::{
    assert_snapshot_spellings, build_snapshot, choice_definition, empty_path_table,
    struct_definition, test_path,
};
use crate::compiler_frontend::compiler_messages::diagnostic_types::TypeDisplayCapacityError;
use crate::compiler_frontend::compiler_messages::diagnostic_types::type_display_snapshot::arena_index;
use crate::compiler_frontend::datatypes::definitions::{
    ChoiceVariantDefinition, ChoiceVariantPayloadDefinition,
};
use crate::compiler_frontend::datatypes::display::TypeDisplayView;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::{
    BuiltinTypeConstructor, BuiltinTypeKey, FunctionTypeKey, GenericParameterId, TypeConstructor,
    TypeId,
};
use crate::compiler_frontend::external_packages::ExternalTypeId;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathIdRemap, PathInternerBuilder};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use moth_lexical::numeric::decimal::NumberScale;
use moth_lexical::numeric::fixed_scalar::FixedScalar;

#[test]
fn snapshot_spelling_matches_live_for_every_shape() {
    let mut strings = StringTable::new();
    let mut paths = PathInternerBuilder::new();
    let config = test_path(&mut paths, &mut strings, "Config");
    let settings = test_path(&mut paths, &mut strings, "Settings");
    let status = test_path(&mut paths, &mut strings, "Status");
    let boxed = test_path(&mut paths, &mut strings, "Box");
    let ready = strings.intern("Ready");
    let failed = strings.intern("Failed");
    let t_name = strings.intern("T");

    let mut environment = TypeEnvironment::new();
    let bool_type = environment.builtins().bool;
    let string_type = environment.builtins().string;
    let char_type = environment.builtins().char;
    let range_type = environment.builtins().range;
    let none_type = environment.builtins().none;
    let i32_type = environment.intern_builtin_key(BuiltinTypeKey::FixedScalar(FixedScalar::I32));
    let dec_type = environment.intern_number(NumberScale::ZERO);
    let dec2_type = environment.intern_number(NumberScale::new(2).expect("scale 2 fits"));
    let (_, config_type) = environment.register_nominal_struct(struct_definition(config, false));
    let (_, settings_type) = environment.register_nominal_struct(struct_definition(settings, true));
    let (_, status_type) = environment.register_nominal_choice(choice_definition(
        status,
        Box::new([
            ChoiceVariantDefinition {
                name: ready,
                tag: 0,
                payload: ChoiceVariantPayloadDefinition::Unit,
                span: None,
            },
            ChoiceVariantDefinition {
                name: failed,
                tag: 1,
                payload: ChoiceVariantPayloadDefinition::Record {
                    fields: Box::new([]),
                },
                span: None,
            },
        ]),
    ));
    let (box_base, _) = environment.register_nominal_struct(struct_definition(boxed, false));
    let box_of_i32 = environment.intern_generic_instance(box_base, Box::new([i32_type]));
    let fixed_collection = environment.intern_collection(i32_type, Some(3));
    let open_collection = environment.intern_collection(i32_type, None);
    let map_type = environment.intern_map(string_type, i32_type);
    let option_type = environment.intern_option(i32_type);
    let function_type = environment.intern_function(FunctionTypeKey {
        parameters: Box::new([i32_type]),
        returns: Box::new([bool_type]),
        error_return: None,
    });
    let external_type = environment.intern_external(ExternalTypeId(7));
    let generic_type = environment.intern_generic_parameter(GenericParameterId(0), t_name);
    let anonymous_type = environment.anonymous_const_record_type();

    let cases: Vec<(TypeId, &str)> = vec![
        (bool_type, "Bool"),
        (string_type, "String"),
        (char_type, "Char"),
        (range_type, "Range"),
        (none_type, "None"),
        (i32_type, "I32"),
        (dec_type, "Dec"),
        (dec2_type, "Dec2"),
        (config_type, "Config"),
        (settings_type, "const record Settings"),
        (status_type, "Status::{Ready, Failed(...)}"),
        (box_of_i32, "Box of I32"),
        (fixed_collection, "{3 I32}"),
        (open_collection, "{I32}"),
        (map_type, "{String = I32}"),
        (option_type, "I32?"),
        (function_type, "Function(I32 -> Bool)"),
        (external_type, "External(7)"),
        (generic_type, "T"),
        (anonymous_type, "anonymous const record"),
        (TypeId(9_999), "<unknown type>"),
    ];
    assert_snapshot_spellings(&environment, &strings, paths.paths(), &cases);
}

#[test]
fn snapshot_spelling_matches_live_for_nested_combinations() {
    let mut strings = StringTable::new();
    let mut paths = PathInternerBuilder::new();
    let config = test_path(&mut paths, &mut strings, "Config");
    let boxed = test_path(&mut paths, &mut strings, "Box");

    let mut environment = TypeEnvironment::new();
    let bool_type = environment.builtins().bool;
    let string_type = environment.builtins().string;
    let i32_type = environment.intern_builtin_key(BuiltinTypeKey::FixedScalar(FixedScalar::I32));
    let (_, config_type) = environment.register_nominal_struct(struct_definition(config, false));
    let (box_base, _) = environment.register_nominal_struct(struct_definition(boxed, false));
    let box_of_i32 = environment.intern_generic_instance(box_base, Box::new([i32_type]));
    let box_of_two =
        environment.intern_generic_instance(box_base, Box::new([i32_type, string_type]));

    let inner_function = environment.intern_function(FunctionTypeKey {
        parameters: Box::new([i32_type]),
        returns: Box::new([i32_type]),
        error_return: None,
    });
    let higher_order = environment.intern_function(FunctionTypeKey {
        parameters: Box::new([inner_function, string_type]),
        returns: Box::new([bool_type]),
        error_return: None,
    });
    let fallible = environment.intern_function(FunctionTypeKey {
        parameters: Box::new([i32_type]),
        returns: Box::new([config_type]),
        error_return: Some(string_type),
    });
    let collection = environment.intern_collection(i32_type, Some(3));
    let optional_collection = environment.intern_option(collection);
    let map_of_box = environment.intern_map(string_type, box_of_i32);
    let optional_element = environment.intern_option(i32_type);
    let collection_of_option = environment.intern_collection(optional_element, None);
    let fixed_collection_of_option = environment.intern_collection(optional_element, Some(3));
    let dual_parameter_function = environment.intern_function(FunctionTypeKey {
        parameters: Box::new([box_of_i32, map_of_box]),
        returns: Box::new([i32_type, bool_type]),
        error_return: Some(string_type),
    });

    let cases: Vec<(TypeId, &str)> = vec![
        (
            higher_order,
            "Function(Function(I32 -> I32), String -> Bool)",
        ),
        (fallible, "Function(I32 -> Config, String!)"),
        (optional_collection, "{3 I32}?"),
        (map_of_box, "{String = Box of I32}"),
        (box_of_two, "Box of I32, String"),
        (collection_of_option, "{I32?}"),
        (fixed_collection_of_option, "{3 I32?}"),
        (
            dual_parameter_function,
            "Function(Box of I32, {String = Box of I32} -> I32, Bool, String!)",
        ),
    ];
    assert_snapshot_spellings(&environment, &strings, paths.paths(), &cases);
}

#[test]
fn snapshot_spelling_matches_live_for_callable_result_arities() {
    let strings = StringTable::new();
    let paths = empty_path_table();

    let mut environment = TypeEnvironment::new();
    let bool_type = environment.builtins().bool;
    let string_type = environment.builtins().string;
    let i32_type = environment.intern_builtin_key(BuiltinTypeKey::FixedScalar(FixedScalar::I32));
    let no_results = environment.intern_function(FunctionTypeKey {
        parameters: Box::new([i32_type]),
        returns: Box::new([]),
        error_return: None,
    });
    let several_results = environment.intern_function(FunctionTypeKey {
        parameters: Box::new([i32_type]),
        returns: Box::new([i32_type, bool_type]),
        error_return: None,
    });
    let error_only = environment.intern_function(FunctionTypeKey {
        parameters: Box::new([i32_type]),
        returns: Box::new([]),
        error_return: Some(string_type),
    });
    let several_results_with_error = environment.intern_function(FunctionTypeKey {
        parameters: Box::new([i32_type]),
        returns: Box::new([i32_type, bool_type]),
        error_return: Some(string_type),
    });

    let cases: Vec<(TypeId, &str)> = vec![
        (no_results, "Function(I32 -> )"),
        (several_results, "Function(I32 -> I32, Bool)"),
        (error_only, "Function(I32 -> String!)"),
        (
            several_results_with_error,
            "Function(I32 -> I32, Bool, String!)",
        ),
    ];
    assert_snapshot_spellings(&environment, &strings, &paths, &cases);
}

#[test]
fn snapshot_spelling_matches_live_for_generic_instance_of_choice() {
    let mut strings = StringTable::new();
    let mut paths = PathInternerBuilder::new();
    let outcome = test_path(&mut paths, &mut strings, "Outcome");
    let ready = strings.intern("Ready");
    let failed = strings.intern("Failed");

    let mut environment = TypeEnvironment::new();
    let i32_type = environment.intern_builtin_key(BuiltinTypeKey::FixedScalar(FixedScalar::I32));
    let (outcome_base, _) = environment.register_nominal_choice(choice_definition(
        outcome,
        Box::new([
            ChoiceVariantDefinition {
                name: ready,
                tag: 0,
                payload: ChoiceVariantPayloadDefinition::Unit,
                span: None,
            },
            ChoiceVariantDefinition {
                name: failed,
                tag: 1,
                payload: ChoiceVariantPayloadDefinition::Record {
                    fields: Box::new([]),
                },
                span: None,
            },
        ]),
    ));
    let outcome_of_i32 = environment.intern_generic_instance(outcome_base, Box::new([i32_type]));

    let cases: Vec<(TypeId, &str)> = vec![(outcome_of_i32, "Outcome of I32")];
    assert_snapshot_spellings(&environment, &strings, paths.paths(), &cases);
}

#[test]
fn snapshot_spelling_matches_live_fallbacks_without_path_names() {
    let mut strings = StringTable::new();
    let mut paths = PathInternerBuilder::new();
    let config = test_path(&mut paths, &mut strings, "Config");
    let settings = test_path(&mut paths, &mut strings, "Settings");
    let status = test_path(&mut paths, &mut strings, "Status");
    let boxed = test_path(&mut paths, &mut strings, "Box");
    let ready = strings.intern("Ready");

    let mut environment = TypeEnvironment::new();
    let i32_type = environment.intern_builtin_key(BuiltinTypeKey::FixedScalar(FixedScalar::I32));
    let (_, config_type) = environment.register_nominal_struct(struct_definition(config, false));
    let (_, settings_type) = environment.register_nominal_struct(struct_definition(settings, true));
    let (_, status_type) = environment.register_nominal_choice(choice_definition(
        status,
        Box::new([ChoiceVariantDefinition {
            name: ready,
            tag: 0,
            payload: ChoiceVariantPayloadDefinition::Unit,
            span: None,
        }]),
    ));
    let (box_base, _) = environment.register_nominal_struct(struct_definition(boxed, false));
    let box_of_i32 = environment.intern_generic_instance(box_base, Box::new([i32_type]));
    let bare_map = environment.intern_constructed(
        TypeConstructor::Builtin(BuiltinTypeConstructor::OrderedMap),
        Box::new([]),
    );
    let bare_option = environment.intern_constructed(
        TypeConstructor::Builtin(BuiltinTypeConstructor::Option),
        Box::new([]),
    );
    let bare_collection = environment.intern_constructed(
        TypeConstructor::Builtin(BuiltinTypeConstructor::Collection {
            fixed_capacity: None,
        }),
        Box::new([]),
    );

    let cases: Vec<(TypeId, &str)> = vec![
        (config_type, "<anonymous struct>"),
        (settings_type, "const record <anonymous struct>"),
        (status_type, "<choice>::{Ready}"),
        (box_of_i32, "<generic> of I32"),
        (bare_map, "Map"),
        (bare_option, "Option"),
        (bare_collection, "Collection"),
    ];
    assert_snapshot_spellings(&environment, &strings, &empty_path_table(), &cases);
}

#[test]
fn snapshot_string_remap_rewrites_generic_parameter_and_variant_names() {
    // The root table interns another spelling first, so no source StringId keeps its meaning.
    let mut root_strings = StringTable::new();
    let mut root_paths = PathInternerBuilder::new();
    let _ = root_strings.intern("Padding");
    let status = test_path(&mut root_paths, &mut root_strings, "Status");

    let mut source_strings = StringTable::new();
    let t_name = source_strings.intern("T");
    let ready = source_strings.intern("Ready");

    let mut environment = TypeEnvironment::new();
    let generic_type = environment.intern_generic_parameter(GenericParameterId(0), t_name);
    let (_, status_type) = environment.register_nominal_choice(choice_definition(
        status,
        Box::new([ChoiceVariantDefinition {
            name: ready,
            tag: 0,
            payload: ChoiceVariantPayloadDefinition::Unit,
            span: None,
        }]),
    ));
    let (mut snapshot, display_ids) = build_snapshot(&environment, &[generic_type, status_type]);

    let remap = root_strings.merge_from(&source_strings);
    assert!(!remap.is_identity(), "fixture must move string ids");
    snapshot.remap_string_ids(&remap);

    assert_eq!(
        snapshot.display(display_ids[0], &root_strings, root_paths.paths()),
        "T"
    );
    assert_eq!(
        snapshot.display(display_ids[1], &root_strings, root_paths.paths()),
        "Status::{Ready}"
    );
}

#[test]
fn snapshot_path_remap_rewrites_struct_choice_and_instance_paths() {
    let mut source_strings = StringTable::new();
    let mut source_paths = PathInternerBuilder::new();
    let config = test_path(&mut source_paths, &mut source_strings, "Config");
    let status = test_path(&mut source_paths, &mut source_strings, "Status");
    let boxed = test_path(&mut source_paths, &mut source_strings, "Box");

    let mut environment = TypeEnvironment::new();
    let i32_type = environment.intern_builtin_key(BuiltinTypeKey::FixedScalar(FixedScalar::I32));
    let (_, config_type) = environment.register_nominal_struct(struct_definition(config, false));
    let (_, status_type) =
        environment.register_nominal_choice(choice_definition(status, Box::new([])));
    let (box_base, _) = environment.register_nominal_struct(struct_definition(boxed, false));
    let box_of_i32 = environment.intern_generic_instance(box_base, Box::new([i32_type]));
    let (mut snapshot, display_ids) =
        build_snapshot(&environment, &[config_type, status_type, box_of_i32]);

    // The destination interns the same spellings in another order, so every source index changes meaning.
    let mut dest_strings = StringTable::new();
    let mut dest_paths = PathInternerBuilder::new();
    let dest_box = test_path(&mut dest_paths, &mut dest_strings, "Box");
    let dest_status = test_path(&mut dest_paths, &mut dest_strings, "Status");
    let dest_config = test_path(&mut dest_paths, &mut dest_strings, "Config");

    let mut mapped = vec![PathId::ROOT; source_paths.len()];
    mapped[config.index()] = dest_config;
    mapped[status.index()] = dest_status;
    mapped[boxed.index()] = dest_box;
    let remap = PathIdRemap::from_full(mapped);
    assert!(!remap.is_identity(), "fixture must move path ids");
    snapshot.remap_path_ids(&remap);

    let dest_table = dest_paths.paths();
    assert_eq!(
        snapshot.display(display_ids[0], &dest_strings, dest_table),
        "Config"
    );
    assert_eq!(
        snapshot.display(display_ids[1], &dest_strings, dest_table),
        "Status::{}"
    );
    assert_eq!(
        snapshot.display(display_ids[2], &dest_strings, dest_table),
        "Box of I32"
    );
}

#[test]
fn snapshot_path_remap_rewrites_the_root_path_word() {
    // The root path encodes as data word 0, so the remap must rewrite that word as well.
    let mut environment = TypeEnvironment::new();
    let (_, root_type) =
        environment.register_nominal_struct(struct_definition(PathId::ROOT, false));
    let (mut snapshot, display_ids) = build_snapshot(&environment, &[root_type]);

    let mut dest_strings = StringTable::new();
    let mut dest_paths = PathInternerBuilder::new();
    let named = test_path(&mut dest_paths, &mut dest_strings, "Named");
    // The source table holds only the root, so one mapped entry covers every source path.
    let remap = PathIdRemap::from_full(vec![named]);
    assert!(!remap.is_identity(), "fixture must move the root path");
    snapshot.remap_path_ids(&remap);

    assert_eq!(
        snapshot.display(display_ids[0], &dest_strings, dest_paths.paths()),
        "Named"
    );
}

#[test]
fn snapshot_renders_after_source_environment_is_dropped() {
    let mut strings = StringTable::new();
    let mut paths = PathInternerBuilder::new();
    let config = test_path(&mut paths, &mut strings, "Config");
    let (snapshot, display_id) = {
        let mut environment = TypeEnvironment::new();
        let (_, config_type) =
            environment.register_nominal_struct(struct_definition(config, false));
        let (snapshot, display_ids) = build_snapshot(&environment, &[config_type]);
        (snapshot, display_ids[0])
    };
    assert_eq!(
        snapshot.display(display_id, &strings, paths.paths()),
        "Config"
    );
}

#[test]
fn arena_index_accepts_u32_max_and_rejects_the_next_length() {
    let cases = [
        (u32::MAX as usize, Ok(u32::MAX)),
        (u32::MAX as usize + 1, Err(TypeDisplayCapacityError)),
    ];
    for (length, expected) in cases {
        assert_eq!(arena_index(length), expected, "length {length}");
    }
}

#[test]
fn snapshot_generic_parameter_query_matches_live_view() {
    let mut strings = StringTable::new();
    let t_name = strings.intern("T");

    let mut environment = TypeEnvironment::new();
    let generic_type = environment.intern_generic_parameter(GenericParameterId(0), t_name);
    let i32_type = environment.intern_builtin_key(BuiltinTypeKey::FixedScalar(FixedScalar::I32));
    let option_type = environment.intern_option(generic_type);
    let cases = [
        (generic_type, true),
        (i32_type, false),
        (option_type, false),
        (TypeId(9_999), false),
    ];

    let type_ids: Vec<TypeId> = cases.iter().map(|(type_id, _)| *type_id).collect();
    let (snapshot, display_ids) = build_snapshot(&environment, &type_ids);
    for ((type_id, expected), display_id) in cases.into_iter().zip(display_ids) {
        assert_eq!(
            environment.is_generic_parameter(type_id),
            expected,
            "live {type_id:?}"
        );
        assert_eq!(
            snapshot.is_generic_parameter(display_id),
            expected,
            "snapshot {type_id:?}"
        );
    }
}
