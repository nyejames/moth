//! Unit tests for generic parameter metadata and TypeId-native generic helpers.

use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::definitions::{StructTypeDefinition, TypeDefinition};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::generic_bindings::GenericTypeBindings;
use crate::compiler_frontend::datatypes::generic_identity_bridge::{
    BuiltinTypeKey, GenericInstantiationKey, TypeIdentityKey,
};
use crate::compiler_frontend::datatypes::generic_parameters::{
    GenericParameter, GenericParameterList, GenericParameterScope, TypeParameterId,
};
use crate::compiler_frontend::datatypes::ids::{
    BuiltinTypeConstructor, FunctionTypeKey, GenericParameterId, GenericParameterListId,
    NominalTypeId, TypeConstructor, TypeId,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::compiler_frontend::traits::ids::TraitId;
use rustc_hash::{FxHashMap, FxHashSet};

fn register_environment_parameter(
    type_environment: &mut TypeEnvironment,
    string_table: &mut StringTable,
    parameter_id: GenericParameterId,
    name: &str,
) -> TypeId {
    type_environment.intern_generic_parameter(parameter_id, string_table.intern(name))
}

fn register_single_parameter_list(
    type_environment: &mut TypeEnvironment,
    string_table: &mut StringTable,
    name: &str,
) -> GenericParameterListId {
    let parameter_list = GenericParameterList {
        parameters: vec![GenericParameter {
            id: TypeParameterId(0),
            name: string_table.intern(name),
            location: SourceLocation::default(),
            trait_bounds: Vec::new(),
        }],
    };

    type_environment
        .register_generic_parameter_list(&parameter_list, &Default::default())
        .list_id
}

fn register_empty_generic_struct(
    type_environment: &mut TypeEnvironment,
    string_table: &mut StringTable,
    name: &str,
    generic_parameters: GenericParameterListId,
) -> NominalTypeId {
    let path = InternedPath::from_single_str(name, string_table);
    let (nominal_id, _) = type_environment.register_nominal_struct(StructTypeDefinition {
        id: NominalTypeId(0),
        path,
        fields: Box::new([]),
        generic_parameters: Some(generic_parameters),
        const_record: false,
    });
    nominal_id
}

/// Owns the hidden `GenericParameterScope` membership algorithm, not source acceptance.
///
/// WHAT: valid names build a scope whose `contains_name` lookup resolves every interned
///      parameter name that was registered for the list.
/// WHY: source acceptance of valid parameter names is integration-owned. This unit stays
///      because the interned-name to scope-membership mapping is a hidden data-structure
///      fact that integration output cannot inspect.
#[test]
fn generic_scope_accepts_pascal_case_and_single_uppercase_names() {
    let mut string_table = StringTable::new();
    let item_name = string_table.intern("ItemType");
    let t_name = string_table.intern("T");
    let error_kind_name = string_table.intern("ErrorKind");
    let list = GenericParameterList {
        parameters: vec![
            GenericParameter {
                id: TypeParameterId(0),
                name: item_name,
                location: SourceLocation::default(),
                trait_bounds: Vec::new(),
            },
            GenericParameter {
                id: TypeParameterId(1),
                name: t_name,
                location: SourceLocation::default(),
                trait_bounds: Vec::new(),
            },
            GenericParameter {
                id: TypeParameterId(2),
                name: error_kind_name,
                location: SourceLocation::default(),
                trait_bounds: Vec::new(),
            },
        ],
    };

    let scope = GenericParameterScope::from_parameter_list(
        &list,
        None,
        &FxHashSet::default(),
        &string_table,
        "AST Construction",
    )
    .expect("valid generic names should be accepted");

    assert!(scope.contains_name(item_name));
    assert!(scope.contains_name(t_name));
    assert!(scope.contains_name(error_kind_name));
}

#[test]
fn type_identity_keys_distinguish_nominal_generic_arguments() {
    let mut string_table = StringTable::new();
    let box_path = InternedPath::from_single_str("Box", &mut string_table);
    let pair_path = InternedPath::from_single_str("Pair", &mut string_table);
    let int_key = TypeIdentityKey::Builtin(BuiltinTypeKey::Int);
    let string_key = TypeIdentityKey::Builtin(BuiltinTypeKey::String);

    let int_instance = TypeIdentityKey::GenericInstance(GenericInstantiationKey {
        base_path: box_path.to_owned(),
        arguments: vec![int_key.to_owned()],
    });
    let another_int_instance = TypeIdentityKey::GenericInstance(GenericInstantiationKey {
        base_path: box_path.to_owned(),
        arguments: vec![int_key],
    });
    let string_instance = TypeIdentityKey::GenericInstance(GenericInstantiationKey {
        base_path: box_path,
        arguments: vec![string_key],
    });
    let pair_int_string = TypeIdentityKey::GenericInstance(GenericInstantiationKey {
        base_path: pair_path.to_owned(),
        arguments: vec![
            TypeIdentityKey::Builtin(BuiltinTypeKey::Int),
            TypeIdentityKey::Builtin(BuiltinTypeKey::String),
        ],
    });
    let pair_string_int = TypeIdentityKey::GenericInstance(GenericInstantiationKey {
        base_path: pair_path,
        arguments: vec![
            TypeIdentityKey::Builtin(BuiltinTypeKey::String),
            TypeIdentityKey::Builtin(BuiltinTypeKey::Int),
        ],
    });

    assert_eq!(int_instance, another_int_instance);
    assert_ne!(int_instance, string_instance);
    assert_ne!(pair_int_string, pair_string_int);
}

#[test]
fn type_bindings_accept_consistent_repeated_binding() {
    let mut bindings = GenericTypeBindings::new();
    let parameter_id = GenericParameterId(0);
    let concrete_type_id = TypeId(1);

    bindings
        .insert_consistent(parameter_id, concrete_type_id)
        .expect("initial binding should succeed");
    bindings
        .insert_consistent(parameter_id, concrete_type_id)
        .expect("same repeated binding should succeed");

    assert_eq!(bindings.get(parameter_id), Some(concrete_type_id));
}

#[test]
fn type_bindings_reject_conflicting_repeated_binding() {
    let mut bindings = GenericTypeBindings::new();
    let parameter_id = GenericParameterId(0);

    bindings
        .insert_consistent(parameter_id, TypeId(1))
        .expect("initial binding should succeed");

    let conflict = bindings
        .insert_consistent(parameter_id, TypeId(2))
        .expect_err("different concrete type should conflict");

    assert_eq!(conflict.parameter_id, parameter_id);
    assert_eq!(conflict.existing_type_id, TypeId(1));
    assert_eq!(conflict.replacement_type_id, TypeId(2));
}

#[test]
fn type_bindings_collect_arguments_in_parameter_order() {
    let mut type_environment = TypeEnvironment::new();
    let mut string_table = StringTable::new();
    let parsed_parameters = GenericParameterList {
        parameters: vec![
            GenericParameter {
                id: TypeParameterId(0),
                name: string_table.intern("First"),
                location: SourceLocation::default(),
                trait_bounds: Vec::new(),
            },
            GenericParameter {
                id: TypeParameterId(1),
                name: string_table.intern("Second"),
                location: SourceLocation::default(),
                trait_bounds: Vec::new(),
            },
        ],
    };
    let registered_parameters =
        type_environment.register_generic_parameter_list(&parsed_parameters, &Default::default());
    let first = registered_parameters.canonical_by_local[&TypeParameterId(0)];
    let second = registered_parameters.canonical_by_local[&TypeParameterId(1)];
    let list = registered_parameters.list_id;

    let mut bindings = GenericTypeBindings::new();
    bindings
        .insert_consistent(second, TypeId(2))
        .expect("second parameter should bind");
    bindings
        .insert_consistent(first, TypeId(1))
        .expect("first parameter should bind");

    assert!(bindings.is_complete_for(list, &type_environment));
    assert_eq!(
        bindings.concrete_arguments_for(list, &type_environment),
        Some(vec![TypeId(1), TypeId(2)].into_boxed_slice())
    );
}

#[test]
fn type_environment_allocates_distinct_canonical_ids_for_local_parameter_ids() {
    let mut type_environment = TypeEnvironment::new();
    let mut string_table = StringTable::new();
    let first_name = string_table.intern("T");
    let second_name = string_table.intern("T");

    let first_list = GenericParameterList {
        parameters: vec![GenericParameter {
            id: TypeParameterId(0),
            name: first_name,
            location: SourceLocation::default(),
            trait_bounds: Vec::new(),
        }],
    };
    let second_list = GenericParameterList {
        parameters: vec![GenericParameter {
            id: TypeParameterId(0),
            name: second_name,
            location: SourceLocation::default(),
            trait_bounds: Vec::new(),
        }],
    };

    let first_registered =
        type_environment.register_generic_parameter_list(&first_list, &Default::default());
    let second_registered =
        type_environment.register_generic_parameter_list(&second_list, &Default::default());
    let first_canonical = first_registered.canonical_by_local[&TypeParameterId(0)];
    let second_canonical = second_registered.canonical_by_local[&TypeParameterId(0)];

    assert_ne!(
        first_canonical, second_canonical,
        "declaration-local TypeParameterId(0) values must not share semantic identity"
    );
    assert_ne!(
        type_environment.type_id_for_generic_parameter(first_canonical),
        type_environment.type_id_for_generic_parameter(second_canonical),
        "each canonical generic parameter should have its own TypeId"
    );
}

#[test]
fn type_id_bindings_unify_generic_parameter_with_concrete_type() {
    let mut type_environment = TypeEnvironment::new();
    let mut string_table = StringTable::new();
    let parameter_id = GenericParameterId(0);
    let parameter_type_id =
        register_environment_parameter(&mut type_environment, &mut string_table, parameter_id, "T");
    let int_type_id = type_environment.builtins().int;
    let mut bindings = GenericTypeBindings::new();

    assert!(
        type_environment
            .try_collect_type_parameter_bindings_typeid(
                parameter_type_id,
                int_type_id,
                &mut bindings,
            )
            .unwrap()
    );
    assert_eq!(bindings.get(parameter_id), Some(int_type_id));
}

#[test]
fn type_id_bindings_accept_repeated_identical_unification() {
    let mut type_environment = TypeEnvironment::new();
    let mut string_table = StringTable::new();
    let parameter_id = GenericParameterId(0);
    let parameter_type_id =
        register_environment_parameter(&mut type_environment, &mut string_table, parameter_id, "T");
    let int_type_id = type_environment.builtins().int;
    let mut bindings = GenericTypeBindings::new();

    assert!(
        type_environment
            .try_collect_type_parameter_bindings_typeid(
                parameter_type_id,
                int_type_id,
                &mut bindings,
            )
            .unwrap()
    );
    assert!(
        type_environment
            .try_collect_type_parameter_bindings_typeid(
                parameter_type_id,
                int_type_id,
                &mut bindings,
            )
            .unwrap()
    );
    assert_eq!(bindings.get(parameter_id), Some(int_type_id));
}

#[test]
fn type_id_bindings_reject_conflicting_unification() {
    let mut type_environment = TypeEnvironment::new();
    let mut string_table = StringTable::new();
    let parameter_id = GenericParameterId(0);
    let parameter_type_id =
        register_environment_parameter(&mut type_environment, &mut string_table, parameter_id, "T");
    let int_type_id = type_environment.builtins().int;
    let string_type_id = type_environment.builtins().string;
    let mut bindings = GenericTypeBindings::new();

    assert!(
        type_environment
            .try_collect_type_parameter_bindings_typeid(
                parameter_type_id,
                int_type_id,
                &mut bindings,
            )
            .unwrap()
    );
    assert!(
        type_environment
            .try_collect_type_parameter_bindings_typeid(
                parameter_type_id,
                string_type_id,
                &mut bindings,
            )
            .is_err()
    );
    assert_eq!(bindings.get(parameter_id), Some(int_type_id));
}

#[test]
fn type_id_bindings_unify_collection_arguments() {
    let mut type_environment = TypeEnvironment::new();
    let mut string_table = StringTable::new();
    let parameter_id = GenericParameterId(0);
    let parameter_type_id =
        register_environment_parameter(&mut type_environment, &mut string_table, parameter_id, "T");
    let int_type_id = type_environment.builtins().int;
    let template_collection = type_environment.intern_constructed(
        TypeConstructor::Builtin(BuiltinTypeConstructor::Collection {
            fixed_capacity: None,
        }),
        Box::new([parameter_type_id]),
    );
    let concrete_collection = type_environment.intern_constructed(
        TypeConstructor::Builtin(BuiltinTypeConstructor::Collection {
            fixed_capacity: None,
        }),
        Box::new([int_type_id]),
    );
    let mut bindings = GenericTypeBindings::new();

    assert!(
        type_environment
            .try_collect_type_parameter_bindings_typeid(
                template_collection,
                concrete_collection,
                &mut bindings,
            )
            .unwrap()
    );
    assert_eq!(bindings.get(parameter_id), Some(int_type_id));
}

#[test]
fn type_id_bindings_unify_option_arguments() {
    let mut type_environment = TypeEnvironment::new();
    let mut string_table = StringTable::new();
    let parameter_id = GenericParameterId(0);
    let parameter_type_id =
        register_environment_parameter(&mut type_environment, &mut string_table, parameter_id, "T");
    let string_type_id = type_environment.builtins().string;
    let template_option = type_environment.intern_constructed(
        TypeConstructor::Builtin(BuiltinTypeConstructor::Option),
        Box::new([parameter_type_id]),
    );
    let concrete_option = type_environment.intern_constructed(
        TypeConstructor::Builtin(BuiltinTypeConstructor::Option),
        Box::new([string_type_id]),
    );
    let mut bindings = GenericTypeBindings::new();

    assert!(
        type_environment
            .try_collect_type_parameter_bindings_typeid(
                template_option,
                concrete_option,
                &mut bindings,
            )
            .unwrap()
    );
    assert_eq!(bindings.get(parameter_id), Some(string_type_id));
}

#[test]
fn type_id_bindings_unify_generic_instances_only_when_base_matches() {
    let mut type_environment = TypeEnvironment::new();
    let mut string_table = StringTable::new();
    let parameter_id = GenericParameterId(0);
    let parameter_type_id =
        register_environment_parameter(&mut type_environment, &mut string_table, parameter_id, "T");
    let parameter_list =
        register_single_parameter_list(&mut type_environment, &mut string_table, "T");
    let box_nominal = register_empty_generic_struct(
        &mut type_environment,
        &mut string_table,
        "Box",
        parameter_list,
    );
    let wrapper_nominal = register_empty_generic_struct(
        &mut type_environment,
        &mut string_table,
        "Wrapper",
        parameter_list,
    );
    let int_type_id = type_environment.builtins().int;

    let template_box =
        type_environment.intern_generic_instance(box_nominal, Box::new([parameter_type_id]));
    let concrete_box =
        type_environment.intern_generic_instance(box_nominal, Box::new([int_type_id]));
    let concrete_wrapper =
        type_environment.intern_generic_instance(wrapper_nominal, Box::new([int_type_id]));

    let mut matching_bindings = GenericTypeBindings::new();
    assert!(
        type_environment
            .try_collect_type_parameter_bindings_typeid(
                template_box,
                concrete_box,
                &mut matching_bindings,
            )
            .unwrap()
    );
    assert_eq!(matching_bindings.get(parameter_id), Some(int_type_id));

    let mut mismatched_bindings = GenericTypeBindings::new();
    assert!(
        !type_environment
            .try_collect_type_parameter_bindings_typeid(
                template_box,
                concrete_wrapper,
                &mut mismatched_bindings,
            )
            .unwrap()
    );
    assert_eq!(mismatched_bindings.get(parameter_id), None);
}

#[test]
fn substitute_type_id_rewrites_constructed_function_and_nominal_instances() {
    let mut type_environment = TypeEnvironment::new();
    let mut string_table = StringTable::new();
    let parameter_id = GenericParameterId(0);
    let parameter_type_id =
        register_environment_parameter(&mut type_environment, &mut string_table, parameter_id, "T");
    let int_type_id = type_environment.builtins().int;

    let parameter_list =
        register_single_parameter_list(&mut type_environment, &mut string_table, "T");
    let box_nominal = register_empty_generic_struct(
        &mut type_environment,
        &mut string_table,
        "Box",
        parameter_list,
    );

    let box_of_parameter =
        type_environment.intern_generic_instance(box_nominal, Box::new([parameter_type_id]));
    let collection_of_parameter = type_environment.intern_constructed(
        TypeConstructor::Builtin(BuiltinTypeConstructor::Collection {
            fixed_capacity: None,
        }),
        Box::new([parameter_type_id]),
    );
    let option_of_parameter = type_environment.intern_constructed(
        TypeConstructor::Builtin(BuiltinTypeConstructor::Option),
        Box::new([parameter_type_id]),
    );
    let tuple_with_parameter =
        type_environment.intern_tuple(vec![parameter_type_id, option_of_parameter]);
    let function_with_parameter = type_environment.intern_function(FunctionTypeKey {
        parameters: Box::new([box_of_parameter, collection_of_parameter]),
        returns: Box::new([tuple_with_parameter]),
        error_return: None,
    });

    let mut mapping = FxHashMap::default();
    mapping.insert(parameter_id, int_type_id);

    let substituted_collection =
        type_environment.substitute_type_id(collection_of_parameter, &mapping);
    assert_eq!(
        type_environment.collection_element_type(substituted_collection),
        Some(int_type_id)
    );

    let substituted_option = type_environment.substitute_type_id(option_of_parameter, &mapping);
    assert_eq!(
        type_environment.option_inner_type(substituted_option),
        Some(int_type_id)
    );

    let substituted_box = type_environment.substitute_type_id(box_of_parameter, &mapping);
    let Some(TypeDefinition::GenericInstance(instance)) = type_environment.get(substituted_box)
    else {
        panic!("substituted nominal generic should remain a generic instance");
    };
    assert_eq!(instance.base, box_nominal);
    assert_eq!(instance.arguments.as_ref(), &[int_type_id]);

    let substituted_tuple = type_environment.substitute_type_id(tuple_with_parameter, &mapping);
    let tuple_fields = type_environment
        .tuple_field_ids(substituted_tuple)
        .expect("substitution should preserve tuple identity");
    assert_eq!(tuple_fields[0], int_type_id);
    assert_eq!(
        type_environment.option_inner_type(tuple_fields[1]),
        Some(int_type_id)
    );

    let substituted_function =
        type_environment.substitute_type_id(function_with_parameter, &mapping);
    let substituted_function_again =
        type_environment.substitute_type_id(function_with_parameter, &mapping);
    assert_eq!(
        substituted_function_again, substituted_function,
        "repeated substitution should reuse the canonical function TypeId"
    );

    let Some(TypeDefinition::Function(function_definition)) =
        type_environment.get(substituted_function)
    else {
        panic!("substitution should preserve function type identity");
    };

    assert_eq!(function_definition.parameters[0].type_id, substituted_box);
    assert_eq!(
        type_environment.collection_element_type(function_definition.parameters[1].type_id),
        Some(int_type_id)
    );
    assert_eq!(function_definition.returns[0], substituted_tuple);
}

#[test]
fn trait_bounds_lookup_succeeds_after_registration() {
    let mut type_environment = TypeEnvironment::new();
    let mut string_table = StringTable::new();

    let parsed_parameters = GenericParameterList {
        parameters: vec![
            GenericParameter {
                id: TypeParameterId(0),
                name: string_table.intern("T"),
                location: SourceLocation::default(),
                trait_bounds: Vec::new(),
            },
            GenericParameter {
                id: TypeParameterId(1),
                name: string_table.intern("U"),
                location: SourceLocation::default(),
                trait_bounds: Vec::new(),
            },
        ],
    };

    let mut resolved_bounds = FxHashMap::default();
    resolved_bounds.insert(TypeParameterId(0), vec![TraitId(0), TraitId(1)]);
    resolved_bounds.insert(TypeParameterId(1), vec![TraitId(2)]);

    let registered =
        type_environment.register_generic_parameter_list(&parsed_parameters, &resolved_bounds);
    let canonical_t = registered.canonical_by_local[&TypeParameterId(0)];
    let canonical_u = registered.canonical_by_local[&TypeParameterId(1)];

    assert_eq!(
        type_environment.trait_bounds_for_generic_parameter(canonical_t),
        Some(&[TraitId(0), TraitId(1)][..]),
    );
    assert_eq!(
        type_environment.trait_bounds_for_generic_parameter(canonical_u),
        Some(&[TraitId(2)][..]),
    );
}

#[test]
fn trait_bounds_lookup_succeeds_after_update() {
    let mut type_environment = TypeEnvironment::new();
    let mut string_table = StringTable::new();

    let parsed_parameters = GenericParameterList {
        parameters: vec![GenericParameter {
            id: TypeParameterId(0),
            name: string_table.intern("T"),
            location: SourceLocation::default(),
            trait_bounds: Vec::new(),
        }],
    };

    // Register with empty bounds initially.
    let registered =
        type_environment.register_generic_parameter_list(&parsed_parameters, &Default::default());
    let canonical_t = registered.canonical_by_local[&TypeParameterId(0)];

    assert_eq!(
        type_environment.trait_bounds_for_generic_parameter(canonical_t),
        Some(&[][..]),
    );

    // Update bounds once trait definitions are resolved.
    let mut updated_bounds = FxHashMap::default();
    updated_bounds.insert(TypeParameterId(0), vec![TraitId(5)]);

    type_environment.update_generic_parameter_bounds(
        registered.list_id,
        &updated_bounds,
        &registered.canonical_by_local,
    );

    assert_eq!(
        type_environment.trait_bounds_for_generic_parameter(canonical_t),
        Some(&[TraitId(5)][..]),
    );
}

#[test]
fn trait_bounds_lookup_returns_none_for_unknown_parameter_id() {
    let type_environment = TypeEnvironment::new();

    assert_eq!(
        type_environment.trait_bounds_for_generic_parameter(GenericParameterId(999)),
        None,
    );
}

// -----------------------------------------------------------
//  Fixed-capacity collection generic tests
// -----------------------------------------------------------

#[test]
fn type_id_to_type_identity_key_preserves_fixed_capacity() {
    use crate::compiler_frontend::datatypes::generic_identity_bridge::{
        TypeIdentityKey, type_identity_key_to_type_id,
    };

    let mut type_environment = TypeEnvironment::new();
    let int = type_environment.builtins().int;

    // Round-trip a fixed-capacity collection
    let fixed_64 = type_environment.intern_collection(int, Some(64));
    let key = type_environment
        .type_id_to_type_identity_key(fixed_64)
        .expect("should produce key");

    // Verify the key carries fixed_capacity
    match &key {
        TypeIdentityKey::Collection {
            element: _,
            fixed_capacity,
        } => assert_eq!(*fixed_capacity, Some(64)),
        _ => panic!("expected Collection key, got {key:?}"),
    }

    // Round-trip back
    let round_tripped =
        type_identity_key_to_type_id(&key, &mut type_environment).expect("should round-trip");
    assert_eq!(round_tripped, fixed_64);
}

#[test]
fn type_id_to_type_identity_key_preserves_growable_collection() {
    use crate::compiler_frontend::datatypes::generic_identity_bridge::{
        TypeIdentityKey, type_identity_key_to_type_id,
    };

    let mut type_environment = TypeEnvironment::new();
    let int = type_environment.builtins().int;

    let growable = type_environment.intern_collection(int, None);
    let key = type_environment
        .type_id_to_type_identity_key(growable)
        .expect("should produce key");

    match &key {
        TypeIdentityKey::Collection {
            element: _,
            fixed_capacity,
        } => assert_eq!(*fixed_capacity, None),
        _ => panic!("expected Collection key"),
    }

    let round_tripped =
        type_identity_key_to_type_id(&key, &mut type_environment).expect("should round-trip");
    assert_eq!(round_tripped, growable);
}

#[test]
fn data_type_fixed_collection_bridge_preserves_capacity() {
    use crate::compiler_frontend::datatypes::generic_identity_bridge::{
        data_type_to_type_identity_key, type_identity_key_to_type_id,
    };

    let data_type = DataType::fixed_collection(DataType::Int, 64);
    let key = data_type_to_type_identity_key(&data_type).expect("fixed collection should bridge");

    match &key {
        TypeIdentityKey::Collection {
            fixed_capacity,
            element,
        } => {
            assert_eq!(*fixed_capacity, Some(64));
            assert_eq!(
                element.as_ref(),
                &TypeIdentityKey::Builtin(BuiltinTypeKey::Int)
            );
        }
        _ => panic!("expected Collection key"),
    }

    let mut type_environment = TypeEnvironment::new();
    let type_id =
        type_identity_key_to_type_id(&key, &mut type_environment).expect("key should resolve");
    let shape = type_environment
        .collection_shape(type_id)
        .expect("resolved type should be a collection");

    assert_eq!(shape.element_type, type_environment.builtins().int);
    assert_eq!(shape.fixed_capacity, Some(64));
}

#[test]
fn substitute_preserves_fixed_capacity() {
    use rustc_hash::FxHashMap;

    let mut type_environment = TypeEnvironment::new();
    let mut string_table = StringTable::new();
    let parameter_id = GenericParameterId(0);
    let parameter_type_id =
        register_environment_parameter(&mut type_environment, &mut string_table, parameter_id, "T");
    let int_type_id = type_environment.builtins().int;

    // Build a fixed-capacity collection parameterized by T
    let template_fixed = type_environment.intern_collection(parameter_type_id, Some(64));

    // Substitute T -> Int
    let mut mapping = FxHashMap::default();
    mapping.insert(parameter_id, int_type_id);

    let substituted = type_environment.substitute_type_id(template_fixed, &mapping);

    // Verify substitution produced the correct fixed-collection type
    let expected = type_environment.intern_collection(int_type_id, Some(64));
    assert_eq!(substituted, expected);

    // Verify the fixed capacity survived
    let shape = type_environment
        .collection_shape(substituted)
        .expect("should be a collection");
    assert_eq!(shape.element_type, int_type_id);
    assert_eq!(shape.fixed_capacity, Some(64));
}

#[test]
fn substitute_preserves_fixed_capacity_element_only_substitution() {
    use rustc_hash::FxHashMap;

    let mut type_environment = TypeEnvironment::new();
    let mut string_table = StringTable::new();
    let parameter_id = GenericParameterId(0);
    let parameter_type_id =
        register_environment_parameter(&mut type_environment, &mut string_table, parameter_id, "T");
    let string_type_id = type_environment.builtins().string;

    // Build a growable collection parameterized by T
    let template = type_environment.intern_collection(parameter_type_id, None);

    let mut mapping = FxHashMap::default();
    mapping.insert(parameter_id, string_type_id);

    let substituted = type_environment.substitute_type_id(template, &mapping);

    let expected = type_environment.intern_collection(string_type_id, None);
    assert_eq!(substituted, expected);
    assert_eq!(
        type_environment.collection_fixed_capacity(substituted),
        None
    );
}

fn register_two_parameter_list(
    type_environment: &mut TypeEnvironment,
    string_table: &mut StringTable,
    first_name: &str,
    second_name: &str,
) -> GenericParameterListId {
    let parameter_list = GenericParameterList {
        parameters: vec![
            GenericParameter {
                id: TypeParameterId(0),
                name: string_table.intern(first_name),
                location: SourceLocation::default(),
                trait_bounds: Vec::new(),
            },
            GenericParameter {
                id: TypeParameterId(1),
                name: string_table.intern(second_name),
                location: SourceLocation::default(),
                trait_bounds: Vec::new(),
            },
        ],
    };

    type_environment
        .register_generic_parameter_list(&parameter_list, &Default::default())
        .list_id
}

/// A nested structural mismatch after a partial binding must not leave the caller's
/// binding map mutated.
///
/// WHAT: a constructed type's first argument binds `T`, then a later nested argument is a
///       structural non-match, so the whole walk returns `Ok(false)` and the staged `T`
///       binding is rolled back.
/// WHY: without a transactional walk the partial `T` binding would survive and could poison
///      a later constraint or turn a mismatch into a spurious binding-conflict diagnostic.
#[test]
fn type_id_bindings_rollback_constructed_mismatch_after_partial_binding() {
    let mut type_environment = TypeEnvironment::new();
    let mut string_table = StringTable::new();
    let first_parameter_id = GenericParameterId(0);
    let second_parameter_id = GenericParameterId(1);
    let first_parameter_type_id = register_environment_parameter(
        &mut type_environment,
        &mut string_table,
        first_parameter_id,
        "T",
    );
    let second_parameter_type_id = register_environment_parameter(
        &mut type_environment,
        &mut string_table,
        second_parameter_id,
        "U",
    );
    let int_type_id = type_environment.builtins().int;

    // Template: OrderedMap<T, Collection<U>>; concrete: OrderedMap<Int, Option<Int>>.
    // The first argument binds T -> Int, the second is a structural non-match
    // (Collection vs Option) which makes the whole walk return Ok(false).
    let template_inner_collection = type_environment.intern_constructed(
        TypeConstructor::Builtin(BuiltinTypeConstructor::Collection {
            fixed_capacity: None,
        }),
        Box::new([second_parameter_type_id]),
    );
    let template = type_environment.intern_constructed(
        TypeConstructor::Builtin(BuiltinTypeConstructor::OrderedMap),
        Box::new([first_parameter_type_id, template_inner_collection]),
    );
    let concrete_inner_option = type_environment.intern_constructed(
        TypeConstructor::Builtin(BuiltinTypeConstructor::Option),
        Box::new([int_type_id]),
    );
    let concrete = type_environment.intern_constructed(
        TypeConstructor::Builtin(BuiltinTypeConstructor::OrderedMap),
        Box::new([int_type_id, concrete_inner_option]),
    );

    let mut bindings = GenericTypeBindings::new();
    assert!(
        !type_environment
            .try_collect_type_parameter_bindings_typeid(template, concrete, &mut bindings)
            .unwrap()
    );
    assert_eq!(
        bindings.get(first_parameter_id),
        None,
        "the partial T binding must be rolled back after the nested mismatch"
    );
    assert_eq!(
        bindings.get(second_parameter_id),
        None,
        "no parameter should be bound after a structural mismatch"
    );
}

/// A nested generic-instance mismatch after a partial binding must not leave the caller's
/// binding map mutated.
///
/// WHAT: a generic nominal instance's first argument binds `T`, then a later nested argument
///       is a generic-instance structural non-match, so the whole walk returns `Ok(false)` and
///       the staged `T` binding is rolled back.
/// WHY: the rollback property must hold for generic-instance recursion, not only constructed
///      types, so a mismatch cannot leave partial inference evidence behind.
#[test]
fn type_id_bindings_rollback_generic_instance_mismatch_after_partial_binding() {
    let mut type_environment = TypeEnvironment::new();
    let mut string_table = StringTable::new();
    let parameter_id = GenericParameterId(0);
    let parameter_type_id =
        register_environment_parameter(&mut type_environment, &mut string_table, parameter_id, "T");

    let pair_list = register_two_parameter_list(&mut type_environment, &mut string_table, "A", "B");
    let box_list = register_single_parameter_list(&mut type_environment, &mut string_table, "Item");
    let wrapper_list =
        register_single_parameter_list(&mut type_environment, &mut string_table, "Item");
    let pair_nominal =
        register_empty_generic_struct(&mut type_environment, &mut string_table, "Pair", pair_list);
    let box_nominal =
        register_empty_generic_struct(&mut type_environment, &mut string_table, "Box", box_list);
    let wrapper_nominal = register_empty_generic_struct(
        &mut type_environment,
        &mut string_table,
        "Wrapper",
        wrapper_list,
    );
    let int_type_id = type_environment.builtins().int;

    // Template: Pair<T, Box<T>>; concrete: Pair<Int, Wrapper<Int>>.
    // The first argument binds T -> Int, the second is a base mismatch
    // (Box vs Wrapper) which makes the whole walk return Ok(false).
    let template_inner_box =
        type_environment.intern_generic_instance(box_nominal, Box::new([parameter_type_id]));
    let template = type_environment.intern_generic_instance(
        pair_nominal,
        Box::new([parameter_type_id, template_inner_box]),
    );
    let concrete_inner_wrapper =
        type_environment.intern_generic_instance(wrapper_nominal, Box::new([int_type_id]));
    let concrete = type_environment.intern_generic_instance(
        pair_nominal,
        Box::new([int_type_id, concrete_inner_wrapper]),
    );

    let mut bindings = GenericTypeBindings::new();
    assert!(
        !type_environment
            .try_collect_type_parameter_bindings_typeid(template, concrete, &mut bindings)
            .unwrap()
    );
    assert_eq!(
        bindings.get(parameter_id),
        None,
        "the partial T binding must be rolled back after the nested mismatch"
    );
}

/// A repeated-parameter conflict produced within one staged walk must still surface the
/// `BindingConflict` facts while leaving the caller's binding map unchanged.
///
/// WHAT: the same generic parameter appears in two argument positions; the first binds it,
///       the second binds it to a different concrete type, producing a `BindingConflict` whose
///       existing/replacement TypeIds come from within the same staged walk.
/// WHY: the conflict must be reported with its real evidence, but the caller's map must stay
///      byte-for-byte unchanged so the conflict does not leak partial bindings.
#[test]
fn type_id_bindings_rollback_preserves_repeated_parameter_conflict_within_one_walk() {
    let mut type_environment = TypeEnvironment::new();
    let mut string_table = StringTable::new();
    let parameter_id = GenericParameterId(0);
    let parameter_type_id =
        register_environment_parameter(&mut type_environment, &mut string_table, parameter_id, "T");
    let int_type_id = type_environment.builtins().int;
    let string_type_id = type_environment.builtins().string;

    // Template: OrderedMap<T, T>; concrete: OrderedMap<Int, String>.
    // The first argument stages T -> Int, the second stages T -> String and conflicts.
    let template = type_environment.intern_constructed(
        TypeConstructor::Builtin(BuiltinTypeConstructor::OrderedMap),
        Box::new([parameter_type_id, parameter_type_id]),
    );
    let concrete = type_environment.intern_constructed(
        TypeConstructor::Builtin(BuiltinTypeConstructor::OrderedMap),
        Box::new([int_type_id, string_type_id]),
    );

    let mut bindings = GenericTypeBindings::new();
    let conflict = type_environment
        .try_collect_type_parameter_bindings_typeid(template, concrete, &mut bindings)
        .expect_err("a repeated parameter bound to two types must conflict");
    assert_eq!(conflict.parameter_id, parameter_id);
    assert_eq!(conflict.existing_type_id, int_type_id);
    assert_eq!(conflict.replacement_type_id, string_type_id);
    assert_eq!(
        bindings.get(parameter_id),
        None,
        "the caller's binding map must remain unchanged after a conflict"
    );
}
