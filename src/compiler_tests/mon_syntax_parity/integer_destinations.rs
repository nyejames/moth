//! Integer receiver boundaries and numeric-profile parity.
//!
//! WHAT: exercises `Int`, every fixed integer width and `Byte` through typed nominal fields.
//! WHY: boundaries and retained numeric identities belong to the receiving type, not the spelling.

use crate::compiler_frontend::compiler_messages::TypeMismatchContext;
use crate::compiler_frontend::instrumentation::lock_counter_test;
use crate::mon::{Field, MonErrorCode, PathSegment, Schema, SchemaType, Value};
use moth_lexical::numeric::fixed_scalar::FixedScalar;
use moth_lexical::numeric::parse::NumberLiteralErrorReason;
use moth_lexical::numeric::profile::{FloatPrecision, IntWidth, NumericProfile};

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
        "Int" => SchemaType::Int,
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

fn accepted_number(
    name: &str,
    spelling: &str,
    type_name: &str,
    value: Value,
    profile: Option<NumericProfile>,
) -> ParityFixture {
    let literal = format!("value = Box(number = {spelling})");
    let fixture = fixture(
        name,
        literal,
        format!("Box = | number {type_name} |\n"),
        single_box_schema(schema_type(type_name)),
        SourceExpectation::Accept,
        MonExpectation::Accept(single_box_value(value)),
    )
    .with_source_nominal_type("value", "Box");
    match profile {
        Some(profile) => fixture.with_profile(profile),
        None => fixture,
    }
}

fn rejected_number(
    name: &str,
    spelling: &str,
    type_name: &str,
    source: SourceExpectation,
    mon_code: MonErrorCode,
    profile: Option<NumericProfile>,
) -> ParityFixture {
    let literal = format!("value = Box(number = {spelling})");
    let fixture = fixture(
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
    .with_source_nominal_type("value", "Box");
    match profile {
        Some(profile) => fixture.with_profile(profile),
        None => fixture,
    }
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
            None,
        ));
        fixtures.push(accepted_number(
            &format!("{type_name}_maximum"),
            maximum_text,
            type_name,
            maximum,
            None,
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
                None,
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
            None,
        ));
        fixtures.push(accepted_number(
            &format!("{type_name}_maximum"),
            maximum_text,
            type_name,
            maximum,
            None,
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
            None,
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
            None,
        ));
    }

    fixtures.push(accepted_number(
        "byte_zero",
        "0",
        "Byte",
        Value::Byte(0),
        None,
    ));
    fixtures.push(accepted_number(
        "byte_maximum",
        "255",
        "Byte",
        Value::Byte(255),
        None,
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
    ] {
        let type_name = if name.starts_with("u8_") {
            "U8"
        } else {
            "Byte"
        };
        let literal = format!("value = Box(number = {spelling})");
        fixtures.push(rejected_number(
            name,
            spelling,
            type_name,
            source_invalid_number(reason, in_literal(&literal, spelling, 1)),
            MonErrorCode::NumericRange,
            None,
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
            None,
        ));
    }

    // Values above binary64's exact-integer range remain exact in integer receivers.
    fixtures.push(accepted_number(
        "i64_above_binary64_exact_integer_range",
        "-9007199254740993",
        "I64",
        Value::I64(-9_007_199_254_740_993),
        None,
    ));
    fixtures.push(accepted_number(
        "u64_above_binary64_exact_integer_range",
        "9007199254740993",
        "U64",
        Value::U64(9_007_199_254_740_993),
        None,
    ));

    fixtures
}

