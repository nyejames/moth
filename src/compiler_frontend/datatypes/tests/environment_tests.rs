//! Unit tests for `TypeEnvironment`.

use crate::compiler_frontend::datatypes::definitions::{
    ChoiceTypeDefinition, ChoiceVariantDefinition, ChoiceVariantPayloadDefinition, FieldDefinition,
    FunctionParameterDefinition, FunctionTypeDefinition, StructTypeDefinition, TypeDefinition,
};
use crate::compiler_frontend::datatypes::display::display_type;
use crate::compiler_frontend::datatypes::environment::{
    TypeEnvironment, TypeEnvironmentRemapCache,
};
use crate::compiler_frontend::datatypes::generic_parameters::{
    GenericParameter, GenericParameterList, TypeParameterId,
};
use crate::compiler_frontend::datatypes::ids::{
    BuiltinTypeConstructor, FunctionTypeKey, GenericInstanceKey, GenericParameterId, NominalTypeId,
    TypeConstructor, TypeId,
};
use crate::compiler_frontend::datatypes::{
    BuiltinScalarReceiver, DataType, ReceiverKey, diagnostic_type_spelling,
};
use crate::compiler_frontend::external_packages::ExternalTypeId;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use rustc_hash::FxHashMap;

fn single_generic_parameter_list(
    name: crate::compiler_frontend::symbols::string_interning::StringId,
) -> GenericParameterList {
    GenericParameterList {
        parameters: vec![GenericParameter {
            id: TypeParameterId(0),
            name,
            location: SourceLocation::default(),
            trait_bounds: Vec::new(),
        }],
    }
}

#[test]
fn builtin_seeding_creates_all_expected_ids() {
    let env = TypeEnvironment::new();
    let builtins = env.builtins();

    assert_eq!(
        env.type_kind(builtins.bool),
        Some(super::super::queries::TypeKind::Builtin)
    );
    assert_eq!(
        env.type_kind(builtins.int),
        Some(super::super::queries::TypeKind::Builtin)
    );
    assert_eq!(
        env.type_kind(builtins.float),
        Some(super::super::queries::TypeKind::Builtin)
    );
    assert_eq!(
        env.type_kind(builtins.decimal),
        Some(super::super::queries::TypeKind::Builtin)
    );
    assert_eq!(
        env.type_kind(builtins.string),
        Some(super::super::queries::TypeKind::Builtin)
    );
    assert_eq!(
        env.type_kind(builtins.char),
        Some(super::super::queries::TypeKind::Builtin)
    );
    assert_eq!(
        env.type_kind(builtins.range),
        Some(super::super::queries::TypeKind::Builtin)
    );
    assert_eq!(
        env.type_kind(builtins.none),
        Some(super::super::queries::TypeKind::Builtin)
    );
}

#[test]
fn fresh_environments_have_independent_ids() {
    let env_a = TypeEnvironment::new();
    let env_b = TypeEnvironment::new();

    // IDs happen to line up because both start from 0, but they belong to
    // different environments. This test documents that equality is only
    // meaningful within one environment.
    assert_eq!(env_a.builtins().int, env_b.builtins().int);
}

#[test]
fn type_id_identity_equality_within_environment() {
    let env = TypeEnvironment::new();
    let int_a = env.builtins().int;
    let int_b = env.builtins().int;

    assert_eq!(int_a, int_b);
}

#[test]
fn constructed_type_interning_reuses_ids() {
    let mut env = TypeEnvironment::new();
    let int = env.builtins().int;

    let collection_a = env.intern_constructed(
        TypeConstructor::Builtin(BuiltinTypeConstructor::Collection {
            fixed_capacity: None,
        }),
        Box::new([int]),
    );
    let collection_b = env.intern_constructed(
        TypeConstructor::Builtin(BuiltinTypeConstructor::Collection {
            fixed_capacity: None,
        }),
        Box::new([int]),
    );

    assert_eq!(
        collection_a, collection_b,
        "same constructed key should reuse TypeId"
    );
}

#[test]
fn distinct_constructed_types_get_distinct_ids() {
    let mut env = TypeEnvironment::new();
    let int = env.builtins().int;
    let string = env.builtins().string;

    let collection_int = env.intern_constructed(
        TypeConstructor::Builtin(BuiltinTypeConstructor::Collection {
            fixed_capacity: None,
        }),
        Box::new([int]),
    );
    let collection_string = env.intern_constructed(
        TypeConstructor::Builtin(BuiltinTypeConstructor::Collection {
            fixed_capacity: None,
        }),
        Box::new([string]),
    );

    assert_ne!(
        collection_int, collection_string,
        "different element types should produce different TypeIds"
    );
}

#[test]
fn result_interning_is_deterministic() {
    let mut env = TypeEnvironment::new();
    let int = env.builtins().int;
    let error_type = env.builtins().string;

    let result_a = env.intern_constructed(
        TypeConstructor::Builtin(BuiltinTypeConstructor::FallibleCarrier),
        Box::new([int, error_type]),
    );
    let result_b = env.intern_constructed(
        TypeConstructor::Builtin(BuiltinTypeConstructor::FallibleCarrier),
        Box::new([int, error_type]),
    );

    assert_eq!(result_a, result_b);
}

#[test]
fn generic_instance_key_equality() {
    let key_a = GenericInstanceKey {
        base: NominalTypeId(0),
        arguments: Box::new([TypeId(1), TypeId(2)]),
    };
    let key_b = GenericInstanceKey {
        base: NominalTypeId(0),
        arguments: Box::new([TypeId(1), TypeId(2)]),
    };
    let key_c = GenericInstanceKey {
        base: NominalTypeId(0),
        arguments: Box::new([TypeId(2), TypeId(1)]),
    };

    assert_eq!(key_a, key_b);
    assert_ne!(key_a, key_c);
}

#[test]
fn display_renders_builtin_names() {
    let env = TypeEnvironment::new();
    let table = StringTable::new();

    assert_eq!(display_type(env.builtins().int, &env, &table), "Int");
    assert_eq!(display_type(env.builtins().float, &env, &table), "Float");
    assert_eq!(display_type(env.builtins().bool, &env, &table), "Bool");
    assert_eq!(display_type(env.builtins().string, &env, &table), "String");
    assert_eq!(display_type(env.builtins().char, &env, &table), "Char");
}

#[test]
fn display_renders_collection_with_braces() {
    let mut env = TypeEnvironment::new();
    let table = StringTable::new();
    let int = env.builtins().int;

    let collection = env.intern_constructed(
        TypeConstructor::Builtin(BuiltinTypeConstructor::Collection {
            fixed_capacity: None,
        }),
        Box::new([int]),
    );

    assert_eq!(display_type(collection, &env, &table), "{Int}");
}

#[test]
fn display_renders_option_with_question_mark() {
    let mut env = TypeEnvironment::new();
    let table = StringTable::new();
    let int = env.builtins().int;

    let option = env.intern_constructed(
        TypeConstructor::Builtin(BuiltinTypeConstructor::Option),
        Box::new([int]),
    );

    assert_eq!(display_type(option, &env, &table), "Int?");
}

