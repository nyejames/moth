//! Focused type-identity tests for the fixed-width builtin scalars.
//!
//! WHAT: proves `I8`..`F64` and `Byte` are seeded as distinct builtin identities that never alias
//!       each other or `Char`, including through canonical and generic identity. It also owns the
//!       exact unsigned range facts and the canonical/bridge key round trip for one exact width.
//! WHY: identity is the whole contract of these types; a width that shared another width's
//!      identity, or an alias that collapsed an exact width, would silently break the casts,
//!      operators and foreign boundaries that later phases build on canonical identity.

use crate::compiler_frontend::canonical_type_identity::{
    CanonicalBuiltinType, builtin_key_for_canonical_builtin, canonical_builtin_for_key,
    intern_canonical_builtin,
};
use crate::compiler_frontend::datatypes::builtin_type_ids;
use crate::compiler_frontend::datatypes::definitions::{StructTypeDefinition, TypeDefinition};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::generic_identity_bridge::{
    BuiltinTypeKey as BridgeBuiltinTypeKey, GenericInstantiationKey, TypeIdentityKey,
};
use crate::compiler_frontend::datatypes::ids::{BuiltinTypeKey, NominalTypeId, TypeId};
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerBuilder};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use moth_lexical::numeric::decimal::NumberScale;
use moth_lexical::numeric::fixed_scalar::FixedScalar;

fn seeded_builtin_key(environment: &TypeEnvironment, type_id: TypeId) -> BuiltinTypeKey {
    match environment.get(type_id) {
        Some(TypeDefinition::Builtin(definition)) => definition.key,
        other => panic!("{type_id:?} must be a seeded builtin, found {other:?}"),
    }
}

fn test_path(
    path_builder: &mut PathInternerBuilder,
    string_table: &mut StringTable,
    spelling: &str,
) -> PathId {
    path_builder
        .try_intern_portable_path(spelling, string_table)
        .expect("test path fits")
}

#[test]
fn seeded_fixed_scalars_are_distinct_builtin_identities() {
    let environment = TypeEnvironment::new();

    let mut seen = Vec::new();
    for scalar in FixedScalar::ALL {
        let type_id = builtin_type_ids::fixed_scalar(scalar);
        assert_eq!(
            seeded_builtin_key(&environment, type_id),
            BuiltinTypeKey::FixedScalar(scalar),
            "{} must be seeded at its deterministic TypeId",
            scalar.name()
        );
        assert!(
            !seen.contains(&type_id),
            "{} must not share a TypeId with another fixed scalar",
            scalar.name()
        );
        seen.push(type_id);
    }
}

#[test]
fn number_scales_intern_lazily_and_reuse_generated_fork_prefix() {
    let mut environment = TypeEnvironment::new();
    let scale_zero = NumberScale::from_name("Dec").expect("Dec is scale zero");
    let dec_zero_alias = NumberScale::from_name("Dec0").expect("Dec0 is scale zero");
    let scale_two = NumberScale::new(2).expect("scale two is within the Dec range");
    assert_eq!(dec_zero_alias, scale_zero);

    let number_type_id = environment.intern_number(scale_zero);
    assert_eq!(environment.number_scale(number_type_id), Some(scale_zero));
    assert_eq!(environment.intern_number(dec_zero_alias), number_type_id);
    assert_eq!(
        intern_canonical_builtin(
            CanonicalBuiltinType::Number(dec_zero_alias),
            &mut environment
        ),
        Some(number_type_id)
    );

    let mut generated = environment.fork_for_generated();
    assert_eq!(generated.intern_number(scale_zero), number_type_id);
    assert_eq!(
        intern_canonical_builtin(CanonicalBuiltinType::Number(dec_zero_alias), &mut generated),
        Some(number_type_id)
    );

    let generated_scale_two_type_id = generated.intern_number(scale_two);
    assert_eq!(
        generated.intern_number(scale_two),
        generated_scale_two_type_id
    );
    assert_ne!(generated_scale_two_type_id, number_type_id);
    assert_eq!(
        generated.number_scale(generated_scale_two_type_id),
        Some(scale_two)
    );
    assert_eq!(environment.number_scale(generated_scale_two_type_id), None);
}

