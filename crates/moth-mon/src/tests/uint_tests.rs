//! Focused `Uint` preparation and map-key index invariants.
//!
//! WHAT: asserts the captured profile width on the prepared `Uint` node and the unsigned
//!       key rendering and duplicate-index facts that only the crate's private owners expose.
//! WHY: public round-trip tests own text, values, defaults and budgets end to end; this file
//!      owns what those paths cannot observe on the prepared node itself.

use moth_lexical::numeric::profile::{FloatPrecision, IntWidth, NumericProfile};

use crate::map_keys::{MapKeyIndex, map_key_name, map_key_name_len};
use crate::schema::PreparedType;
use crate::{Field, PreparedSchema, Schema, SchemaType, Value};

fn profile(int_width: IntWidth) -> NumericProfile {
    NumericProfile {
        int_width,
        float_precision: FloatPrecision::Bits64,
    }
}

fn prepared_uint_field(profile: NumericProfile) -> PreparedType {
    let prepared: PreparedSchema = Schema::record(vec![Field::required("count", SchemaType::Uint)])
        .with_profile(profile)
        .prepare()
        .expect("Uint schema prepares");
    assert_eq!(prepared.profile(), profile);
    let PreparedType::Record { fields } = &prepared.root else {
        panic!("Uint schema prepares a record root");
    };
    fields
        .iter()
        .find(|field| field.name == "count")
        .expect("Uint field prepares")
        .ty
        .clone()
}

#[test]
fn preparation_captures_the_profile_int_width_on_the_uint_node() {
    assert_eq!(
        prepared_uint_field(profile(IntWidth::Bits32)),
        PreparedType::Uint {
            width: IntWidth::Bits32
        },
    );
    assert_eq!(
        prepared_uint_field(profile(IntWidth::Bits64)),
        PreparedType::Uint {
            width: IntWidth::Bits64
        },
    );
    // Standalone consumers that never select a profile prepare the Uint32 node.
    assert_eq!(
        prepared_uint_field(NumericProfile::STANDARD),
        PreparedType::Uint {
            width: IntWidth::Bits32
        },
    );
}

#[test]
fn uint_map_keys_render_digits_and_index_exact_families() {
    assert_eq!(map_key_name(&Value::Uint(u64::MAX)), u64::MAX.to_string());
    assert_eq!(map_key_name_len(&Value::Uint(u64::MAX)), 20);
    assert_eq!(map_key_name_len(&Value::Uint(0)), 1);

    let key_type = PreparedType::Uint {
        width: IntWidth::Bits64,
    };
    let mut seen = MapKeyIndex::new(&key_type);
    assert_eq!(seen.contains_value(&Value::Uint(3)), Some(false));
    // An equal `Int` carrier never probes the `Uint` family.
    assert_eq!(seen.contains_value(&Value::Int(3)), None);
    assert!(seen.insert_value(&Value::Uint(3)));
    assert_eq!(seen.contains_value(&Value::Uint(3)), Some(true));
}