#[test]
fn display_renders_internal_result_carrier_as_fallible_signature() {
    let mut env = TypeEnvironment::new();
    let table = StringTable::new();
    let int = env.builtins().int;
    let error_type = env.builtins().string;

    let result = env.intern_constructed(
        TypeConstructor::Builtin(BuiltinTypeConstructor::FallibleCarrier),
        Box::new([int, error_type]),
    );

    assert_eq!(display_type(result, &env, &table), "Int, String!");
}

#[test]
fn display_renders_multi_success_result_carrier_as_fallible_signature() {
    let mut env = TypeEnvironment::new();
    let table = StringTable::new();
    let int = env.builtins().int;
    let string = env.builtins().string;
    let error_type = env.builtins().string;

    let success_tuple = env.intern_tuple(vec![int, string]);
    let result = env.intern_constructed(
        TypeConstructor::Builtin(BuiltinTypeConstructor::FallibleCarrier),
        Box::new([success_tuple, error_type]),
    );

    assert_eq!(display_type(result, &env, &table), "Int, String, String!");
}

#[test]
fn display_renders_zero_success_result_carrier_as_error_signature() {
    let mut env = TypeEnvironment::new();
    let table = StringTable::new();
    let none = env.builtins().none;
    let error_type = env.builtins().string;

    let result = env.intern_constructed(
        TypeConstructor::Builtin(BuiltinTypeConstructor::FallibleCarrier),
        Box::new([none, error_type]),
    );

    assert_eq!(display_type(result, &env, &table), "String!");
}

#[test]
fn display_renders_function_error_return_slot() {
    let mut env = TypeEnvironment::new();
    let table = StringTable::new();
    let int = env.builtins().int;
    let string = env.builtins().string;
    let error_type = env.builtins().string;

    let function = env.intern_function(FunctionTypeKey {
        parameters: Box::new([int]),
        returns: Box::new([string]),
        error_return: Some(error_type),
    });

    assert_eq!(
        display_type(function, &env, &table),
        "Function(Int -> String, String!)"
    );
}

#[test]
fn display_renders_tuple_fields() {
    let mut env = TypeEnvironment::new();
    let table = StringTable::new();
    let int = env.builtins().int;
    let string = env.builtins().string;

    let tuple = env.intern_constructed(
        TypeConstructor::Builtin(BuiltinTypeConstructor::Tuple),
        Box::new([int, string]),
    );

    assert_eq!(display_type(tuple, &env, &table), "(Int, String)");
}

#[test]
fn display_renders_choice_variants() {
    let mut env = TypeEnvironment::new();
    let mut table = StringTable::new();
    let path = InternedPath::from_single_str("Status", &mut table);
    let ready = table.get_or_intern("Ready".to_string());
    let failed = table.get_or_intern("Failed".to_string());

    let (_, type_id) = env.register_nominal_choice(ChoiceTypeDefinition {
        id: NominalTypeId(0),
        path,
        variants: vec![
            ChoiceVariantDefinition {
                name: ready,
                tag: 0,
                payload: ChoiceVariantPayloadDefinition::Unit,
                location: SourceLocation::default(),
            },
            ChoiceVariantDefinition {
                name: failed,
                tag: 1,
                payload: ChoiceVariantPayloadDefinition::Record {
                    fields: Box::new([]),
                },
                location: SourceLocation::default(),
            },
        ]
        .into_boxed_slice(),
        generic_parameters: None,
    });

    assert_eq!(
        display_type(type_id, &env, &table),
        "Status::{Ready, Failed(...)}"
    );
}

#[test]
fn display_renders_generic_parameter_names() {
    let mut env = TypeEnvironment::new();
    let mut table = StringTable::new();
    let name = table.get_or_intern("T".to_string());
    let type_id = env.intern_generic_parameter(GenericParameterId(0), name);

    assert_eq!(display_type(type_id, &env, &table), "T");
}

#[test]
fn nominal_struct_registration_allocates_id() {
    let mut env = TypeEnvironment::new();
    let mut table = StringTable::new();
    let path = InternedPath::from_single_str("Point", &mut table);

    let struct_def = StructTypeDefinition {
        id: NominalTypeId(0), // will be overwritten by register_nominal_struct
        path: path.clone(),
        fields: Box::new([]),
        generic_parameters: None,
        const_record: false,
    };

    let (nominal_id, type_id) = env.register_nominal_struct(struct_def);

    assert_eq!(
        env.type_kind(type_id),
        Some(super::super::queries::TypeKind::Struct)
    );
    assert_eq!(
        env.nominal_path_by_id(nominal_id),
        Some(&path),
        "nominal path should be retrievable"
    );
    assert_eq!(
        env.nominal_id_for_path(&path),
        Some(nominal_id),
        "reverse lookup by path should work"
    );
}

#[test]
fn nominal_choice_registration_allocates_id() {
    let mut env = TypeEnvironment::new();
    let mut table = StringTable::new();
    let path = InternedPath::from_single_str("Status", &mut table);

    let choice_def = ChoiceTypeDefinition {
        id: NominalTypeId(0),
        path: path.clone(),
        variants: Box::new([]),
        generic_parameters: None,
    };

    let (nominal_id, type_id) = env.register_nominal_choice(choice_def);

    assert_eq!(
        env.type_kind(type_id),
        Some(super::super::queries::TypeKind::Choice)
    );
    assert_eq!(env.nominal_path_by_id(nominal_id), Some(&path));
}

#[test]
fn member_definition_queries_return_borrowed_views_and_direct_matches() {
    let mut env = TypeEnvironment::new();
    let mut table = StringTable::new();

    let value_name = InternedPath::from_single_str("value", &mut table);
    let point_path = InternedPath::from_single_str("Point", &mut table);
    let (_, point_type_id) = env.register_nominal_struct(StructTypeDefinition {
        id: NominalTypeId(0),
        path: point_path,
        fields: vec![FieldDefinition {
            name: value_name.clone(),
            type_id: env.builtins().int,
            location: SourceLocation::default(),
        }]
        .into_boxed_slice(),
        generic_parameters: None,
        const_record: false,
    });

    let borrowed_fields: Option<&[FieldDefinition]> = env.fields_for(point_type_id);
    assert_eq!(borrowed_fields.map(|fields| fields.len()), Some(1));

    let value_id = value_name
        .name()
        .expect("single-segment field path should expose a field name");
    let value_field = env
        .field_for(point_type_id, value_id)
        .expect("direct field lookup should find base struct field");
    assert_eq!(value_field.type_id, env.builtins().int);

    let ready_name = table.intern("Ready");
    let status_path = InternedPath::from_single_str("Status", &mut table);
    let (_, status_type_id) = env.register_nominal_choice(ChoiceTypeDefinition {
        id: NominalTypeId(0),
        path: status_path,
        variants: vec![ChoiceVariantDefinition {
            name: ready_name,
            tag: 0,
            payload: ChoiceVariantPayloadDefinition::Unit,
            location: SourceLocation::default(),
        }]
        .into_boxed_slice(),
        generic_parameters: None,
    });

    let borrowed_variants: Option<&[ChoiceVariantDefinition]> = env.variants_for(status_type_id);
    assert_eq!(borrowed_variants.map(|variants| variants.len()), Some(1));

    let ready_variant = env
        .variant_for(status_type_id, ready_name)
        .expect("direct variant lookup should find base choice variant");
    assert_eq!(ready_variant.tag, 0);
}

