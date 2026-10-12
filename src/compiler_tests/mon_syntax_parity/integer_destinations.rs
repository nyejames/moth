//! Integer receiver boundaries and exact-width identity.
//!
//! WHAT: exercises every fixed integer width and `Byte` through typed nominal fields.
//! WHY: boundaries and retained numeric identities belong to the receiving type, not the spelling.

use crate::compiler_frontend::compiler_messages::TypeMismatchContext;
use crate::compiler_frontend::instrumentation::lock_counter_test;
use crate::mon::{Field, MonErrorCode, PathSegment, Schema, SchemaType, Value};
use moth_lexical::numeric::fixed_scalar::FixedScalar;
use moth_lexical::numeric::parse::NumberLiteralErrorReason;

use super::support::{
    MonExpectation, ParityFixture, SourceExpectation, assert_fixture, fixture, in_literal,
    mon_numeric_error, source_invalid_number, source_type_mismatch,
};

fn single_box_schema(number_type: SchemaType) -> Schema {
    Schema::record(vec![Field::required(
        "value",
        SchemaType::Struct {
            name: "Box".into(),
            fields: vec![Field::required("number", number_type)],
        },
    )])
}

fn single_box_value(number: Value) -> Value {
    Value::Record(vec![(
        "value".into(),
        Value::Record(vec![("number".into(), number)]),
    )])
}

fn schema_type(type_name: &str) -> SchemaType {
    match type_name {
        "I8" => SchemaType::I8,
        "I16" => SchemaType::I16,
        "I32" => SchemaType::I32,
        "I64" => SchemaType::I64,
        "U8" => SchemaType::U8,
        "U16" => SchemaType::U16,
        "U32" => SchemaType::U32,
        "U64" => SchemaType::U64,
        "Byte" => SchemaType::Byte,
        _ => panic!("unsupported integer parity type {type_name}"),
    }
}

fn accepted_number(name: &str, spelling: &str, type_name: &str, value: Value) -> ParityFixture {
    let literal = format!("value = Box(number = {spelling})");
    fixture(
        name,
        literal,
        format!("Box = | number {type_name} |\n"),
        single_box_schema(schema_type(type_name)),
        SourceExpectation::Accept,
        MonExpectation::Accept(single_box_value(value)),
    )
    .with_source_nominal_type("value", "Box")
}

fn rejected_number(
    name: &str,
    spelling: &str,
    type_name: &str,
    source: SourceExpectation,
    mon_code: MonErrorCode,
) -> ParityFixture {
    let literal = format!("value = Box(number = {spelling})");
    fixture(
        name,
        literal.clone(),
        format!("Box = | number {type_name} |\n"),
        single_box_schema(schema_type(type_name)),
        source,
        mon_numeric_error(
            &literal,
            spelling,
            1,
            mon_code,
            vec![
                PathSegment::Field("value".into()),
                PathSegment::Field("number".into()),
            ],
        ),
    )
    .with_source_nominal_type("value", "Box")
}

