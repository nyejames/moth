//! Focused type-identity tests for the fixed-width builtin scalars.
//!
//! WHAT: proves `I8`..`F64` and `Byte` are seeded as distinct builtin identities that never alias
//!       `Int`/`Float`, each other, or `Char`, including through canonical and generic identity.
//! WHY: identity is the whole contract of these types; a width that resolved to `Int`, or a
//!      profile width that made `Float` alias `F32`, would silently break the casts, operators
//!      and foreign boundaries that later phases build on canonical identity.

use crate::compiler_frontend::canonical_type_identity::{
    CanonicalBuiltinType, builtin_type_id_for_canonical_builtin, canonical_builtin_for_key,
};
use crate::compiler_frontend::datatypes::builtin_type_ids;
use crate::compiler_frontend::datatypes::definitions::{StructTypeDefinition, TypeDefinition};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::fixed_scalar::FixedScalar;
use crate::compiler_frontend::datatypes::generic_identity_bridge::{
    BuiltinTypeKey as BridgeBuiltinTypeKey, GenericInstantiationKey, TypeIdentityKey,
};
use crate::compiler_frontend::datatypes::ids::{BuiltinTypeKey, NominalTypeId, TypeId};
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerBuilder};
use crate::compiler_frontend::symbols::string_interning::StringTable;

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
fn canonical_identity_mapping_is_injective_and_matches_seeded_ids() {
    let canonical_scalars: Vec<CanonicalBuiltinType> = FixedScalar::ALL
        .into_iter()
        .map(CanonicalBuiltinType::FixedScalar)
        .collect();

    for (index, scalar) in FixedScalar::ALL.into_iter().enumerate() {
        let canonical = canonical_builtin_for_key(BuiltinTypeKey::FixedScalar(scalar));
        assert_eq!(canonical, canonical_scalars[index]);
        assert_eq!(
            builtin_type_id_for_canonical_builtin(canonical),
            Some(builtin_type_ids::fixed_scalar(scalar)),
            "{} must round-trip through its canonical identity",
            scalar.name()
        );
        assert!(
            !canonical_scalars[..index].contains(&canonical),
            "{} must not share a canonical identity with another scalar",
            scalar.name()
        );
    }

    // `Int`/`Float` canonical identity is a different variant, so a matching width can never
    // unify them with a fixed width.
    assert_ne!(
        canonical_builtin_for_key(BuiltinTypeKey::Int),
        canonical_builtin_for_key(BuiltinTypeKey::FixedScalar(FixedScalar::I32))
    );
    assert_ne!(
        canonical_builtin_for_key(BuiltinTypeKey::Float),
        canonical_builtin_for_key(BuiltinTypeKey::FixedScalar(FixedScalar::F32))
    );
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