#[test]
fn generic_member_definition_queries_return_substituted_borrowed_views() {
    let mut env = TypeEnvironment::new();
    let mut table = StringTable::new();

    let box_parameter_name = table.intern("T");
    let box_parameters = single_generic_parameter_list(box_parameter_name);
    let registered_box_parameters =
        env.register_generic_parameter_list(&box_parameters, &Default::default());
    let box_parameter_list = registered_box_parameters.list_id;
    let box_parameter_id = registered_box_parameters.canonical_by_local[&TypeParameterId(0)];
    let box_parameter_type_id = env
        .type_id_for_generic_parameter(box_parameter_id)
        .expect("registered struct parameter should have a TypeId");

    let item_name = InternedPath::from_single_str("item", &mut table);
    let box_path = InternedPath::from_single_str("Box", &mut table);
    let (box_nominal_id, _) = env.register_nominal_struct(StructTypeDefinition {
        id: NominalTypeId(0),
        path: box_path,
        fields: vec![FieldDefinition {
            name: item_name.clone(),
            type_id: box_parameter_type_id,
            location: SourceLocation::default(),
        }]
        .into_boxed_slice(),
        generic_parameters: Some(box_parameter_list),
        const_record: false,
    });

    let box_of_int = env.intern_generic_instance(box_nominal_id, Box::new([env.builtins().int]));
    let borrowed_fields: Option<&[FieldDefinition]> = env.fields_for(box_of_int);
    assert_eq!(borrowed_fields.map(|fields| fields.len()), Some(1));

    let item_id = item_name
        .name()
        .expect("single-segment field path should expose a field name");
    let item_field = env
        .field_for(box_of_int, item_id)
        .expect("direct field lookup should find substituted generic field");
    assert_eq!(item_field.type_id, env.builtins().int);

    let state_parameter_name = table.intern("U");
    let state_parameters = single_generic_parameter_list(state_parameter_name);
    let registered_state_parameters =
        env.register_generic_parameter_list(&state_parameters, &Default::default());
    let state_parameter_list = registered_state_parameters.list_id;
    let state_parameter_id = registered_state_parameters.canonical_by_local[&TypeParameterId(0)];
    let state_parameter_type_id = env
        .type_id_for_generic_parameter(state_parameter_id)
        .expect("registered choice parameter should have a TypeId");

    let full_name = table.intern("Full");
    let state_path = InternedPath::from_single_str("State", &mut table);
    let (state_nominal_id, _) = env.register_nominal_choice(ChoiceTypeDefinition {
        id: NominalTypeId(0),
        path: state_path,
        variants: vec![ChoiceVariantDefinition {
            name: full_name,
            tag: 0,
            payload: ChoiceVariantPayloadDefinition::Record {
                fields: vec![FieldDefinition {
                    name: InternedPath::from_single_str("inner", &mut table),
                    type_id: state_parameter_type_id,
                    location: SourceLocation::default(),
                }]
                .into_boxed_slice(),
            },
            location: SourceLocation::default(),
        }]
        .into_boxed_slice(),
        generic_parameters: Some(state_parameter_list),
    });

    let state_of_string =
        env.intern_generic_instance(state_nominal_id, Box::new([env.builtins().string]));
    let borrowed_variants: Option<&[ChoiceVariantDefinition]> = env.variants_for(state_of_string);
    assert_eq!(borrowed_variants.map(|variants| variants.len()), Some(1));

    let full_variant = env
        .variant_for(state_of_string, full_name)
        .expect("direct variant lookup should find substituted generic variant");
    let ChoiceVariantPayloadDefinition::Record { fields } = &full_variant.payload else {
        panic!("generic choice variant should keep record payload fields");
    };
    assert_eq!(fields[0].type_id, env.builtins().string);
}

#[test]
fn receiver_key_queries_use_type_id_semantics() {
    let mut env = TypeEnvironment::new();
    let mut table = StringTable::new();

    assert_eq!(
        env.receiver_key_for_type_id(env.builtins().int),
        Some(ReceiverKey::BuiltinScalar(BuiltinScalarReceiver::Int))
    );
    assert_eq!(
        env.receiver_key_for_type_id(env.builtins().string),
        Some(ReceiverKey::BuiltinScalar(BuiltinScalarReceiver::String))
    );

    let point_path = InternedPath::from_single_str("Point", &mut table);
    let (_, point_type_id) = env.register_nominal_struct(StructTypeDefinition {
        id: NominalTypeId(0),
        path: point_path.clone(),
        fields: Box::new([]),
        generic_parameters: None,
        const_record: false,
    });
    assert_eq!(
        env.receiver_key_for_type_id(point_type_id),
        Some(ReceiverKey::Struct(point_path))
    );

    let const_config_path = InternedPath::from_single_str("Config", &mut table);
    let (_, const_config_type_id) = env.register_nominal_struct(StructTypeDefinition {
        id: NominalTypeId(0),
        path: const_config_path.clone(),
        fields: Box::new([]),
        generic_parameters: None,
        const_record: true,
    });
    assert_eq!(
        env.receiver_key_for_type_id(const_config_type_id),
        Some(ReceiverKey::Struct(const_config_path)),
        "const records still need the receiver key so member lookup can report the const-call diagnostic"
    );

    let external_type = ExternalTypeId(42);
    let external_type_id = env.intern_external(external_type);
    assert_eq!(
        env.receiver_key_for_type_id(external_type_id),
        Some(ReceiverKey::External(external_type)),
        "external opaque types keep receiver keys for builder-owned external member metadata"
    );

    let box_parameter_name = table.intern("T");
    let box_parameters = single_generic_parameter_list(box_parameter_name);
    let registered_box_parameters =
        env.register_generic_parameter_list(&box_parameters, &Default::default());
    let box_parameter_list = registered_box_parameters.list_id;
    let box_path = InternedPath::from_single_str("Box", &mut table);
    let (box_nominal_id, _) = env.register_nominal_struct(StructTypeDefinition {
        id: NominalTypeId(0),
        path: box_path.clone(),
        fields: Box::new([]),
        generic_parameters: Some(box_parameter_list),
        const_record: false,
    });
    let box_of_int = env.intern_generic_instance(box_nominal_id, Box::new([env.builtins().int]));
    assert_eq!(
        env.receiver_key_for_type_id(box_of_int),
        Some(ReceiverKey::Struct(box_path)),
        "generic instances use their constructor key for receiver-method lookup"
    );
}

