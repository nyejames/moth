//! Compatibility check tests for `type_coercion::compatibility`.

use crate::compiler_frontend::datatypes::definitions::StructTypeDefinition;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::BuiltinTypeConstructor;
use crate::compiler_frontend::datatypes::ids::{NominalTypeId, TypeConstructor};
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::type_coercion::compatibility::{
    TypeCompatibilityCache, TypeCompatibilityMode, is_declaration_compatible,
    is_postfix_error_compatible, is_type_compatible,
};

#[test]
fn type_compatibility_int_vs_float_is_incompatible() {
    let env = TypeEnvironment::new();
    assert!(!is_type_compatible(
        env.builtins().float,
        env.builtins().int,
        &env
    ));
}

#[test]
fn declaration_compatibility_int_vs_float_is_compatible() {
    let env = TypeEnvironment::new();
    assert!(is_declaration_compatible(
        env.builtins().float,
        env.builtins().int,
        &env
    ));
}

#[test]
fn float_to_int_is_never_compatible() {
    let env = TypeEnvironment::new();
    assert!(!is_type_compatible(
        env.builtins().int,
        env.builtins().float,
        &env
    ));
    assert!(!is_declaration_compatible(
        env.builtins().int,
        env.builtins().float,
        &env
    ));
}

#[test]
fn bool_to_float_is_never_compatible() {
    let env = TypeEnvironment::new();
    assert!(!is_type_compatible(
        env.builtins().float,
        env.builtins().bool,
        &env
    ));
}

#[test]
fn option_accepts_none() {
    let mut env = TypeEnvironment::new();
    let int = env.builtins().int;
    let option_int = env.intern_constructed(
        TypeConstructor::Builtin(BuiltinTypeConstructor::Option),
        Box::new([int]),
    );
    assert!(is_type_compatible(option_int, env.builtins().none, &env));
}

#[test]
fn option_accepts_inner_type() {
    let mut env = TypeEnvironment::new();
    let int = env.builtins().int;
    let option_int = env.intern_constructed(
        TypeConstructor::Builtin(BuiltinTypeConstructor::Option),
        Box::new([int]),
    );
    assert!(is_type_compatible(option_int, int, &env));
}

#[test]
fn raw_result_compatibility_does_not_use_none_slots_as_wildcards() {
    let mut env = TypeEnvironment::new();
    let int = env.builtins().int;
    let error = env.builtins().string;
    let none = env.builtins().none;
    let result_with_success = env.intern_constructed(
        TypeConstructor::Builtin(BuiltinTypeConstructor::FallibleCarrier),
        Box::new([int, error]),
    );
    let result_without_success = env.intern_constructed(
        TypeConstructor::Builtin(BuiltinTypeConstructor::FallibleCarrier),
        Box::new([none, error]),
    );

    assert!(!is_type_compatible(
        result_without_success,
        result_with_success,
        &env
    ));
    assert!(!is_type_compatible(
        result_with_success,
        result_without_success,
        &env
    ));
}

#[test]
fn postfix_error_compatibility_accepts_exact_and_one_level_option_wrapping() {
    let mut env = TypeEnvironment::new();
    let error = env.builtins().string;
    let optional_error = env.intern_option(error);

    assert!(is_postfix_error_compatible(error, error, &env));
    assert!(is_postfix_error_compatible(optional_error, error, &env));
    assert!(!is_postfix_error_compatible(error, optional_error, &env));
}

#[test]
fn identical_types_are_always_compatible() {
    let env = TypeEnvironment::new();
    assert!(is_type_compatible(
        env.builtins().int,
        env.builtins().int,
        &env
    ));
    assert!(is_type_compatible(
        env.builtins().float,
        env.builtins().float,
        &env
    ));
    assert!(is_type_compatible(
        env.builtins().bool,
        env.builtins().bool,
        &env
    ));
}

#[test]
fn compatibility_cache_bypasses_exact_type_identity() {
    let env = TypeEnvironment::new();
    let mut cache = TypeCompatibilityCache::new();

    assert!(cache.is_compatible(
        env.builtins().int,
        env.builtins().int,
        TypeCompatibilityMode::Standard,
        &env
    ));

    assert_eq!(cache.len(), 0);
}

#[test]
fn collection_type_identity_matches_exact_shape() {
    let mut env = TypeEnvironment::new();
    let int = env.builtins().int;
    let collection_a = env.intern_collection(int, None);
    let collection_b = env.intern_collection(int, None);

    // Same constructed type key resolves to the same canonical TypeId.
    assert_eq!(collection_a, collection_b);
    assert!(is_type_compatible(collection_a, collection_b, &env));
}

#[test]
fn fixed_and_growable_collections_are_not_compatible() {
    let mut env = TypeEnvironment::new();
    let int = env.builtins().int;
    let growable = env.intern_collection(int, None);
    let fixed_64 = env.intern_collection(int, Some(64));
    let fixed_128 = env.intern_collection(int, Some(128));

    assert!(!is_type_compatible(fixed_64, growable, &env));
    assert!(!is_type_compatible(growable, fixed_64, &env));
    assert!(!is_type_compatible(fixed_64, fixed_128, &env));

    let mut cache = TypeCompatibilityCache::new();
    assert!(!cache.is_compatible(
        fixed_64,
        growable,
        TypeCompatibilityMode::FreshMutableRvalue,
        &env
    ));
}

