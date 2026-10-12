//! Compatibility check tests for `type_coercion::compatibility`.

use crate::compiler_frontend::datatypes::builtin_type_ids;
use crate::compiler_frontend::datatypes::definitions::StructTypeDefinition;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::BuiltinTypeConstructor;
use crate::compiler_frontend::datatypes::ids::{NominalTypeId, TypeConstructor};
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::type_coercion::compatibility::{
    TypeCompatibilityCache, TypeCompatibilityMode, is_postfix_error_compatible, is_type_compatible,
};
use moth_lexical::numeric::fixed_scalar::FixedScalar;

/// One fieldless nominal struct definition whose registration-assigned identity is still unset.
fn unregistered_struct_definition(path: PathId, const_record: bool) -> StructTypeDefinition {
    StructTypeDefinition {
        // `register_nominal_struct` overwrites this placeholder and returns the real identity,
        // so fixtures must take nominal ids from registration rather than from this literal.
        id: NominalTypeId(0),
        path,
        fields: Box::new([]),
        generic_parameters: None,
        const_record,
    }
}

#[test]
fn numeric_scalars_require_exact_type_compatibility() {
    let env = TypeEnvironment::new();

    // Numeric conversion is explicit-only, so no row here may become compatible from width,
    // precision or family convenience: narrow integers stay narrow, the removed
    // integer/unsigned-to-binary-float receiving promotions stay removed, and signedness and
    // float precision stay part of the type identity.
    let mismatches = [
        (FixedScalar::I8, FixedScalar::I32),
        (FixedScalar::U8, FixedScalar::U16),
        (FixedScalar::I32, FixedScalar::F64),
        (FixedScalar::U32, FixedScalar::F64),
        (FixedScalar::U64, FixedScalar::F64),
        (FixedScalar::F64, FixedScalar::I32),
        (FixedScalar::F64, FixedScalar::U32),
        (FixedScalar::F64, FixedScalar::F32),
        (FixedScalar::F32, FixedScalar::F64),
        (FixedScalar::I32, FixedScalar::U32),
        (FixedScalar::U32, FixedScalar::I32),
        (FixedScalar::Byte, FixedScalar::U8),
        (FixedScalar::U8, FixedScalar::Byte),
    ];

    for (value, receiver) in mismatches {
        assert!(
            !is_type_compatible(
                builtin_type_ids::fixed_scalar(receiver),
                builtin_type_ids::fixed_scalar(value),
                &env
            ),
            "{} must not implicitly satisfy {}",
            value.name(),
            receiver.name()
        );
    }
}

#[test]
fn bool_to_float_is_never_compatible() {
    let env = TypeEnvironment::new();
    assert!(!is_type_compatible(
        builtin_type_ids::fixed_scalar(FixedScalar::F64),
        env.builtins().bool,
        &env
    ));
}

#[test]
fn collection_element_scalars_keep_distinct_identities() {
    let mut env = TypeEnvironment::new();
    let u8_collection =
        env.intern_collection(builtin_type_ids::fixed_scalar(FixedScalar::U8), None);
    let byte_collection =
        env.intern_collection(builtin_type_ids::fixed_scalar(FixedScalar::Byte), None);

    // Different element scalars make different collection types, so `{Byte}` is not accepted
    // where `{U8}` is expected, or the reverse.
    assert!(!is_type_compatible(byte_collection, u8_collection, &env));
    assert!(!is_type_compatible(u8_collection, byte_collection, &env));
}

#[test]
fn option_accepts_none() {
    let mut env = TypeEnvironment::new();
    let i32_type = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let option_i32 = env.intern_constructed(
        TypeConstructor::Builtin(BuiltinTypeConstructor::Option),
        Box::new([i32_type]),
    );
    assert!(is_type_compatible(option_i32, env.builtins().none, &env));
}

#[test]
fn option_accepts_inner_type() {
    let mut env = TypeEnvironment::new();
    let i32_type = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let option_i32 = env.intern_option(i32_type);
    assert!(is_type_compatible(option_i32, i32_type, &env));
}