#[test]
fn updating_choice_variants_preserves_generic_parameter_list() {
    let mut env = TypeEnvironment::new();
    let mut table = StringTable::new();
    let path = InternedPath::from_single_str("ResultShape", &mut table);
    let parameter_name = table.intern("T");

    let parsed_parameters = single_generic_parameter_list(parameter_name);
    let parameter_list = env
        .register_generic_parameter_list(&parsed_parameters, &Default::default())
        .list_id;

    let (_, choice_type_id) = env.register_nominal_choice(ChoiceTypeDefinition {
        id: NominalTypeId(0),
        path,
        variants: Box::new([]),
        generic_parameters: Some(parameter_list),
    });

    env.update_choice_variants(
        choice_type_id,
        vec![ChoiceVariantDefinition {
            name: table.intern("Empty"),
            tag: 0,
            payload: ChoiceVariantPayloadDefinition::Unit,
            location: SourceLocation::default(),
        }]
        .into_boxed_slice(),
    );

    assert_eq!(
        env.generic_parameter_list_id_for_type(choice_type_id),
        Some(parameter_list)
    );
}

#[test]
fn updating_choice_variants_refreshes_generic_instance_variant_cache() {
    let mut env = TypeEnvironment::new();
    let mut table = StringTable::new();
    let path = InternedPath::from_single_str("State", &mut table);
    let parameter_name = table.intern("T");

    let parsed_parameters = single_generic_parameter_list(parameter_name);
    let registered_parameters =
        env.register_generic_parameter_list(&parsed_parameters, &Default::default());
    let parameter_list = registered_parameters.list_id;
    let parameter_id = registered_parameters.canonical_by_local[&TypeParameterId(0)];
    let parameter_type_id = env
        .type_id_for_generic_parameter(parameter_id)
        .expect("registered generic parameter should have a TypeId");

    let (nominal_id, choice_type_id) = env.register_nominal_choice(ChoiceTypeDefinition {
        id: NominalTypeId(0),
        path,
        variants: Box::new([]),
        generic_parameters: Some(parameter_list),
    });

    let int_type_id = env.builtins().int;
    let instance_type_id = env.intern_generic_instance(nominal_id, Box::new([int_type_id]));

    assert_eq!(
        env.variants_for(instance_type_id)
            .map(|variants| variants.len()),
        Some(0),
        "the pre-update instance starts with the shell's empty variant list"
    );

    env.update_choice_variants(
        choice_type_id,
        vec![
            ChoiceVariantDefinition {
                name: table.intern("Empty"),
                tag: 0,
                payload: ChoiceVariantPayloadDefinition::Unit,
                location: SourceLocation::default(),
            },
            ChoiceVariantDefinition {
                name: table.intern("Full"),
                tag: 1,
                payload: ChoiceVariantPayloadDefinition::Record {
                    fields: vec![FieldDefinition {
                        name: InternedPath::from_single_str("value", &mut table),
                        type_id: parameter_type_id,
                        location: SourceLocation::default(),
                    }]
                    .into_boxed_slice(),
                },
                location: SourceLocation::default(),
            },
        ]
        .into_boxed_slice(),
    );

    let variants = env
        .variants_for(instance_type_id)
        .expect("generic choice instance should expose substituted variants");

    assert_eq!(variants.len(), 2);
    assert!(matches!(
        &variants[0].payload,
        ChoiceVariantPayloadDefinition::Unit
    ));

    let ChoiceVariantPayloadDefinition::Record { fields } = &variants[1].payload else {
        panic!("second variant should keep its payload fields");
    };

    assert_eq!(fields[0].type_id, int_type_id);
}

#[test]
fn updating_struct_fields_refreshes_cached_substituted_generic_instance_views() {
    let mut env = TypeEnvironment::new();
    let mut table = StringTable::new();
    let path = InternedPath::from_single_str("Box", &mut table);
    let parameter_name = table.intern("T");

    let parsed_parameters = single_generic_parameter_list(parameter_name);
    let registered_parameters =
        env.register_generic_parameter_list(&parsed_parameters, &Default::default());
    let parameter_list = registered_parameters.list_id;
    let parameter_id = registered_parameters.canonical_by_local[&TypeParameterId(0)];
    let parameter_type_id = env
        .type_id_for_generic_parameter(parameter_id)
        .expect("registered generic parameter should have a TypeId");

    let (nominal_id, struct_type_id) = env.register_nominal_struct(StructTypeDefinition {
        id: NominalTypeId(0),
        path,
        fields: Box::new([]),
        generic_parameters: Some(parameter_list),
        const_record: false,
    });

    let box_of_parameter = env.intern_generic_instance(nominal_id, Box::new([parameter_type_id]));
    let int_type_id = env.builtins().int;
    let mut mapping = FxHashMap::default();
    mapping.insert(parameter_id, int_type_id);

    let box_of_int = env.substitute_type_id(box_of_parameter, &mapping);
    assert_eq!(
        env.fields_for(box_of_int).map(|fields| fields.len()),
        Some(0),
        "the pre-update generic instance starts with the shell's empty field list"
    );

    env.update_struct_fields(
        struct_type_id,
        vec![FieldDefinition {
            name: InternedPath::from_single_str("value", &mut table),
            type_id: parameter_type_id,
            location: SourceLocation::default(),
        }]
        .into_boxed_slice(),
    );

    let box_of_int_after_patch = env.substitute_type_id(box_of_parameter, &mapping);
    assert_eq!(
        box_of_int_after_patch, box_of_int,
        "nominal patching should preserve the canonical generic instance TypeId"
    );

    let fields = env
        .fields_for(box_of_int_after_patch)
        .expect("updated generic struct instance should expose substituted fields");

    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0].type_id, int_type_id);
}

