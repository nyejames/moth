//! Focused type-identity tests for the fixed-width builtin scalars.
//!
//! WHAT: proves `I8`..`F64` and `Byte` are seeded as distinct builtin identities that never alias
//!       `Int`/`Float`, each other, or `Char`, including through canonical and generic identity.
//! WHY: identity is the whole contract of these types; a width that resolved to `Int`, or a
//!      profile width that made `Float` alias `F32`, would silently break the casts, operators
//!      and foreign boundaries that later phases build on canonical identity.

use crate::compiler_frontend::canonical_type_identity::{
    CanonicalBuiltinType, intern_canonical_builtin,
};
use crate::compiler_frontend::datatypes::builtin_type_ids;
use crate::compiler_frontend::datatypes::definitions::{StructTypeDefinition, TypeDefinition};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::generic_identity_bridge::{
    BuiltinTypeKey as BridgeBuiltinTypeKey, GenericInstantiationKey, TypeIdentityKey,
};
use crate::compiler_frontend::datatypes::ids::{BuiltinTypeKey, NominalTypeId, TypeId};
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
    assert_ne!(
        builtin_type_ids::INT,
        builtin_type_ids::fixed_scalar(FixedScalar::I32)
    );
    assert_ne!(
        builtin_type_ids::FLOAT,
        builtin_type_ids::fixed_scalar(FixedScalar::F32)
    );
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