fn fixed_integer_boundary_fixtures() -> Vec<ParityFixture> {
    let mut fixtures = Vec::new();
    for (type_name, scalar, minimum_text, minimum, maximum_text, maximum, below, above) in [
        (
            "I8",
            FixedScalar::I8,
            "-128",
            Value::I8(-128),
            "127",
            Value::I8(127),
            "-129",
            "128",
        ),
        (
            "I16",
            FixedScalar::I16,
            "-32768",
            Value::I16(-32_768),
            "32767",
            Value::I16(32_767),
            "-32769",
            "32768",
        ),
        (
            "I32",
            FixedScalar::I32,
            "-2147483648",
            Value::I32(i32::MIN),
            "2147483647",
            Value::I32(i32::MAX),
            "-2147483649",
            "2147483648",
        ),
        (
            "I64",
            FixedScalar::I64,
            "-9223372036854775808",
            Value::I64(i64::MIN),
            "9223372036854775807",
            Value::I64(i64::MAX),
            "-9223372036854775809",
            "9223372036854775808",
        ),
    ] {
        fixtures.push(accepted_number(
            &format!("{type_name}_minimum"),
            minimum_text,
            type_name,
            minimum,
        ));
        fixtures.push(accepted_number(
            &format!("{type_name}_maximum"),
            maximum_text,
            type_name,
            maximum,
        ));
        for (suffix, spelling) in [("below_minimum", below), ("above_maximum", above)] {
            let literal = format!("value = Box(number = {spelling})");
            fixtures.push(rejected_number(
                &format!("{type_name}_{suffix}"),
                spelling,
                type_name,
                source_invalid_number(
                    NumberLiteralErrorReason::OutsideFixedScalarRange(scalar),
                    in_literal(&literal, spelling, 1),
                ),
                MonErrorCode::NumericRange,
            ));
        }
    }

    for (type_name, scalar, maximum_text, zero, maximum, above) in [
        (
            "U8",
            FixedScalar::U8,
            "255",
            Value::U8(0),
            Value::U8(255),
            "256",
        ),
        (
            "U16",
            FixedScalar::U16,
            "65535",
            Value::U16(0),
            Value::U16(65_535),
            "65536",
        ),
        (
            "U32",
            FixedScalar::U32,
            "4294967295",
            Value::U32(0),
            Value::U32(u32::MAX),
            "4294967296",
        ),
        (
            "U64",
            FixedScalar::U64,
            "18446744073709551615",
            Value::U64(0),
            Value::U64(u64::MAX),
            "18446744073709551616",
        ),
    ] {
        fixtures.push(accepted_number(
            &format!("{type_name}_minimum"),
            "0",
            type_name,
            zero,
        ));
        fixtures.push(accepted_number(
            &format!("{type_name}_maximum"),
            maximum_text,
            type_name,
            maximum,
        ));
        let negative = "-1";
        let literal = format!("value = Box(number = {negative})");
        fixtures.push(rejected_number(
            &format!("{type_name}_negative"),
            negative,
            type_name,
            source_invalid_number(
                NumberLiteralErrorReason::NegativeUnsignedLiteral(scalar),
                in_literal(&literal, negative, 1),
            ),
            MonErrorCode::NumericRange,
        ));
        let literal = format!("value = Box(number = {above})");
        fixtures.push(rejected_number(
            &format!("{type_name}_above_maximum"),
            above,
            type_name,
            source_invalid_number(
                NumberLiteralErrorReason::OutsideFixedScalarRange(scalar),
                in_literal(&literal, above, 1),
            ),
            MonErrorCode::NumericRange,
        ));
    }

    fixtures.push(accepted_number("byte_zero", "0", "Byte", Value::Byte(0)));
    fixtures.push(accepted_number(
        "byte_maximum",
        "255",
        "Byte",
        Value::Byte(255),
    ));
    for (name, spelling, reason) in [
        (
            "byte_negative_one",
            "-1",
            NumberLiteralErrorReason::NegativeUnsignedLiteral(FixedScalar::Byte),
        ),
        (
            "byte_negative_zero_spelling",
            "-0",
            NumberLiteralErrorReason::NegativeUnsignedLiteral(FixedScalar::Byte),
        ),
        (
            "byte_above_maximum",
            "256",
            NumberLiteralErrorReason::OutsideFixedScalarRange(FixedScalar::Byte),
        ),
        (
            "u8_negative_zero_spelling",
            "-0",
            NumberLiteralErrorReason::NegativeUnsignedLiteral(FixedScalar::U8),
        ),
        (
            "u32_negative_zero_spelling",
            "-0",
            NumberLiteralErrorReason::NegativeUnsignedLiteral(FixedScalar::U32),
        ),
        (
            "u64_negative_zero_spelling",
            "-0",
            NumberLiteralErrorReason::NegativeUnsignedLiteral(FixedScalar::U64),
        ),
        (
            "u32_above_signed_64_bit_maximum",
            "9223372036854775808",
            NumberLiteralErrorReason::OutsideFixedScalarRange(FixedScalar::U32),
        ),
    ] {
        let type_name = match reason {
            NumberLiteralErrorReason::NegativeUnsignedLiteral(scalar)
            | NumberLiteralErrorReason::OutsideFixedScalarRange(scalar) => scalar.name(),
            _ => unreachable!("every row names its receiving scalar"),
        };
        let literal = format!("value = Box(number = {spelling})");
        fixtures.push(rejected_number(
            name,
            spelling,
            type_name,
            source_invalid_number(reason, in_literal(&literal, spelling, 1)),
            MonErrorCode::NumericRange,
        ));
    }

    for (name, spelling, type_name) in [
        ("decimal_spelling_into_i8", "1.0", "I8"),
        ("exponent_spelling_into_u16", "1e0", "U16"),
        ("decimal_spelling_into_byte", "1.0", "Byte"),
        ("exponent_spelling_into_byte", "1e2", "Byte"),
    ] {
        let literal = format!("value = Box(number = {spelling})");
        fixtures.push(rejected_number(
            name,
            spelling,
            type_name,
            source_type_mismatch(
                TypeMismatchContext::ConstructorArgument,
                in_literal(&literal, spelling, 1),
            ),
            MonErrorCode::NumericType,
        ));
    }

    // Values above binary64's exact-integer range remain exact in integer receivers.
    fixtures.push(accepted_number(
        "i64_above_binary64_exact_integer_range",
        "-9007199254740993",
        "I64",
        Value::I64(-9_007_199_254_740_993),
    ));
    fixtures.push(accepted_number(
        "u64_above_binary64_exact_integer_range",
        "9007199254740993",
        "U64",
        Value::U64(9_007_199_254_740_993),
    ));

    fixtures
}