#[test]
fn remap_string_ids_updates_definitions_indexes_and_generic_instance_caches() {
    let mut env = TypeEnvironment::new();
    let mut local_table = StringTable::new();

    let source_scope = InternedPath::from_single_str("source.moth", &mut local_table);
    let source_location =
        SourceLocation::new(source_scope.clone(), Default::default(), Default::default());

    let box_path = InternedPath::from_single_str("Box", &mut local_table);
    let box_parameter_name = local_table.intern("T");
    let box_parsed_parameters = single_generic_parameter_list(box_parameter_name);
    let box_registered_parameters =
        env.register_generic_parameter_list(&box_parsed_parameters, &Default::default());
    let box_parameter_list = box_registered_parameters.list_id;
    let box_parameter_id = box_registered_parameters.canonical_by_local[&TypeParameterId(0)];
    let box_parameter_type_id = env
        .type_id_for_generic_parameter(box_parameter_id)
        .expect("registered box parameter should have a TypeId");

    let (box_nominal_id, box_type_id) = env.register_nominal_struct(StructTypeDefinition {
        id: NominalTypeId(0),
        path: box_path,
        fields: vec![FieldDefinition {
            name: InternedPath::from_single_str("value", &mut local_table),
            type_id: box_parameter_type_id,
            location: source_location.clone(),
        }]
        .into_boxed_slice(),
        generic_parameters: Some(box_parameter_list),
        const_record: false,
    });
    let box_of_int = env.intern_generic_instance(box_nominal_id, Box::new([env.builtins().int]));

    let state_path = InternedPath::from_single_str("State", &mut local_table);
    let state_parameter_name = local_table.intern("U");
    let state_parsed_parameters = single_generic_parameter_list(state_parameter_name);
    let state_registered_parameters =
        env.register_generic_parameter_list(&state_parsed_parameters, &Default::default());
    let state_parameter_list = state_registered_parameters.list_id;
    let state_parameter_id = state_registered_parameters.canonical_by_local[&TypeParameterId(0)];
    let state_parameter_type_id = env
        .type_id_for_generic_parameter(state_parameter_id)
        .expect("registered state parameter should have a TypeId");

    let (state_nominal_id, _state_type_id) = env.register_nominal_choice(ChoiceTypeDefinition {
        id: NominalTypeId(0),
        path: state_path,
        variants: vec![ChoiceVariantDefinition {
            name: local_table.intern("Full"),
            tag: 0,
            payload: ChoiceVariantPayloadDefinition::Record {
                fields: vec![FieldDefinition {
                    name: InternedPath::from_single_str("item", &mut local_table),
                    type_id: state_parameter_type_id,
                    location: source_location.clone(),
                }]
                .into_boxed_slice(),
            },
            location: source_location.clone(),
        }]
        .into_boxed_slice(),
        generic_parameters: Some(state_parameter_list),
    });
    let state_of_string =
        env.intern_generic_instance(state_nominal_id, Box::new([env.builtins().string]));

    let named_parameter = local_table.intern("input");
    let function_type_id = env.insert_function_type_for_test(FunctionTypeDefinition {
        parameters: vec![FunctionParameterDefinition {
            name: Some(named_parameter),
            type_id: env.builtins().int,
        }]
        .into_boxed_slice(),
        returns: vec![env.builtins().string].into_boxed_slice(),
        error_return: None,
    });

    let mut merged_table = StringTable::new();
    merged_table.intern("preexisting-name");
    let remap = merged_table.merge_from(&local_table);

    env.remap_string_ids(&remap);

    let remapped_box_path = InternedPath::from_single_str("Box", &mut merged_table);
    assert_eq!(
        env.nominal_id_for_path(&remapped_box_path),
        Some(box_nominal_id)
    );
    assert_eq!(display_type(box_type_id, &env, &merged_table), "Box");
    assert_eq!(display_type(box_of_int, &env, &merged_table), "Box of Int");

    let generic_parameters = env
        .generic_parameters(box_parameter_list)
        .expect("generic parameter list should survive remapping");
    assert_eq!(
        merged_table.resolve(generic_parameters.parameters[0].name),
        "T"
    );

    let Some(TypeDefinition::GenericParameter(parameter)) = env.get(box_parameter_type_id) else {
        panic!("generic parameter TypeId should still point to a generic parameter definition");
    };
    assert_eq!(merged_table.resolve(parameter.name), "T");

    let Some(TypeDefinition::Function(function)) = env.get(function_type_id) else {
        panic!("function TypeId should still point to a function definition");
    };
    assert_eq!(
        function.parameters[0]
            .name
            .map(|name| merged_table.resolve(name)),
        Some("input")
    );

    let fields = env
        .fields_for(box_of_int)
        .expect("generic struct substituted fields should survive remapping");
    assert_eq!(fields[0].name.name_str(&merged_table), Some("value"));
    assert_eq!(
        fields[0].location.scope.name_str(&merged_table),
        Some("source.moth")
    );

    let variants = env
        .variants_for(state_of_string)
        .expect("generic choice substituted variants should survive remapping");
    assert_eq!(merged_table.resolve(variants[0].name), "Full");

    let ChoiceVariantPayloadDefinition::Record { fields } = &variants[0].payload else {
        panic!("choice payload should remain a record");
    };
    assert_eq!(fields[0].name.name_str(&merged_table), Some("item"));
    assert_eq!(
        fields[0].location.scope.name_str(&merged_table),
        Some("source.moth")
    );
}

#[test]
fn struct_and_choice_share_nominal_id_space() {
    let mut env = TypeEnvironment::new();
    let mut table = StringTable::new();

    let struct_path = InternedPath::from_single_str("Point", &mut table);
    let choice_path = InternedPath::from_single_str("Status", &mut table);

    let (struct_nominal, _) = env.register_nominal_struct(StructTypeDefinition {
        id: NominalTypeId(0),
        path: struct_path,
        fields: Box::new([]),
        generic_parameters: None,
        const_record: false,
    });

    let (choice_nominal, _) = env.register_nominal_choice(ChoiceTypeDefinition {
        id: NominalTypeId(0),
        path: choice_path,
        variants: Box::new([]),
        generic_parameters: None,
    });

    assert_ne!(
        struct_nominal, choice_nominal,
        "structs and choices should not share NominalTypeId"
    );
}

#[test]
fn numeric_query_recognizes_numeric_builtins() {
    let env = TypeEnvironment::new();

    assert!(env.is_numeric(env.builtins().int));
    assert!(env.is_numeric(env.builtins().float));
    // Decimal is intentionally inactive in the Alpha surface: it remains seeded in
    // the environment for stable TypeId layout, but it must not be treated as numeric.
    assert!(!env.is_numeric(env.builtins().decimal));
    assert!(!env.is_numeric(env.builtins().bool));
    assert!(!env.is_numeric(env.builtins().string));
}

#[test]
fn collection_element_type_query_works() {
    let mut env = TypeEnvironment::new();
    let int = env.builtins().int;

    let collection = env.intern_constructed(
        TypeConstructor::Builtin(BuiltinTypeConstructor::Collection {
            fixed_capacity: None,
        }),
        Box::new([int]),
    );

    assert_eq!(env.collection_element_type(collection), Some(int));
}

#[test]
fn option_inner_type_query_works() {
    let mut env = TypeEnvironment::new();
    let string = env.builtins().string;

    let option = env.intern_option(string);

    assert_eq!(env.option_inner_type(option), Some(string));
}

#[test]
fn option_interning_is_deterministic() {
    let mut env = TypeEnvironment::new();
    let string = env.builtins().string;

    let option_a = env.intern_option(string);
    let option_b = env.intern_option(string);

    assert_eq!(option_a, option_b);
    assert!(env.is_option(option_a));
}

#[test]
fn fallible_carrier_slots_query_works() {
    let mut env = TypeEnvironment::new();
    let int = env.builtins().int;
    let error_type = env.builtins().string;

    let result = env.intern_fallible_carrier(int, error_type);

    assert_eq!(env.fallible_carrier_slots(result), Some((int, error_type)));
}