#[test]
fn raw_result_compatibility_does_not_use_none_slots_as_wildcards() {
    let mut env = TypeEnvironment::new();
    let i32_type = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let error = env.builtins().string;
    let none = env.builtins().none;
    let result_with_success = env.intern_constructed(
        TypeConstructor::Builtin(BuiltinTypeConstructor::FallibleCarrier),
        Box::new([i32_type, error]),
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
    let i32_type = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let f64_type = builtin_type_ids::fixed_scalar(FixedScalar::F64);

    assert!(is_type_compatible(i32_type, i32_type, &env));
    assert!(is_type_compatible(f64_type, f64_type, &env));
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
    let i32_type = builtin_type_ids::fixed_scalar(FixedScalar::I32);

    assert!(cache.is_compatible(i32_type, i32_type, TypeCompatibilityMode::Standard, &env));

    assert_eq!(cache.len(), 0);
}

#[test]
fn collection_type_identity_matches_exact_shape() {
    let mut env = TypeEnvironment::new();
    let i32_type = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let collection_a = env.intern_collection(i32_type, None);
    let collection_b = env.intern_collection(i32_type, None);

    // Same constructed type key resolves to the same canonical TypeId.
    assert_eq!(collection_a, collection_b);
    assert!(is_type_compatible(collection_a, collection_b, &env));
}

#[test]
fn fixed_and_growable_collections_are_not_compatible() {
    let mut env = TypeEnvironment::new();
    let i32_type = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let growable = env.intern_collection(i32_type, None);
    let fixed_64 = env.intern_collection(i32_type, Some(64));
    let fixed_128 = env.intern_collection(i32_type, Some(128));

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
    let path = path_fork
        .try_intern_portable_path("User", &mut string_table)
        .expect("test path fits");

    let (_, runtime_a) = env.register_nominal_struct(unregistered_struct_definition(path, false));
    let (_, runtime_b) = env.register_nominal_struct(unregistered_struct_definition(path, false));
    let (_, const_record) = env.register_nominal_struct(unregistered_struct_definition(path, true));

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
    let pair_path = path_fork
        .try_intern_portable_path("Pair", &mut string_table)
        .expect("test path fits");

    let (pair_nominal, _) =
        env.register_nominal_struct(unregistered_struct_definition(pair_path, false));

    let i32_type = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let string = env.builtins().string;

    let instance_a = env.intern_generic_instance(pair_nominal, Box::new([i32_type, string]));
    let instance_b = env.intern_generic_instance(pair_nominal, Box::new([i32_type, string]));

    // Same generic instance key resolves to the same canonical TypeId.
    assert_eq!(instance_a, instance_b);
    assert!(is_type_compatible(instance_a, instance_b, &env));
}

#[test]
fn const_record_generic_instance_is_not_compatible_with_runtime_generic_instance() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut env = TypeEnvironment::new();
    let pair_path = path_fork
        .try_intern_portable_path("Pair", &mut string_table)
        .expect("test path fits");

    let (runtime_nominal, _) =
        env.register_nominal_struct(unregistered_struct_definition(pair_path, false));
    let (const_nominal, _) =
        env.register_nominal_struct(unregistered_struct_definition(pair_path, true));

    let i32_type = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let string = env.builtins().string;

    let runtime_instance =
        env.intern_generic_instance(runtime_nominal, Box::new([i32_type, string]));
    let const_instance = env.intern_generic_instance(const_nominal, Box::new([i32_type, string]));

    assert!(!is_type_compatible(runtime_instance, const_instance, &env));
}

#[test]
fn generic_instance_argument_order_still_matters() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut env = TypeEnvironment::new();
    let pair_path = path_fork
        .try_intern_portable_path("Pair", &mut string_table)
        .expect("test path fits");

    let (pair_nominal, _) =
        env.register_nominal_struct(unregistered_struct_definition(pair_path, false));

    let i32_type = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let string = env.builtins().string;

    let i32_string_pair = env.intern_generic_instance(pair_nominal, Box::new([i32_type, string]));
    let string_i32_pair = env.intern_generic_instance(pair_nominal, Box::new([string, i32_type]));

    assert!(!is_type_compatible(i32_string_pair, string_i32_pair, &env));
}

#[test]
fn compatibility_cache_reuses_standard_results() {
    let mut env = TypeEnvironment::new();
    let i32_type = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let option_i32 = env.intern_constructed(
        TypeConstructor::Builtin(BuiltinTypeConstructor::Option),
        Box::new([i32_type]),
    );

    let mut cache = TypeCompatibilityCache::new();

    assert!(cache.is_compatible(option_i32, i32_type, TypeCompatibilityMode::Standard, &env));
    assert_eq!(cache.len(), 1);

    assert!(cache.is_compatible(option_i32, i32_type, TypeCompatibilityMode::Standard, &env));
    assert_eq!(cache.len(), 1);
}

#[test]
fn compatibility_cache_keeps_mutable_rvalue_policy_separate() {
    let mut env = TypeEnvironment::new();
    let i32_type = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let option_i32 = env.intern_option(i32_type);
    let option_i32_collection = env.intern_collection(option_i32, None);
    let i32_collection = env.intern_collection(i32_type, None);

    let mut cache = TypeCompatibilityCache::new();

    assert!(!cache.is_compatible(
        option_i32_collection,
        i32_collection,
        TypeCompatibilityMode::Standard,
        &env
    ));
    assert_eq!(cache.len(), 1);

    assert!(cache.is_compatible(
        option_i32_collection,
        i32_collection,
        TypeCompatibilityMode::FreshMutableRvalue,
        &env
    ));
    assert_eq!(cache.len(), 2);

    assert!(cache.is_compatible(
        option_i32_collection,
        i32_collection,
        TypeCompatibilityMode::FreshMutableRvalue,
        &env
    ));
    assert_eq!(cache.len(), 2);
}