/// Destination rows that the fixed boundary sets above do not already cover.
///
/// WHAT: one exact `I32` receiver rejecting a magnitude only `I64` can hold, and one exact `I64`
///       receiver accepting that same spelling.
/// WHY:  value size never selects a type, so a narrow receiver diagnoses the literal instead of
///       retrying it wider. The exact `I32` and `I64` rows complement the fixed boundary rows
///       from `fixed_integer_boundary_fixtures`.
fn exact_int_destination_fixtures() -> Vec<ParityFixture> {
    let mut fixtures = Vec::new();
    let beyond_binary64_exact = "9007199254740993";
    let literal = format!("value = Box(number = {beyond_binary64_exact})");
    fixtures.push(rejected_number(
        "i32_rejects_above_binary64_exact_integer_range",
        beyond_binary64_exact,
        "I32",
        source_invalid_number(
            NumberLiteralErrorReason::OutsideFixedScalarRange(FixedScalar::I32),
            in_literal(&literal, beyond_binary64_exact, 1),
        ),
        MonErrorCode::NumericRange,
    ));
    fixtures.push(accepted_number(
        "i64_accepts_above_binary64_exact_integer_range",
        beyond_binary64_exact,
        "I64",
        Value::I64(9_007_199_254_740_993),
    ));
    fixtures
}

/// Unsigned destination rows that the fixed boundary sets above do not already cover.
///
/// WHY: an unsigned receiver accepts magnitudes above the signed maximum at its own exact width;
///      its zero, maximum, negative and above-maximum rows already live in the fixed sets.
fn exact_uint_destination_fixtures() -> Vec<ParityFixture> {
    vec![
        accepted_number(
            "u32_accepts_signed_max_plus_one",
            "2147483648",
            "U32",
            Value::U32(2_147_483_648),
        ),
        accepted_number(
            "u64_accepts_above_signed_max",
            "9223372036854775808",
            "U64",
            Value::U64(9_223_372_036_854_775_808),
        ),
    ]
}

fn uint_spelling_fixtures() -> Vec<ParityFixture> {
    let mut fixtures = Vec::new();
    fixtures.push(accepted_number(
        "uint_grouped_whole_spelling",
        "1_0",
        "U32",
        Value::U32(10),
    ));
    for (name, spelling) in [
        ("decimal_spelling_into_uint", "1.0"),
        ("exponent_spelling_into_uint", "1e2"),
    ] {
        let literal = format!("value = Box(number = {spelling})");
        fixtures.push(rejected_number(
            name,
            spelling,
            "U32",
            source_type_mismatch(
                TypeMismatchContext::ConstructorArgument,
                in_literal(&literal, spelling, 1),
            ),
            MonErrorCode::NumericType,
        ));
    }
    fixtures
}