#[test]
fn runtime_equality_query_accepts_supported_scalar_types() {
    let env = TypeEnvironment::new();

    assert!(env.supports_runtime_equality(env.builtins().int));
    assert!(env.supports_runtime_equality(env.builtins().float));
    assert!(env.supports_runtime_equality(env.builtins().bool));
    assert!(env.supports_runtime_equality(env.builtins().char));
    assert!(env.supports_runtime_equality(env.builtins().string));
}

#[test]
fn runtime_equality_query_rejects_unsupported_non_choice_types() {
    let mut env = TypeEnvironment::new();
    let mut table = StringTable::new();
    let struct_path = InternedPath::from_single_str("Point", &mut table);
    let int_type_id = env.builtins().int;

    let (_, struct_type_id) = env.register_nominal_struct(StructTypeDefinition {
        id: NominalTypeId(0),
        path: struct_path,
        fields: Box::new([]),
        generic_parameters: None,
        const_record: false,
    });
    let collection_type_id = env.intern_constructed(
        TypeConstructor::Builtin(BuiltinTypeConstructor::Collection {
            fixed_capacity: None,
        }),
        Box::new([int_type_id]),
    );
    let function_type_id = env.intern_function(FunctionTypeKey {
        parameters: Box::new([]),
        returns: Box::new([]),
        error_return: None,
    });
    let external_type_id = env.intern_external(ExternalTypeId(42));

    assert!(!env.supports_runtime_equality(struct_type_id));
    assert!(!env.supports_runtime_equality(collection_type_id));
    assert!(!env.supports_runtime_equality(function_type_id));
    assert!(!env.supports_runtime_equality(external_type_id));
}

#[test]
fn runtime_equality_query_accepts_unit_choices() {
    let mut env = TypeEnvironment::new();
    let mut table = StringTable::new();
    let path = InternedPath::from_single_str("Status", &mut table);

    let (_, choice_type_id) = env.register_nominal_choice(ChoiceTypeDefinition {
        id: NominalTypeId(0),
        path,
        variants: vec![ChoiceVariantDefinition {
            name: table.intern("Ready"),
            tag: 0,
            payload: ChoiceVariantPayloadDefinition::Unit,
            location: SourceLocation::default(),
        }]
        .into_boxed_slice(),
        generic_parameters: None,
    });

    assert!(env.supports_runtime_equality(choice_type_id));
}

#[test]
fn runtime_equality_query_accepts_choice_payloads_when_fields_do() {
    let mut env = TypeEnvironment::new();
    let mut table = StringTable::new();
    let path = InternedPath::from_single_str("Response", &mut table);
    let int_type_id = env.builtins().int;

    let (_, choice_type_id) = env.register_nominal_choice(ChoiceTypeDefinition {
        id: NominalTypeId(0),
        path,
        variants: vec![ChoiceVariantDefinition {
            name: table.intern("Ok"),
            tag: 0,
            payload: ChoiceVariantPayloadDefinition::Record {
                fields: vec![FieldDefinition {
                    name: InternedPath::from_single_str("value", &mut table),
                    type_id: int_type_id,
                    location: SourceLocation::default(),
                }]
                .into_boxed_slice(),
            },
            location: SourceLocation::default(),
        }]
        .into_boxed_slice(),
        generic_parameters: None,
    });

    assert!(env.supports_runtime_equality(choice_type_id));
}

#[test]
fn runtime_equality_query_rejects_choice_payloads_when_fields_do_not() {
    let mut env = TypeEnvironment::new();
    let mut table = StringTable::new();
    let path = InternedPath::from_single_str("Response", &mut table);
    let function_type_id = env.intern_function(FunctionTypeKey {
        parameters: Box::new([]),
        returns: Box::new([]),
        error_return: None,
    });

    let (_, choice_type_id) = env.register_nominal_choice(ChoiceTypeDefinition {
        id: NominalTypeId(0),
        path,
        variants: vec![ChoiceVariantDefinition {
            name: table.intern("Ok"),
            tag: 0,
            payload: ChoiceVariantPayloadDefinition::Record {
                fields: vec![FieldDefinition {
                    name: InternedPath::from_single_str("callback", &mut table),
                    type_id: function_type_id,
                    location: SourceLocation::default(),
                }]
                .into_boxed_slice(),
            },
            location: SourceLocation::default(),
        }]
        .into_boxed_slice(),
        generic_parameters: None,
    });

    assert!(!env.supports_runtime_equality(choice_type_id));
}

#[test]
fn runtime_equality_query_rejects_recursive_choice_payloads() {
    let mut env = TypeEnvironment::new();
    let mut table = StringTable::new();
    let path = InternedPath::from_single_str("Recursive", &mut table);

    let (_, choice_type_id) = env.register_nominal_choice(ChoiceTypeDefinition {
        id: NominalTypeId(0),
        path,
        variants: Box::new([]),
        generic_parameters: None,
    });

    env.update_choice_variants(
        choice_type_id,
        vec![ChoiceVariantDefinition {
            name: table.intern("Next"),
            tag: 0,
            payload: ChoiceVariantPayloadDefinition::Record {
                fields: vec![FieldDefinition {
                    name: InternedPath::from_single_str("next", &mut table),
                    type_id: choice_type_id,
                    location: SourceLocation::default(),
                }]
                .into_boxed_slice(),
            },
            location: SourceLocation::default(),
        }]
        .into_boxed_slice(),
    );

    assert!(!env.supports_runtime_equality(choice_type_id));
}

#[test]
fn external_type_interning_reuses_ids() {
    let mut env = TypeEnvironment::new();
    let external_type = ExternalTypeId(42);

    let first = env.intern_external(external_type);
    let second = env.intern_external(external_type);

    assert_eq!(first, second);
}

#[test]
fn external_type_interning_distinguishes_external_ids() {
    let mut env = TypeEnvironment::new();

    let first = env.intern_external(ExternalTypeId(1));
    let second = env.intern_external(ExternalTypeId(2));

    assert_ne!(first, second);
}

#[test]
fn external_type_id_reconstructs_legacy_data_type() {
    let mut env = TypeEnvironment::new();
    let external_type = ExternalTypeId(7);

    let type_id = env.intern_external(external_type);

    assert_eq!(
        diagnostic_type_spelling(type_id, &env),
        DataType::External {
            type_id: external_type
        }
    );
}

#[test]
fn intern_generic_instance_reuses_ids() {
    let mut env = TypeEnvironment::new();
    let mut table = StringTable::new();

    let box_path = InternedPath::from_single_str("Box", &mut table);
    let (box_nominal, _) = env.register_nominal_struct(StructTypeDefinition {
        id: NominalTypeId(0),
        path: box_path,
        fields: Box::new([]),
        generic_parameters: None,
        const_record: false,
    });

    let int = env.builtins().int;
    let instance_a = env.intern_generic_instance(box_nominal, Box::new([int]));
    let instance_b = env.intern_generic_instance(box_nominal, Box::new([int]));

    assert_eq!(
        instance_a, instance_b,
        "same generic instance key should reuse TypeId"
    );
}

