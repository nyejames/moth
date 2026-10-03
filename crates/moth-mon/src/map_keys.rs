//! Typed duplicate detection and diagnostic rendering for supported MON map keys.

use std::collections::HashSet;

use super::Value;
use super::schema::PreparedType;
use moth_lexical::numeric::fixed_scalar::FixedScalar;

/// Render a supported map key and its byte length for diagnostic paths.
pub(super) fn map_key_name(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Int(number) => number.to_string(),
        Value::I8(number) => number.to_string(),
        Value::I16(number) => number.to_string(),
        Value::I32(number) => number.to_string(),
        Value::I64(number) => number.to_string(),
        Value::U8(number) => number.to_string(),
        Value::U16(number) => number.to_string(),
        Value::U32(number) => number.to_string(),
        Value::U64(number) => number.to_string(),
        Value::Byte(number) => number.to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Char(value) => value.to_string(),
        _ => String::new(),
    }
}

pub(super) fn map_key_name_len(value: &Value) -> usize {
    match value {
        Value::String(text) => text.len(),
        Value::Int(number) => signed_key_len(*number),
        Value::I8(number) => signed_key_len(i64::from(*number)),
        Value::I16(number) => signed_key_len(i64::from(*number)),
        Value::I32(number) => signed_key_len(i64::from(*number)),
        Value::I64(number) => signed_key_len(*number),
        Value::U8(number) => unsigned_key_len(u64::from(*number)),
        Value::U16(number) => unsigned_key_len(u64::from(*number)),
        Value::U32(number) => unsigned_key_len(u64::from(*number)),
        Value::U64(number) => unsigned_key_len(*number),
        Value::Byte(number) => unsigned_key_len(u64::from(*number)),
        Value::Bool(value) => {
            if *value {
                4
            } else {
                5
            }
        }
        Value::Char(value) => value.len_utf8(),
        _ => 0,
    }
}

/// Decimal digit count of a rendered signed key, including its sign.
fn signed_key_len(number: i64) -> usize {
    unsigned_key_len(number.unsigned_abs()) + usize::from(number < 0)
}

/// Decimal digit count of an unsigned magnitude, without allocating its text.
fn unsigned_key_len(mut magnitude: u64) -> usize {
    let mut digits = 1;
    while magnitude >= 10 {
        magnitude /= 10;
        digits += 1;
    }
    digits
}

/// Validation-only duplicate index selected by a prepared map's homogeneous key type.
///
/// The owned [`Value::Map`] output keeps insertion order; this set only answers
/// “have we seen this key” without rescanning prior entries. Matching exact value families
/// prevents equal numeric carriers from aliasing `Int`, `I64`, `U8` or `Byte`.
/// Lookups borrow keys; callers charge the decoded-byte budget before insertion clones strings.
#[derive(Debug)]
pub(super) enum MapKeyIndex {
    String(HashSet<String>),
    Bool(HashSet<bool>),
    Char(HashSet<char>),
    Int(HashSet<i64>),
    I8(HashSet<i8>),
    I16(HashSet<i16>),
    I32(HashSet<i32>),
    I64(HashSet<i64>),
    U8(HashSet<u8>),
    U16(HashSet<u16>),
    U32(HashSet<u32>),
    U64(HashSet<u64>),
    Byte(HashSet<u8>),
}

impl MapKeyIndex {
    pub(super) fn new(key_type: &PreparedType) -> Self {
        match key_type {
            PreparedType::String => Self::String(HashSet::new()),
            PreparedType::Bool => Self::Bool(HashSet::new()),
            PreparedType::Char => Self::Char(HashSet::new()),
            PreparedType::Int { .. } => Self::Int(HashSet::new()),
            PreparedType::Fixed(FixedScalar::I8) => Self::I8(HashSet::new()),
            PreparedType::Fixed(FixedScalar::I16) => Self::I16(HashSet::new()),
            PreparedType::Fixed(FixedScalar::I32) => Self::I32(HashSet::new()),
            PreparedType::Fixed(FixedScalar::I64) => Self::I64(HashSet::new()),
            PreparedType::Fixed(FixedScalar::U8) => Self::U8(HashSet::new()),
            PreparedType::Fixed(FixedScalar::U16) => Self::U16(HashSet::new()),
            PreparedType::Fixed(FixedScalar::U32) => Self::U32(HashSet::new()),
            PreparedType::Fixed(FixedScalar::U64) => Self::U64(HashSet::new()),
            PreparedType::Fixed(FixedScalar::Byte) => Self::Byte(HashSet::new()),
            // Schema preparation rejects unsupported map keys before either value walk.
            _ => unreachable!("prepared map key must have a supported homogeneous family"),
        }
    }

    /// Returns `None` when the value does not match this map's prepared key family.
    pub(super) fn contains_value(&self, value: &Value) -> Option<bool> {
        match (self, value) {
            (Self::String(keys), Value::String(text)) => Some(keys.contains(text)),
            (Self::Bool(keys), Value::Bool(flag)) => Some(keys.contains(flag)),
            (Self::Char(keys), Value::Char(character)) => Some(keys.contains(character)),
            (Self::Int(keys), Value::Int(number)) => Some(keys.contains(number)),
            (Self::I8(keys), Value::I8(number)) => Some(keys.contains(number)),
            (Self::I16(keys), Value::I16(number)) => Some(keys.contains(number)),
            (Self::I32(keys), Value::I32(number)) => Some(keys.contains(number)),
            (Self::I64(keys), Value::I64(number)) => Some(keys.contains(number)),
            (Self::U8(keys), Value::U8(number)) => Some(keys.contains(number)),
            (Self::U16(keys), Value::U16(number)) => Some(keys.contains(number)),
            (Self::U32(keys), Value::U32(number)) => Some(keys.contains(number)),
            (Self::U64(keys), Value::U64(number)) => Some(keys.contains(number)),
            (Self::Byte(keys), Value::Byte(number)) => Some(keys.contains(number)),
            _ => None,
        }
    }

    /// Inserts a matching key, cloning strings only after callers charge the copy.
    pub(super) fn insert_value(&mut self, value: &Value) -> bool {
        match (self, value) {
            (Self::String(keys), Value::String(text)) => keys.insert(text.clone()),
            (Self::Bool(keys), Value::Bool(flag)) => keys.insert(*flag),
            (Self::Char(keys), Value::Char(character)) => keys.insert(*character),
            (Self::Int(keys), Value::Int(number)) => keys.insert(*number),
            (Self::I8(keys), Value::I8(number)) => keys.insert(*number),
            (Self::I16(keys), Value::I16(number)) => keys.insert(*number),
            (Self::I32(keys), Value::I32(number)) => keys.insert(*number),
            (Self::I64(keys), Value::I64(number)) => keys.insert(*number),
            (Self::U8(keys), Value::U8(number)) => keys.insert(*number),
            (Self::U16(keys), Value::U16(number)) => keys.insert(*number),
            (Self::U32(keys), Value::U32(number)) => keys.insert(*number),
            (Self::U64(keys), Value::U64(number)) => keys.insert(*number),
            (Self::Byte(keys), Value::Byte(number)) => keys.insert(*number),
            _ => false,
        }
    }
}
