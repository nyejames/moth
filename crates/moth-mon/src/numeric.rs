//! Conversion between shared neutral numeric carriers and owned MON values.

use super::Value;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};

/// The owned public value for one materialised fixed scalar.
///
/// WHY: `FixedScalarValue` owns the exact range and rounding decisions, but its payload is a
///      byte-exact carrier that the public tree does not expose, so decoded values are copied into
///      their own Rust width here.
pub(super) fn fixed_scalar_to_value(materialised: FixedScalarValue) -> Value {
    match materialised.scalar() {
        FixedScalar::I8 => Value::I8(signed_payload(materialised) as i8),
        FixedScalar::I16 => Value::I16(signed_payload(materialised) as i16),
        FixedScalar::I32 => Value::I32(signed_payload(materialised) as i32),
        FixedScalar::I64 => Value::I64(signed_payload(materialised)),
        FixedScalar::U8 => Value::U8(unsigned_payload(materialised) as u8),
        FixedScalar::U16 => Value::U16(unsigned_payload(materialised) as u16),
        FixedScalar::U32 => Value::U32(unsigned_payload(materialised) as u32),
        FixedScalar::U64 => Value::U64(unsigned_payload(materialised)),
        FixedScalar::F16 => Value::F16(float_payload(materialised)),
        FixedScalar::F32 => Value::F32(float_payload(materialised)),
        FixedScalar::F64 => Value::F64(float_payload(materialised)),
        FixedScalar::Byte => Value::Byte(unsigned_payload(materialised) as u8),
    }
}

/// The payload of a materialised fixed scalar that already matches its own scalar identity.
///
/// WHAT: copies the payload of `value` for a fixed scalar that the caller has already checked
///       against the receiving schema, rounding a binary-float payload once at the scalar's own
///       precision and rejecting a non-finite result.
/// WHY:  completion, default preparation and map-key checks must agree on what “this value is
///       valid for this fixed scalar” means, and a fixed binary float materialises with its own
///       rounding contract exactly as its literal does.
pub(super) fn materialize_fixed_value(value: &Value, scalar: FixedScalar) -> Option<Value> {
    let Some(precision) = scalar.binary_float_precision() else {
        // Integer and octet payloads are `Copy` and already exact in their own carrier width.
        return Some(value.clone());
    };
    let payload = match value {
        Value::F16(payload) | Value::F32(payload) | Value::F64(payload) => *payload,
        _ => return None,
    };
    let rounded = precision.round(payload);
    if !rounded.is_finite() {
        return None;
    }
    match scalar {
        FixedScalar::F16 => Some(Value::F16(rounded)),
        FixedScalar::F32 => Some(Value::F32(rounded)),
        FixedScalar::F64 => Some(Value::F64(rounded)),
        _ => None,
    }
}

/// The signed payload of a materialised signed fixed scalar.
fn signed_payload(materialised: FixedScalarValue) -> i64 {
    materialised
        .as_i64()
        .expect("a signed fixed scalar carries a signed payload")
}

/// The unsigned payload of a materialised unsigned or octet fixed scalar.
fn unsigned_payload(materialised: FixedScalarValue) -> u64 {
    materialised
        .as_u64()
        .expect("an unsigned fixed scalar carries an unsigned payload")
}

/// The binary-float payload of a materialised fixed binary float.
fn float_payload(materialised: FixedScalarValue) -> f64 {
    materialised
        .as_f64()
        .expect("a fixed binary float carries a float payload")
}
