//! `Uint` identity and range-fact tests for the canonical numeric scalar.
//!
//! WHAT: pins `NumericScalar::Uint` as a builtin source word distinct from
//!       `Int`/`U32`/`U64`, deriving its range from the selected profile.
//! WHY: Uint transport keeps exact identity separate from a reused unsigned
//!      carrier; these tests own the identity/range contract.

use super::super::numeric_scalar::NumericScalar;
use moth_lexical::numeric::fixed_scalar::FixedScalar;
use moth_lexical::numeric::profile::{FloatPrecision, IntWidth, NumericProfile};

fn profile(int_width: IntWidth) -> NumericProfile {
    NumericProfile {
        int_width,
        float_precision: FloatPrecision::Bits64,
    }
}

#[test]
fn uint_range_follows_the_selected_int_width() {
    assert_eq!(
        NumericScalar::Uint.integer_range(profile(IntWidth::Bits32)),
        Some((0, i128::from(u32::MAX)))
    );
    assert_eq!(
        NumericScalar::Uint.integer_range(profile(IntWidth::Bits64)),
        Some((0, i128::from(u64::MAX)))
    );
}

#[test]
fn uint_range_is_distinct_from_int_and_fixed_unsigned() {
    let bits32 = profile(IntWidth::Bits32);
    assert_ne!(
        NumericScalar::Uint.integer_range(bits32),
        NumericScalar::Int.integer_range(bits32),
        "Uint starts at zero while Int starts negative"
    );
    assert_eq!(
        NumericScalar::Uint.integer_range(bits32),
        NumericScalar::Fixed(FixedScalar::U32).integer_range(bits32),
        "Uint32 shares the U32 range fact while keeping a distinct identity"
    );
    assert_ne!(
        NumericScalar::Uint,
        NumericScalar::Fixed(FixedScalar::U32),
        "shared range fact must not collapse Uint into U32"
    );
    assert_ne!(
        NumericScalar::Uint,
        NumericScalar::Fixed(FixedScalar::U64),
        "shared range fact must not collapse Uint into U64"
    );
}

#[test]
fn uint_is_an_integer_but_not_a_binary_float() {
    assert!(NumericScalar::Uint.is_integer());
    assert!(
        NumericScalar::Uint
            .binary_float_precision(NumericProfile::STANDARD)
            .is_none()
    );
}

#[test]
fn uint_canonical_builtin_round_trips_through_key() {
    use crate::compiler_frontend::canonical_type_identity::{
        CanonicalBuiltinType, builtin_key_for_canonical_builtin, canonical_builtin_for_key,
    };
    use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
    use crate::compiler_frontend::datatypes::ids::BuiltinTypeKey;

    let canonical = canonical_builtin_for_key(BuiltinTypeKey::Uint);
    assert_eq!(canonical, CanonicalBuiltinType::Uint);
    assert_eq!(
        builtin_key_for_canonical_builtin(canonical),
        Some(BuiltinTypeKey::Uint)
    );

    // `TypeId` <-> identity-key round trip keeps the seeded Uint identity exact.
    use crate::compiler_frontend::datatypes::generic_identity_bridge::BuiltinTypeKey as BridgeKey;
    let env = TypeEnvironment::new();
    let uint_id = env.builtins().uint;
    let key = env
        .type_id_to_type_identity_key(uint_id)
        .expect("Uint has an identity key");
    assert_eq!(
        key,
        crate::compiler_frontend::datatypes::generic_identity_bridge::TypeIdentityKey::Builtin(
            BridgeKey::Uint
        )
    );
}