#[test]
fn struct_type_identity_is_nominal_and_const_record_sensitive_only() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut env = TypeEnvironment::new();
    let path = path_fork.try_intern_portable_path("User", &mut string_table).expect("test path fits");

    let (_, runtime_a) = env.register_nominal_struct(StructTypeDefinition {
        id: NominalTypeId(0),
        path: path.clone(),
        fields: Box::new([]),
        generic_parameters: None,
        const_record: false,
    });

    let (_, runtime_b) = env.register_nominal_struct(StructTypeDefinition {
        id: NominalTypeId(0),
        path: path.clone(),
        fields: Box::new([]),
        generic_parameters: None,
        const_record: false,
    });

    let (_, const_record) = env.register_nominal_struct(StructTypeDefinition {
        id: NominalTypeId(0),
        path,
        fields: Box::new([]),
        generic_parameters: None,
        const_record: true,
    });

    // Same nominal path, same const_record flag => compatible.
    assert!(is_type_compatible(runtime_a, runtime_b, &env));
    // Const-record vs runtime => incompatible.
    assert!(!is_type_compatible(runtime_a, const_record, &env));
}

#[test]
fn generic_instance_same_arguments_are_compatible() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut env = TypeEnvironment::new();
    let pair_path = path_fork.try_intern_portable_path("Pair", &mut string_table).expect("test path fits");

    let (pair_nominal, _) = env.register_nominal_struct(StructTypeDefinition {
        id: NominalTypeId(0),
        path: pair_path,
        fields: Box::new([]),
        generic_parameters: None,
        const_record: false,
    });

    let int = env.builtins().int;
    let string = env.builtins().string;

    let instance_a = env.intern_generic_instance(pair_nominal, Box::new([int, string]));
    let instance_b = env.intern_generic_instance(pair_nominal, Box::new([int, string]));

    // Same generic instance key resolves to the same canonical TypeId.
    assert_eq!(instance_a, instance_b);
    assert!(is_type_compatible(instance_a, instance_b, &env));
    assert!(is_declaration_compatible(instance_a, instance_b, &env));
}

#[test]
fn const_record_generic_instance_is_not_compatible_with_runtime_generic_instance() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut env = TypeEnvironment::new();
    let pair_path = path_fork.try_intern_portable_path("Pair", &mut string_table).expect("test path fits");

    let (runtime_nominal, _) = env.register_nominal_struct(StructTypeDefinition {
        id: NominalTypeId(0),
        path: pair_path.clone(),
        fields: Box::new([]),
        generic_parameters: None,
        const_record: false,
    });

    let (const_nominal, _) = env.register_nominal_struct(StructTypeDefinition {
        id: NominalTypeId(0),
        path: pair_path,
        fields: Box::new([]),
        generic_parameters: None,
        const_record: true,
    });

    let int = env.builtins().int;
    let string = env.builtins().string;

    let runtime_instance = env.intern_generic_instance(runtime_nominal, Box::new([int, string]));
    let const_instance = env.intern_generic_instance(const_nominal, Box::new([int, string]));

    assert!(!is_type_compatible(runtime_instance, const_instance, &env));
}

#[test]
fn generic_instance_argument_order_still_matters() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut env = TypeEnvironment::new();
    let pair_path = path_fork.try_intern_portable_path("Pair", &mut string_table).expect("test path fits");

    let (pair_nominal, _) = env.register_nominal_struct(StructTypeDefinition {
        id: NominalTypeId(0),
        path: pair_path,
        fields: Box::new([]),
        generic_parameters: None,
        const_record: false,
    });

    let int = env.builtins().int;
    let string = env.builtins().string;

    let int_string_pair = env.intern_generic_instance(pair_nominal, Box::new([int, string]));
    let string_int_pair = env.intern_generic_instance(pair_nominal, Box::new([string, int]));

    assert!(!is_type_compatible(int_string_pair, string_int_pair, &env));
}

#[test]
fn compatibility_cache_reuses_standard_results() {
    let mut env = TypeEnvironment::new();
    let int = env.builtins().int;
    let option_int = env.intern_constructed(
        TypeConstructor::Builtin(BuiltinTypeConstructor::Option),
        Box::new([int]),
    );

    let mut cache = TypeCompatibilityCache::new();

    assert!(cache.is_compatible(option_int, int, TypeCompatibilityMode::Standard, &env));
    assert_eq!(cache.len(), 1);

    assert!(cache.is_compatible(option_int, int, TypeCompatibilityMode::Standard, &env));
    assert_eq!(cache.len(), 1);
}

#[test]
fn compatibility_cache_keeps_mutable_rvalue_policy_separate() {
    let mut env = TypeEnvironment::new();
    let int = env.builtins().int;
    let option_int = env.intern_option(int);
    let option_int_collection = env.intern_collection(option_int, None);
    let int_collection = env.intern_collection(int, None);

    let mut cache = TypeCompatibilityCache::new();

    assert!(!cache.is_compatible(
        option_int_collection,
        int_collection,
        TypeCompatibilityMode::Standard,
        &env
    ));
    assert_eq!(cache.len(), 1);

    assert!(cache.is_compatible(
        option_int_collection,
        int_collection,
        TypeCompatibilityMode::FreshMutableRvalue,
        &env
    ));
    assert_eq!(cache.len(), 2);

    assert!(cache.is_compatible(
        option_int_collection,
        int_collection,
        TypeCompatibilityMode::FreshMutableRvalue,
        &env
    ));
    assert_eq!(cache.len(), 2);
}