pub(super) fn profiles() -> [(&'static str, NumericProfile); 4] {
    [
        (
            "int32_float32",
            NumericProfile {
                int_width: IntWidth::Bits32,
                float_precision: FloatPrecision::Bits32,
            },
        ),
        (
            "int32_float64",
            NumericProfile {
                int_width: IntWidth::Bits32,
                float_precision: FloatPrecision::Bits64,
            },
        ),
        (
            "int64_float32",
            NumericProfile {
                int_width: IntWidth::Bits64,
                float_precision: FloatPrecision::Bits32,
            },
        ),
        (
            "int64_float64",
            NumericProfile {
                int_width: IntWidth::Bits64,
                float_precision: FloatPrecision::Bits64,
            },
        ),
    ]
}

fn profiled_int_fixtures() -> Vec<ParityFixture> {
    let mut fixtures = Vec::new();
    for (profile_name, profile) in profiles() {
        let (minimum, minimum_text, maximum, maximum_text, below, above) = match profile.int_width {
            IntWidth::Bits32 => (
                i64::from(i32::MIN),
                "-2147483648",
                i64::from(i32::MAX),
                "2147483647",
                "-2147483649",
                "2147483648",
            ),
            IntWidth::Bits64 => (
                i64::MIN,
                "-9223372036854775808",
                i64::MAX,
                "9223372036854775807",
                "-9223372036854775809",
                "9223372036854775808",
            ),
        };
        fixtures.push(accepted_number(
            &format!("{profile_name}_int_minimum"),
            minimum_text,
            "Int",
            Value::Int(minimum),
            Some(profile),
        ));
        fixtures.push(accepted_number(
            &format!("{profile_name}_int_maximum"),
            maximum_text,
            "Int",
            Value::Int(maximum),
            Some(profile),
        ));
        for (suffix, spelling) in [("below_minimum", below), ("above_maximum", above)] {
            let literal = format!("value = Box(number = {spelling})");
            fixtures.push(rejected_number(
                &format!("{profile_name}_int_{suffix}"),
                spelling,
                "Int",
                source_invalid_number(
                    NumberLiteralErrorReason::OutsideIntRange,
                    in_literal(&literal, spelling, 1),
                ),
                MonErrorCode::NumericRange,
                Some(profile),
            ));
        }

        let beyond_f64 = "9007199254740993";
        if profile.int_width == IntWidth::Bits64 {
            fixtures.push(accepted_number(
                &format!("{profile_name}_int_above_binary64_exact_range"),
                beyond_f64,
                "Int",
                Value::Int(9_007_199_254_740_993),
                Some(profile),
            ));
        } else {
            let literal = format!("value = Box(number = {beyond_f64})");
            fixtures.push(rejected_number(
                &format!("{profile_name}_int_above_binary64_exact_range"),
                beyond_f64,
                "Int",
                source_invalid_number(
                    NumberLiteralErrorReason::OutsideIntRange,
                    in_literal(&literal, beyond_f64, 1),
                ),
                MonErrorCode::NumericRange,
                Some(profile),
            ));
        }
    }
    fixtures
}

fn fixed_profiles_fixture(name: &str, profile: NumericProfile) -> ParityFixture {
    let literal = "value = FixedSet(signed = -1, wide = 9007199254740993, octet = 255)";
    let fields = vec![
        Field::required("signed", SchemaType::I32),
        Field::required("wide", SchemaType::U64),
        Field::required("octet", SchemaType::Byte),
    ];
    fixture(
        name,
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
    .with_profile(profile)
    .with_source_nominal_type("value", "FixedSet")
}

#[test]
fn fixed_integer_and_byte_boundaries_reject_out_of_range_values() {
    let _guard = lock_counter_test();
    for fixture in fixed_integer_boundary_fixtures() {
        assert_fixture(fixture);
    }
}

#[test]
fn profile_int_destinations_keep_selected_width_and_exact_identity() {
    let _guard = lock_counter_test();
    for fixture in profiled_int_fixtures() {
        assert_fixture(fixture);
    }
}

#[test]
fn fixed_width_and_byte_values_are_profile_independent() {
    let _guard = lock_counter_test();
    for (name, profile) in profiles() {
        assert_fixture(fixed_profiles_fixture(name, profile));
    }

    let mut identity_fixtures = Vec::new();
    for (name, profile) in profiles() {
        let (fixed_name, fixed_schema, fixed_value) = match profile.int_width {
            IntWidth::Bits32 => ("I32", SchemaType::I32, Value::I32(42)),
            IntWidth::Bits64 => ("I64", SchemaType::I64, Value::I64(42)),
        };
        let literal = "value = IntPair(profile = 42, fixed = 42)";
        identity_fixtures.push(
            fixture(
                format!("{name}_int_is_distinct_from_{fixed_name}"),
                literal,
                format!("IntPair = | profile Int, fixed {fixed_name} |\n"),
                Schema::record(vec![Field::required(
                    "value",
                    SchemaType::Struct {
                        name: "IntPair".into(),
                        fields: vec![
                            Field::required("profile", SchemaType::Int),
                            Field::required("fixed", fixed_schema),
                        ],
                    },
                )]),
                SourceExpectation::Accept,
                MonExpectation::Accept(Value::Record(vec![(
                    "value".into(),
                    Value::Record(vec![
                        ("profile".into(), Value::Int(42)),
                        ("fixed".into(), fixed_value),
                    ]),
                )])),
            )
            .with_profile(profile)
            .with_source_nominal_type("value", "IntPair"),
        );
    }
    for fixture in identity_fixtures {
        assert_fixture(fixture);
    }
}