/// One exact unsigned pair proving `U32` and `U64` receivers keep distinct identities.
fn uint_width_identity_fixture() -> ParityFixture {
    let literal = "value = UintPair(narrow = 42, wide = 42)";
    fixture(
        "unsigned_widths_stay_distinct",
        literal,
        "UintPair = | narrow U32, wide U64 |\n",
        Schema::record(vec![Field::required(
            "value",
            SchemaType::Struct {
                name: "UintPair".into(),
                fields: vec![
                    Field::required("narrow", SchemaType::U32),
                    Field::required("wide", SchemaType::U64),
                ],
            },
        )]),
        SourceExpectation::Accept,
        MonExpectation::Accept(Value::Record(vec![(
            "value".into(),
            Value::Record(vec![
                ("narrow".into(), Value::U32(42)),
                ("wide".into(), Value::U64(42)),
            ]),
        )])),
    )
    .with_source_nominal_type("value", "UintPair")
}

/// One exact mixed-width pair proving `I32`, `U64` and `Byte` receivers keep distinct identities.
fn fixed_width_identity_fixture() -> ParityFixture {
    let literal = "value = FixedSet(signed = -1, wide = 9007199254740993, octet = 255)";
    let fields = vec![
        Field::required("signed", SchemaType::I32),
        Field::required("wide", SchemaType::U64),
        Field::required("octet", SchemaType::Byte),
    ];
    fixture(
        "fixed_width_identities_stay_distinct",
        literal,
        "FixedSet = | signed I32, wide U64, octet Byte |\n",
        Schema::record(vec![Field::required(
            "value",
            SchemaType::Struct {
                name: "FixedSet".into(),
                fields,
            },
        )]),
        SourceExpectation::Accept,
        MonExpectation::Accept(Value::Record(vec![(
            "value".into(),
            Value::Record(vec![
                ("signed".into(), Value::I32(-1)),
                ("wide".into(), Value::U64(9_007_199_254_740_993)),
                ("octet".into(), Value::Byte(255)),
            ]),
        )])),
    )
    .with_source_nominal_type("value", "FixedSet")
}

/// One exact signed pair proving `I32` and `I64` receivers keep distinct identities instead of
/// both collapsing onto a single signed width.
fn signed_width_identity_fixture() -> ParityFixture {
    let literal = "value = IntPair(first = 42, second = 42)";
    fixture(
        "signed_widths_stay_distinct",
        literal,
        "IntPair = | first I32, second I64 |\n",
        Schema::record(vec![Field::required(
            "value",
            SchemaType::Struct {
                name: "IntPair".into(),
                fields: vec![
                    Field::required("first", SchemaType::I32),
                    Field::required("second", SchemaType::I64),
                ],
            },
        )]),
        SourceExpectation::Accept,
        MonExpectation::Accept(Value::Record(vec![(
            "value".into(),
            Value::Record(vec![
                ("first".into(), Value::I32(42)),
                ("second".into(), Value::I64(42)),
            ]),
        )])),
    )
    .with_source_nominal_type("value", "IntPair")
}

#[test]
fn fixed_integer_and_byte_boundaries_reject_out_of_range_values() {
    let _guard = lock_counter_test();
    for fixture in fixed_integer_boundary_fixtures() {
        assert_fixture(fixture);
    }
}

#[test]
fn exact_int_destinations_keep_declared_width_and_identity() {
    let _guard = lock_counter_test();
    for fixture in exact_int_destination_fixtures() {
        assert_fixture(fixture);
    }
}

#[test]
fn exact_uint_destinations_keep_declared_width_and_identity() {
    let _guard = lock_counter_test();
    for fixture in exact_uint_destination_fixtures() {
        assert_fixture(fixture);
    }
}

#[test]
fn uint_spellings_keep_whole_number_category_on_both_sides() {
    let _guard = lock_counter_test();
    for fixture in uint_spelling_fixtures() {
        assert_fixture(fixture);
    }
}

#[test]
fn unsigned_widths_stay_distinct() {
    let _guard = lock_counter_test();
    assert_fixture(uint_width_identity_fixture());
}

#[test]
fn fixed_width_identities_are_exact() {
    let _guard = lock_counter_test();
    assert_fixture(fixed_width_identity_fixture());
    assert_fixture(signed_width_identity_fixture());
}