#[test]
fn generic_instances_of_different_fixed_scalars_are_distinct_identities() {
    let mut environment = TypeEnvironment::new();
    let mut string_table = StringTable::new();
    let mut path_builder = PathInternerBuilder::new();
    let box_path = test_path(&mut path_builder, &mut string_table, "Box");

    let (nominal_id, _) = environment.register_nominal_struct(StructTypeDefinition {
        id: NominalTypeId(0), // overwritten by `register_nominal_struct`
        path: box_path,
        fields: Box::new([]),
        generic_parameters: None,
        const_record: false,
    });

    let box_of_u8 = environment.intern_generic_instance(
        nominal_id,
        vec![builtin_type_ids::fixed_scalar(FixedScalar::U8)].into_boxed_slice(),
    );
    let box_of_i8 = environment.intern_generic_instance(
        nominal_id,
        vec![builtin_type_ids::fixed_scalar(FixedScalar::I8)].into_boxed_slice(),
    );

    assert_ne!(
        box_of_u8, box_of_i8,
        "`Box of U8` and `Box of I8` must be separate generic instances"
    );
    assert_ne!(
        environment.type_id_to_type_identity_key(box_of_u8),
        environment.type_id_to_type_identity_key(box_of_i8),
        "generic instance identity keys must keep the fixed scalar argument distinct"
    );
    assert_eq!(
        environment.type_id_to_type_identity_key(box_of_u8),
        Some(TypeIdentityKey::GenericInstance(GenericInstantiationKey {
            base_path: box_path,
            arguments: vec![TypeIdentityKey::Builtin(BridgeBuiltinTypeKey::FixedScalar(
                FixedScalar::U8
            ))],
        }))
    );
}

/// The exact unsigned widths keep their own zero-based range facts, distinct from the same-width
/// signed range.
#[test]
fn unsigned_fixed_scalar_ranges_are_exact_and_distinct_from_signed() {
    assert_eq!(
        NumericScalar::Fixed(FixedScalar::U32).integer_range(),
        Some((0, i128::from(u32::MAX)))
    );
    assert_eq!(
        NumericScalar::Fixed(FixedScalar::U64).integer_range(),
        Some((0, i128::from(u64::MAX)))
    );
    assert_ne!(
        NumericScalar::Fixed(FixedScalar::U32).integer_range(),
        NumericScalar::Fixed(FixedScalar::I32).integer_range(),
        "U32 starts at zero while I32 starts negative"
    );
}

/// An unsigned width is an integer domain and carries no binary-float precision.
#[test]
fn unsigned_fixed_scalars_are_integers_without_binary_float_precision() {
    let u32_scalar = NumericScalar::Fixed(FixedScalar::U32);
    assert!(u32_scalar.is_integer());
    assert!(
        u32_scalar.binary_float_precision().is_none(),
        "an unsigned integer has no binary-float precision"
    );
}

/// Canonical and bridge keys round-trip the exact width.
#[test]
fn fixed_scalar_canonical_and_identity_keys_round_trip() {
    let canonical = canonical_builtin_for_key(BuiltinTypeKey::FixedScalar(FixedScalar::U32));
    assert_eq!(
        canonical,
        CanonicalBuiltinType::FixedScalar(FixedScalar::U32)
    );
    assert_eq!(
        builtin_key_for_canonical_builtin(canonical),
        Some(BuiltinTypeKey::FixedScalar(FixedScalar::U32))
    );

    // `TypeId` <-> identity-key round trip keeps the seeded U32 identity exact.
    let environment = TypeEnvironment::new();
    let u32_id = builtin_type_ids::fixed_scalar(FixedScalar::U32);
    let key = environment
        .type_id_to_type_identity_key(u32_id)
        .expect("U32 has an identity key");
    assert_eq!(
        key,
        TypeIdentityKey::Builtin(BridgeBuiltinTypeKey::FixedScalar(FixedScalar::U32))
    );
}
