//! Exact decimal receiver and scale-boundary parity.
//!
//! WHAT: exercises retained decimal spelling against source `DecN` fields and prepared MON schemas.
//! WHY: Dec materialisation is exact at its declared scale and must not expand huge nonzero values.

use crate::compiler_frontend::instrumentation::lock_counter_test;
use crate::mon::{Field, MonErrorCode, PathSegment, Schema, SchemaType, Value};
use moth_lexical::numeric::decimal::NumberScale;
use moth_lexical::numeric::parse::NumberLiteralErrorReason;

use super::support::{
    MonExpectation, ParityFixture, SourceExpectation, assert_fixture, fixture, in_declarations,
    in_literal, mon_numeric_error, source_invalid_number, source_unknown_type_name,
};

fn decimal_schema(scale: u16) -> Schema {
    Schema::record(vec![Field::required(
        "value",
        SchemaType::Struct {
            name: "Box".into(),
            fields: vec![Field::required("number", SchemaType::Decimal { scale })],
        },
    )])
}

fn decimal_expected(text: &str) -> Value {
    Value::Record(vec![(
        "value".into(),
        Value::Record(vec![("number".into(), Value::Decimal(text.into()))]),
    )])
}

fn decimal_type_name(scale: u16) -> String {
    if scale == 0 {
        "Dec".to_owned()
    } else {
        format!("Dec{scale}")
    }
}

fn accepted_decimal(
    name: &str,
    spelling: &str,
    scale: u16,
    expected_text: &str,
    expected_coefficient: &str,
) -> ParityFixture {
    let type_name = decimal_type_name(scale);
    fixture(
        name,
        format!("value = Box(number = {spelling})"),
        format!("Box = | number {type_name} |\n"),
        decimal_schema(scale),
        SourceExpectation::Accept,
        MonExpectation::Accept(decimal_expected(expected_text)),
    )
    .with_source_nominal_type("value", "Box")
    .with_source_decimal("value.number", expected_coefficient, scale)
}

fn inexact_decimal(name: &str, spelling: &str, scale: u16) -> ParityFixture {
    let literal = format!("value = Box(number = {spelling})");
    let scale_identity = NumberScale::new(scale).expect("test scale is in the supported range");
    fixture(
        name,
        literal.clone(),
        format!("Box = | number {} |\n", decimal_type_name(scale)),
        decimal_schema(scale),
        source_invalid_number(
            NumberLiteralErrorReason::InexactNumberScale(scale_identity),
            in_literal(&literal, spelling, 1),
        ),
        mon_numeric_error(
            &literal,
            spelling,
            1,
            MonErrorCode::NumericScale,
            vec![
                PathSegment::Field("value".into()),
                PathSegment::Field("number".into()),
            ],
        ),
    )
}

fn scale_fixtures() -> Vec<ParityFixture> {
    vec![
        accepted_decimal("dec0_whole", "123", 0, "123", "123"),
        accepted_decimal("dec1_fraction", "12.3", 1, "12.3", "123"),
        accepted_decimal("dec2_fraction", "1.23", 2, "1.23", "123"),
        accepted_decimal(
            "dec18_exact_fraction",
            "0.123456789012345678",
            18,
            "0.123456789012345678",
            "123456789012345678",
        ),
        accepted_decimal(
            "dec19_exact_fraction",
            "0.1234567890123456789",
            19,
            "0.1234567890123456789",
            "1234567890123456789",
        ),
        accepted_decimal("dec255_exact_exponent", "1e-255", 255, "1e-255", "1"),
        accepted_decimal("dec256_exact_exponent", "1e-256", 256, "1e-256", "1"),
    ]
}

fn exactness_fixtures() -> Vec<ParityFixture> {
    vec![
        accepted_decimal(
            "dec2_trailing_fractional_zeroes",
            "1.2300",
            2,
            "1.2300",
            "123",
        ),
        accepted_decimal("dec2_negative_trailing_zeroes", "-1.20", 2, "-1.20", "-120"),
        accepted_decimal("dec0_positive_exponent", "1.2e+1", 0, "1.2e+1", "12"),
        accepted_decimal("dec2_negative_exponent", "123e-2", 2, "123e-2", "123"),
        accepted_decimal(
            "dec2_exponent_and_trailing_zeroes",
            "12.30e-1",
            2,
            "12.30e-1",
            "123",
        ),
        accepted_decimal(
            "dec0_trailing_zeroes_reduce_scale",
            "1000e-3",
            0,
            "1000e-3",
            "1",
        ),
        accepted_decimal(
            "dec0_very_large_positive_zero_exponent",
            "0e+99999999999999999999",
            0,
            "0e+99999999999999999999",
            "0",
        ),
        accepted_decimal(
            "dec0_very_large_negative_zero_exponent",
            "0e-99999999999999999999",
            0,
            "0e-99999999999999999999",
            "0",
        ),
        inexact_decimal("dec2_inexact_fraction", "1.239", 2),
        inexact_decimal("dec2_inexact_negative_exponent", "1e-3", 2),
    ]
}

fn scale_257_fixture() -> ParityFixture {
    let declarations = "amount Dec257 = 0\n";
    fixture(
        "dec257_source_declaration_and_mon_schema_reject",
        "amount = 0",
        declarations,
        Schema::record(vec![Field::required(
            "amount",
            SchemaType::Decimal { scale: 257 },
        )]),
        source_unknown_type_name(in_declarations(declarations, "Dec257", 1)),
        MonExpectation::SchemaRejected {
            code: MonErrorCode::NumericScale,
            path: vec![PathSegment::Field("amount".into())],
        },
    )
}

#[test]
fn exact_decimal_scales_and_spellings_match_source_and_mon() {
    let _guard = lock_counter_test();
    for fixture in scale_fixtures()
        .into_iter()
        .chain(exactness_fixtures())
        .chain(std::iter::once(scale_257_fixture()))
    {
        assert_fixture(fixture);
    }
}