#[test]
fn intern_generic_instance_distinguishes_different_arguments() {
    let mut env = TypeEnvironment::new();
    let mut table = StringTable::new();

    let box_path = InternedPath::from_single_str("Box", &mut table);
    let (box_nominal, _) = env.register_nominal_struct(StructTypeDefinition {
        id: NominalTypeId(0),
        path: box_path,
        fields: Box::new([]),
        generic_parameters: None,
        const_record: false,
    });

    let int = env.builtins().int;
    let string = env.builtins().string;

    let box_of_int = env.intern_generic_instance(box_nominal, Box::new([int]));
    let box_of_string = env.intern_generic_instance(box_nominal, Box::new([string]));

    assert_ne!(box_of_int, box_of_string);
}

#[test]
fn const_record_display_uses_source_visible_terminology() {
    let mut env = TypeEnvironment::new();
    let mut table = StringTable::new();
    let path = InternedPath::from_single_str("Config", &mut table);

    let (_, type_id) = env.register_nominal_struct(StructTypeDefinition {
        id: NominalTypeId(0),
        path,
        fields: Box::new([]),
        generic_parameters: None,
        const_record: true,
    });

    assert_eq!(display_type(type_id, &env, &table), "const record Config");
}

#[test]
fn generic_instance_display_uses_of_syntax() {
    let mut env = TypeEnvironment::new();
    let mut table = StringTable::new();

    let box_path = InternedPath::from_single_str("Box", &mut table);
    let (box_nominal, _) = env.register_nominal_struct(StructTypeDefinition {
        id: NominalTypeId(0),
        path: box_path,
        fields: Box::new([]),
        generic_parameters: None,
        const_record: false,
    });

    let int = env.builtins().int;
    let box_of_int = env.intern_generic_instance(box_nominal, Box::new([int]));

    assert_eq!(display_type(box_of_int, &env, &table), "Box of Int");
}

// -----------------------------------------------------------
//  Fixed-capacity collection tests
// -----------------------------------------------------------

#[test]
fn fixed_capacity_collection_interns_to_distinct_type_id() {
    let mut env = TypeEnvironment::new();
    let int = env.builtins().int;

    let growable = env.intern_collection(int, None);
    let fixed_64 = env.intern_collection(int, Some(64));

    assert_ne!(
        growable, fixed_64,
        "growable and fixed-capacity collections must be distinct types"
    );
}

#[test]
fn same_fixed_capacity_reuses_type_id() {
    let mut env = TypeEnvironment::new();
    let int = env.builtins().int;

    let fixed_a = env.intern_collection(int, Some(64));
    let fixed_b = env.intern_collection(int, Some(64));

    assert_eq!(
        fixed_a, fixed_b,
        "same element + capacity must reuse TypeId"
    );
}

#[test]
fn different_fixed_capacities_are_distinct() {
    let mut env = TypeEnvironment::new();
    let int = env.builtins().int;

    let fixed_32 = env.intern_collection(int, Some(32));
    let fixed_64 = env.intern_collection(int, Some(64));

    assert_ne!(
        fixed_32, fixed_64,
        "different capacities must be distinct types"
    );
}

#[test]
fn collection_shape_queries_work() {
    let mut env = TypeEnvironment::new();
    let int = env.builtins().int;

    let growable = env.intern_collection(int, None);
    let fixed_64 = env.intern_collection(int, Some(64));

    // Growable shape
    let shape = env
        .collection_shape(growable)
        .expect("growable should have a shape");
    assert_eq!(shape.element_type, int);
    assert_eq!(shape.fixed_capacity, None);

    // Fixed shape
    let shape = env
        .collection_shape(fixed_64)
        .expect("fixed should have a shape");
    assert_eq!(shape.element_type, int);
    assert_eq!(shape.fixed_capacity, Some(64));

    // element_type query
    assert_eq!(env.collection_element_type(growable), Some(int));
    assert_eq!(env.collection_element_type(fixed_64), Some(int));

    // fixed_capacity query
    assert_eq!(env.collection_fixed_capacity(growable), None);
    assert_eq!(env.collection_fixed_capacity(fixed_64), Some(64));

    // Non-collection returns None
    assert_eq!(env.collection_shape(int), None);
    assert_eq!(env.collection_element_type(int), None);
    assert_eq!(env.collection_fixed_capacity(int), None);
}

#[test]
fn is_collection_works_for_fixed_and_growable() {
    let mut env = TypeEnvironment::new();
    let int = env.builtins().int;

    let growable = env.intern_collection(int, None);
    let fixed_64 = env.intern_collection(int, Some(64));

    assert!(env.is_collection(growable));
    assert!(env.is_collection(fixed_64));
    assert!(!env.is_collection(int));
}

#[test]
fn display_type_renders_growable_and_fixed_collections() {
    let mut env = TypeEnvironment::new();
    let table = StringTable::new();
    let int = env.builtins().int;

    let growable = env.intern_collection(int, None);
    let fixed_64 = env.intern_collection(int, Some(64));

    assert_eq!(display_type(growable, &env, &table), "{Int}");
    assert_eq!(display_type(fixed_64, &env, &table), "{64 Int}");
}

#[test]
fn display_type_renders_nested_fixed_collections() {
    let mut env = TypeEnvironment::new();
    let table = StringTable::new();
    let int = env.builtins().int;

    let inner_fixed = env.intern_collection(int, Some(8));
    let outer_fixed = env.intern_collection(inner_fixed, Some(4));

    assert_eq!(display_type(outer_fixed, &env, &table), "{4 {8 Int}}");
}

#[test]
fn map_interning_reuses_ids() {
    let mut env = TypeEnvironment::new();
    let string = env.builtins().string;
    let int = env.builtins().int;

    let map_a = env.intern_map(string, int);
    let map_b = env.intern_map(string, int);

    assert_eq!(map_a, map_b, "same map key/value should reuse TypeId");
}

#[test]
fn distinct_map_types_get_distinct_ids() {
    let mut env = TypeEnvironment::new();
    let string = env.builtins().string;
    let int = env.builtins().int;
    let bool = env.builtins().bool;

    let map_string_int = env.intern_map(string, int);
    let map_string_bool = env.intern_map(string, bool);

    assert_ne!(
        map_string_int, map_string_bool,
        "different value types should produce different TypeIds"
    );
}

#[test]
fn map_shape_queries_work() {
    let mut env = TypeEnvironment::new();
    let string = env.builtins().string;
    let int = env.builtins().int;

    let map_type = env.intern_map(string, int);

    assert!(env.is_map_type(map_type));
    assert_eq!(env.map_key_type(map_type), Some(string));
    assert_eq!(env.map_value_type(map_type), Some(int));

    let shape = env.map_shape(map_type).expect("should have a shape");
    assert_eq!(shape.key_type, string);
    assert_eq!(shape.value_type, int);

    // Non-map returns None
    assert!(!env.is_map_type(string));
    assert_eq!(env.map_key_type(string), None);
    assert_eq!(env.map_value_type(string), None);
    assert_eq!(env.map_shape(string), None);
}

