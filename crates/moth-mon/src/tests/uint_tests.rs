//! Focused unsigned preparation and map-key index invariants.
//!
//! WHAT: asserts the prepared fixed node for each unsigned schema type and the unsigned key
//!       rendering and duplicate-index facts that only the crate's private owners expose.
//! WHY: public round-trip tests own text, values, defaults and budgets end to end; this file
//!      owns what those paths cannot observe on the prepared node itself.

use moth_lexical::numeric::fixed_scalar::FixedScalar;

use crate::map_keys::{MapKeyIndex, map_key_name, map_key_name_len};
use crate::schema::PreparedType;
use crate::{Field, Schema, SchemaType, Value};

fn prepared_field_type(ty: SchemaType) -> PreparedType {
    let prepared = Schema::record(vec![Field::required("count", ty)])
        .prepare()
        .expect("unsigned schema prepares");
    let PreparedType::Record { fields } = &prepared.root else {
        panic!("unsigned schema prepares a record root");
    };
    fields
        .iter()
        .find(|field| field.name == "count")
        .expect("unsigned field prepares")
        .ty
        .clone()
}

#[test]
fn preparation_keeps_each_unsigned_width_on_its_fixed_node() {
    assert_eq!(
        prepared_field_type(SchemaType::U32),
        PreparedType::Fixed(FixedScalar::U32),
    );
    assert_eq!(
        prepared_field_type(SchemaType::U64),
        PreparedType::Fixed(FixedScalar::U64),
    );
}

#[test]
fn uint_map_keys_render_digits_and_index_exact_families() {
    assert_eq!(map_key_name(&Value::U64(u64::MAX)), u64::MAX.to_string());
    assert_eq!(map_key_name_len(&Value::U64(u64::MAX)), 20);
    assert_eq!(map_key_name_len(&Value::U64(0)), 1);

    let key_type = PreparedType::Fixed(FixedScalar::U64);
    let mut seen = MapKeyIndex::new(&key_type);
    assert_eq!(seen.contains_value(&Value::U64(3)), Some(false));
    // An equal signed carrier never probes the unsigned family.
    assert_eq!(seen.contains_value(&Value::I64(3)), None);
    assert!(seen.insert_value(&Value::U64(3)));
    assert_eq!(seen.contains_value(&Value::U64(3)), Some(true));
}
