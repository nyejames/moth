//! Compiler adapters for token text and exact `Dec` values.
//!
//! The shared lexical crate owns numeric grammar, normalized fixed-width parsing and fixed-scalar
//! materialization. This module resolves compiler `StringTable` tokens and hands exact decimal
//! facts to the compiler-owned `NumberValue` coefficient implementation.

use crate::compiler_frontend::datatypes::number::{NumberMaterializationError, NumberValue};
use crate::compiler_frontend::numeric_text::token::NumericLiteralToken;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use moth_lexical::numeric::decimal::NumberScale;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};
use moth_lexical::numeric::grammar::NumericLiteralSign;
use moth_lexical::numeric::parse::{
    NumberLiteralErrorReason, materialize_normalized_fixed_scalar, materialize_normalized_float,
    materialize_normalized_int, parse_numeric_literal,
};
use moth_lexical::numeric::profile::{FloatPrecision, IntWidth};

/// Resolve a token's normalized text and materialize it at the selected boundary `Int` width.
pub(crate) fn materialize_int(
    token: &NumericLiteralToken,
    sign: NumericLiteralSign,
    width: IntWidth,
    string_table: &StringTable,
) -> Result<i64, NumberLiteralErrorReason> {
    materialize_normalized_int(
        string_table.resolve(token.normalized_text),
        sign == NumericLiteralSign::Negative,
        width,
    )
}

/// Resolve a token's normalized text and materialize it at the selected boundary `Float` precision.
pub(crate) fn materialize_float(
    token: &NumericLiteralToken,
    precision: FloatPrecision,
    string_table: &StringTable,
) -> Result<f64, NumberLiteralErrorReason> {
    materialize_normalized_float(
        string_table.resolve(token.normalized_text),
        token.sign == NumericLiteralSign::Negative,
        precision,
    )
}

/// Parse signed numeric text into an exact compiler-owned `NumberValue` at the requested scale.
pub(crate) fn parse_numeric_text_to_number(
    source: &str,
    scale: NumberScale,
) -> Result<NumberValue, NumberLiteralErrorReason> {
    if source.is_empty() {
        return Err(NumberLiteralErrorReason::InvalidSeparatorPlacement);
    }

    let (sign, unsigned) = if let Some(rest) = source.strip_prefix('-') {
        (NumericLiteralSign::Negative, rest)
    } else if source.starts_with('+') {
        return Err(NumberLiteralErrorReason::InvalidSeparatorPlacement);
    } else {
        (NumericLiteralSign::Positive, source)
    };

    if unsigned.is_empty() {
        return Err(NumberLiteralErrorReason::InvalidSeparatorPlacement);
    }

    let parsed = parse_numeric_literal(unsigned)?;
    materialize_normalized_number(&parsed.normalized_text, sign, scale)
}

/// Resolve a token's retained normalized text and materialize its exact `NumberValue`.
pub(crate) fn materialize_number(
    token: &NumericLiteralToken,
    sign: NumericLiteralSign,
    scale: NumberScale,
    string_table: &StringTable,
) -> Result<NumberValue, NumberLiteralErrorReason> {
    materialize_normalized_number(string_table.resolve(token.normalized_text), sign, scale)
}

fn materialize_normalized_number(
    normalized: &str,
    sign: NumericLiteralSign,
    scale: NumberScale,
) -> Result<NumberValue, NumberLiteralErrorReason> {
    NumberValue::from_normalized(normalized, sign, scale).map_err(|error| match error {
        NumberMaterializationError::InexactScale => {
            NumberLiteralErrorReason::InexactNumberScale(scale)
        }
        NumberMaterializationError::Capacity => NumberLiteralErrorReason::ParseOverflow,
    })
}

/// Resolve a token's normalized text and materialize at a fixed-width scalar destination.
pub(crate) fn materialize_fixed_scalar(
    token: &NumericLiteralToken,
    sign: NumericLiteralSign,
    scalar: FixedScalar,
    string_table: &StringTable,
) -> Result<FixedScalarValue, NumberLiteralErrorReason> {
    materialize_normalized_fixed_scalar(
        string_table.resolve(token.normalized_text),
        sign == NumericLiteralSign::Negative,
        scalar,
    )
}

#[cfg(test)]
#[path = "tests/parse_tests.rs"]
mod tests;