#[test]
fn display_renders_map_type() {
    let mut env = TypeEnvironment::new();
    let table = StringTable::new();
    let string = env.builtins().string;
    let int = env.builtins().int;

    let map_type = env.intern_map(string, int);

    assert_eq!(display_type(map_type, &env, &table), "{String = Int}");
}

#[test]
fn display_renders_nested_map_type() {
    let mut env = TypeEnvironment::new();
    let table = StringTable::new();
    let string = env.builtins().string;
    let int = env.builtins().int;

    let inner_map = env.intern_map(string, int);
    let outer_map = env.intern_map(string, inner_map);

    assert_eq!(
        display_type(outer_map, &env, &table),
        "{String = {String = Int}}"
    );
}

#[test]
fn generated_forks_share_inherited_types_and_keep_local_interning_independent() {
    let mut requester = TypeEnvironment::new();
    let inherited_collection = requester.intern_collection(requester.builtins().int, None);

    let preparation = requester.fork_for_generated();
    let mut first = preparation.clone();
    let mut second = preparation;

    assert_eq!(
        first.intern_collection(first.builtins().int, None),
        inherited_collection
    );
    assert_eq!(
        second.intern_collection(second.builtins().int, None),
        inherited_collection
    );

    let first_local = first.intern_option(first.builtins().bool);
    let second_local = second.intern_option(second.builtins().string);

    assert_eq!(first_local, second_local);
    assert_eq!(
        first.option_inner_type(first_local),
        Some(first.builtins().bool)
    );
    assert_eq!(
        second.option_inner_type(second_local),
        Some(second.builtins().string)
    );
}

#[test]
fn generated_forks_remap_inherited_names_across_sibling_and_nested_layers() {
    let mut requester = TypeEnvironment::new();
    let mut local_table = StringTable::new();
    let source_scope = InternedPath::from_single_str("generated_source.moth", &mut local_table);
    let source_location = SourceLocation::new(source_scope, Default::default(), Default::default());

    let point_path = InternedPath::from_single_str("Point", &mut local_table);
    let (point_nominal_id, point_type_id) =
        requester.register_nominal_struct(StructTypeDefinition {
            id: NominalTypeId(0),
            path: point_path,
            fields: vec![FieldDefinition {
                name: InternedPath::from_single_str("value", &mut local_table),
                type_id: requester.builtins().int,
                location: source_location.clone(),
            }]
            .into_boxed_slice(),
            generic_parameters: None,
            const_record: false,
        });
    let point_alias_path = InternedPath::from_single_str("PointAlias", &mut local_table);
    requester
        .register_nominal_path_alias(point_alias_path, point_type_id)
        .expect("point alias should target the registered nominal");

    let state_path = InternedPath::from_single_str("State", &mut local_table);
    let (state_nominal_id, state_type_id) =
        requester.register_nominal_choice(ChoiceTypeDefinition {
            id: NominalTypeId(0),
            path: state_path,
            variants: vec![ChoiceVariantDefinition {
                name: local_table.intern("Ready"),
                tag: 0,
                payload: ChoiceVariantPayloadDefinition::Record {
                    fields: vec![FieldDefinition {
                        name: InternedPath::from_single_str("item", &mut local_table),
                        type_id: requester.builtins().string,
                        location: source_location.clone(),
                    }]
                    .into_boxed_slice(),
                },
                location: source_location,
            }]
            .into_boxed_slice(),
            generic_parameters: None,
        });

    let preparation = requester.fork_for_generated();
    let mut sibling = preparation.clone();
    let mut nested = preparation.fork_for_generated();

    let mut merged_table = StringTable::new();
    merged_table.intern("preexisting-name");
    let remap = merged_table.merge_from(&local_table);
    assert!(
        !remap.is_identity(),
        "fixture must exercise a real ID remap"
    );

    let mut remap_cache = TypeEnvironmentRemapCache::default();
    sibling.remap_string_ids_with_cache(&remap, &mut remap_cache);
    nested.remap_string_ids_with_cache(&remap, &mut remap_cache);

    let remapped_point_path = InternedPath::from_single_str("Point", &mut merged_table);
    let remapped_point_alias_path = InternedPath::from_single_str("PointAlias", &mut merged_table);
    let remapped_state_path = InternedPath::from_single_str("State", &mut merged_table);
    for generated in [&sibling, &nested] {
        assert_eq!(
            generated.nominal_id_for_path(&remapped_point_path),
            Some(point_nominal_id)
        );
        assert_eq!(
            generated.nominal_id_for_path(&remapped_point_alias_path),
            Some(point_nominal_id)
        );
        assert_eq!(
            generated.nominal_id_for_path(&remapped_state_path),
            Some(state_nominal_id)
        );
        assert_eq!(
            display_type(point_type_id, generated, &merged_table),
            "Point"
        );

        let fields = generated
            .fields_for(point_type_id)
            .expect("generated fork should resolve inherited struct fields");
        assert_eq!(fields[0].name.name_str(&merged_table), Some("value"));
        assert_eq!(
            fields[0].location.scope.name_str(&merged_table),
            Some("generated_source.moth")
        );

        let variants = generated
            .variants_for(state_type_id)
            .expect("generated fork should resolve inherited choice variants");
        assert_eq!(merged_table.resolve(variants[0].name), "Ready");
        let ChoiceVariantPayloadDefinition::Record { fields } = &variants[0].payload else {
            panic!("inherited choice payload should remain a record");
        };
        assert_eq!(fields[0].name.name_str(&merged_table), Some("item"));
        assert_eq!(
            fields[0].location.scope.name_str(&merged_table),
            Some("generated_source.moth")
        );
    }
}

#[test]
fn anonymous_const_record_marker_is_interned_once_and_is_not_a_struct() {
    let env = TypeEnvironment::new();
    let table = StringTable::new();

    let marker = env.anonymous_const_record_type();

    assert!(matches!(
        env.get(marker),
        Some(TypeDefinition::AnonymousConstRecordMarker)
    ));
    assert_eq!(env.anonymous_const_record_type(), marker);
    assert_eq!(
        env.type_kind(marker),
        Some(crate::compiler_frontend::datatypes::queries::TypeKind::AnonymousConstRecordMarker)
    );
    assert!(env.fields_for(marker).is_none());
    assert!(env.struct_definition_for(marker).is_none());
    assert!(!env.is_const_record(marker));
    assert_eq!(display_type(marker, &env, &table), "anonymous const record");
}

#[test]
fn generated_fork_shares_the_anonymous_const_record_marker() {
    let env = TypeEnvironment::new();
    let marker = env.anonymous_const_record_type();

    let fork = env.fork_for_generated();
    assert_eq!(fork.anonymous_const_record_type(), marker);
    assert!(matches!(
        fork.get(marker),
        Some(TypeDefinition::AnonymousConstRecordMarker)
    ));
}
