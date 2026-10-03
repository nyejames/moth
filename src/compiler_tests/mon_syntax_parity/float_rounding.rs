//! Fixed and profile-selected binary-float rounding parity.
//!
//! WHAT: checks typed source/MON materialisation, finite limits, midpoint decisions and signed zero.
//! WHY: each receiver rounds once to its own IEEE precision and retains its declared identity.

use crate::compiler_frontend::instrumentation::lock_counter_test;
use crate::mon::{Field, MonErrorCode, PathSegment, Schema, SchemaType, Value};
use moth_lexical::numeric::fixed_scalar::FixedScalar;
use moth_lexical::numeric::parse::NumberLiteralErrorReason;
use moth_lexical::numeric::profile::{FloatPrecision, IntWidth, NumericProfile};

use super::support::{
    MonExpectation, ParityFixture, SourceExpectation, assert_fixture, fixture, in_literal,
    mon_numeric_error, source_invalid_number,
};

fn box_schema(number_type: SchemaType) -> Schema {
    Schema::record(vec![Field::required(
        "value",
        SchemaType::Struct {
            name: "Box".into(),
            fields: vec![Field::required("number", number_type)],
        },
    )])
}

fn box_value(number: Value) -> Value {
    Value::Record(vec![(
        "value".into(),
        Value::Record(vec![("number".into(), number)]),
    )])
}

fn float_type(type_name: &str) -> SchemaType {
    match type_name {
        "Float" => SchemaType::Float,
        "F16" => SchemaType::F16,
        "F32" => SchemaType::F32,
        "F64" => SchemaType::F64,
        _ => panic!("unsupported float parity type {type_name}"),
    }
}

fn accepted_float(
    name: &str,
    spelling: &str,
    type_name: &str,
    value: Value,
    profile: NumericProfile,
) -> ParityFixture {
    fixture(
        name,
        format!("value = Box(number = {spelling})"),
        format!("Box = | number {type_name} |\n"),
        box_schema(float_type(type_name)),
        SourceExpectation::Accept,
        MonExpectation::Accept(box_value(value)),
    )
    .with_profile(profile)
    .with_source_nominal_type("value", "Box")
}

fn rejected_float(
    name: &str,
    spelling: &str,
    type_name: &str,
    scalar: FixedScalar,
    profile: NumericProfile,
) -> ParityFixture {
    let literal = format!("value = Box(number = {spelling})");
    fixture(
        name,
        literal.clone(),
        format!("Box = | number {type_name} |\n"),
        box_schema(float_type(type_name)),
        source_invalid_number(
            NumberLiteralErrorReason::NonFiniteFixedFloat(scalar),
            in_literal(&literal, spelling, 1),
        ),
        mon_numeric_error(
            &literal,
            spelling,
            1,
            MonErrorCode::NonFiniteFloat,
            vec![
                PathSegment::Field("value".into()),
                PathSegment::Field("number".into()),
            ],
        ),
    )
    .with_profile(profile)
    .with_source_nominal_type("value", "Box")
}

fn profiles() -> [(&'static str, NumericProfile); 4] {
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

fn fixed_float_fixtures(profile: NumericProfile) -> Vec<ParityFixture> {
    let mut fixtures = vec![
        accepted_float(
            "f16_midpoint_below_neighbour",
            "1.0004882812499998",
            "F16",
            Value::F16(f64::from_bits(0x3ff0_0000_0000_0000)), // 1.0, below the midpoint.
            profile,
        ),
        accepted_float(
            "f16_exact_midpoint_ties_to_even_lower",
            "1.00048828125",
            "F16",
            Value::F16(f64::from_bits(0x3ff0_0000_0000_0000)), // 1.0 is the even neighbour.
            profile,
        ),
        accepted_float(
            "f16_required_above_midpoint_decimal",
            "1.0004882812500000000001",
            "F16",
            Value::F16(f64::from_bits(0x3ff0_0400_0000_0000)), // 1 + 2^-10, upper neighbour.
            profile,
        ),
        accepted_float(
            "f16_odd_lower_midpoint_ties_to_even_upper",
            "1.00146484375",
            "F16",
            Value::F16(f64::from_bits(0x3ff0_0800_0000_0000)), // 1 + 2^-9, even upper neighbour.
            profile,
        ),
        accepted_float(
            "f16_finite_maximum",
            "65504",
            "F16",
            Value::F16(f64::from_bits(0x40ef_fc00_0000_0000)), // Largest finite binary16 value.
            profile,
        ),
        accepted_float(
            "f16_below_overflow_midpoint_rounds_to_maximum",
            "65519",
            "F16",
            Value::F16(f64::from_bits(0x40ef_fc00_0000_0000)), // Still below the 65520 threshold.
            profile,
        ),
        accepted_float(
            "f16_smallest_subnormal",
            "0.000000059604644775390625",
            "F16",
            Value::F16(f64::from_bits(0x3e70_0000_0000_0000)), // Binary16 bit pattern 0x0001.
            profile,
        ),
        accepted_float(
            "f16_subnormal_tie_rounds_to_positive_zero",
            "0.0000000298023223876953125",
            "F16",
            Value::F16(f64::from_bits(0x0000_0000_0000_0000)), // Half of the smallest subnormal.
            profile,
        ),
        accepted_float(
            "f16_negative_subnormal_tie_preserves_negative_zero",
            "-0.0000000298023223876953125",
            "F16",
            Value::F16(f64::from_bits(0x8000_0000_0000_0000)), // Signed zero at the negative tie.
            profile,
        ),
        accepted_float(
            "f16_next_even_subnormal",
            "0.0000000894069671630859375",
            "F16",
            Value::F16(f64::from_bits(0x3e80_0000_0000_0000)), // Binary16 bit pattern 0x0002.
            profile,
        ),
        accepted_float(
            "f16_positive_zero",
            "0",
            "F16",
            Value::F16(f64::from_bits(0x0000_0000_0000_0000)),
            profile,
        ),
        accepted_float(
            "f16_negative_zero",
            "-0.0",
            "F16",
            Value::F16(f64::from_bits(0x8000_0000_0000_0000)),
            profile,
        ),
        accepted_float(
            "f32_midpoint_below_neighbour",
            "1.0000000596046446",
            "F32",
            Value::F32(f64::from(f32::from_bits(0x3f80_0000))), // 1.0, below the midpoint.
            profile,
        ),
        accepted_float(
            "f32_exact_midpoint_ties_to_even_lower",
            "1.000000059604644775390625",
            "F32",
            Value::F32(f64::from(f32::from_bits(0x3f80_0000))), // 1.0 is the even neighbour.
            profile,
        ),
        accepted_float(
            "f32_midpoint_above_neighbour",
            "1.0000000596046449",
            "F32",
            Value::F32(f64::from(f32::from_bits(0x3f80_0001))), // Next binary32 after 1.0.
            profile,
        ),
        accepted_float(
            "f32_odd_lower_midpoint_ties_to_even_upper",
            "1.000000178813934326171875",
            "F32",
            Value::F32(f64::from(f32::from_bits(0x3f80_0002))), // Even upper neighbour.
            profile,
        ),
        accepted_float(
            "f32_finite_maximum",
            "3.4028235e38",
            "F32",
            Value::F32(f64::from(f32::from_bits(0x7f7f_ffff))), // Largest finite binary32 value.
            profile,
        ),
        accepted_float(
            "f32_smallest_subnormal",
            "1e-45",
            "F32",
            Value::F32(f64::from(f32::from_bits(0x0000_0001))), // Smallest positive binary32 subnormal.
            profile,
        ),
        accepted_float(
            "f32_smallest_normal",
            "1.1754943508222875e-38",
            "F32",
            Value::F32(f64::from(f32::from_bits(0x0080_0000))), // Smallest normal binary32 value.
            profile,
        ),
        accepted_float(
            "f32_negative_zero",
            "-0",
            "F32",
            Value::F32(f64::from_bits(0x8000_0000_0000_0000)),
            profile,
        ),
        accepted_float(
            "f64_finite_maximum",
            "1.7976931348623157e308",
            "F64",
            Value::F64(f64::from_bits(0x7fef_ffff_ffff_ffff)), // Largest finite binary64 value.
            profile,
        ),
        accepted_float(
            "f64_smallest_subnormal",
            "5e-324",
            "F64",
            Value::F64(f64::from_bits(0x0000_0000_0000_0001)), // Smallest positive binary64 subnormal.
            profile,
        ),
        accepted_float(
            "f64_negative_zero",
            "-0.0",
            "F64",
            Value::F64(f64::from_bits(0x8000_0000_0000_0000)),
            profile,
        ),
    ];
    fixtures.push(rejected_float(
        "f16_overflow_rounding_threshold",
        "65520",
        "F16",
        FixedScalar::F16,
        profile,
    ));
    fixtures.push(rejected_float(
        "f32_overflow_rounding_threshold",
        "340282356779733661637539395458142568448",
        "F32",
        FixedScalar::F32,
        profile,
    ));
    fixtures.push(rejected_float(
        "f64_above_finite_maximum",
        "1.7976931348623159e308",
        "F64",
        FixedScalar::F64,
        profile,
    ));
    fixtures
}

fn profile_float_fixtures(profile_name: &str, profile: NumericProfile) -> Vec<ParityFixture> {
    let (float_value, float_precision_name) = match profile.float_precision {
        FloatPrecision::Bits32 => (
            f64::from(f32::from_bits(0x3dcc_cccd)), // Correctly rounded binary32 0.1.
            "Float32",
        ),
        FloatPrecision::Bits64 => (
            f64::from_bits(0x3fb9_9999_9999_999a), // Correctly rounded binary64 0.1.
            "Float64",
        ),
    };
    let mut fixtures = vec![
        accepted_float(
            &format!("{profile_name}_float_rounded_0_1"),
            "0.1",
            "Float",
            Value::Float(float_value),
            profile,
        ),
        accepted_float(
            &format!("{profile_name}_float_negative_zero"),
            "-0.0",
            "Float",
            Value::Float(f64::from_bits(0x8000_0000_0000_0000)),
            profile,
        ),
        accepted_float(
            &format!("{profile_name}_float_whole_integer_above_2pow53"),
            "9007199254740993",
            "Float",
            Value::Float(f64::from_bits(0x4340_0000_0000_0000)), // Rounds to 2^53 at either precision.
            profile,
        ),
    ];

    let fixed_f32 = f64::from(f32::from_bits(0x3dcc_cccd));
    let pair_schema = Schema::record(vec![Field::required(
        "value",
        SchemaType::Struct {
            name: "FloatPair".into(),
            fields: vec![
                Field::required("profile", SchemaType::Float),
                Field::required("fixed", SchemaType::F32),
            ],
        },
    )]);
    fixtures.push(
        fixture(
            format!("{profile_name}_{float_precision_name}_identity_differs_from_f32"),
            "value = FloatPair(profile = 0.1, fixed = 0.1)",
            "FloatPair = | profile Float, fixed F32 |\n",
            pair_schema,
            SourceExpectation::Accept,
            MonExpectation::Accept(Value::Record(vec![(
                "value".into(),
                Value::Record(vec![
                    ("profile".into(), Value::Float(float_value)),
                    ("fixed".into(), Value::F32(fixed_f32)),
                ]),
            )])),
        )
        .with_profile(profile)
        .with_source_nominal_type("value", "FloatPair"),
    );
    fixtures
}

#[test]
fn fixed_float_rounding_boundaries_are_profile_independent() {
    let _guard = lock_counter_test();
    for (_, profile) in profiles() {
        for fixture in fixed_float_fixtures(profile) {
            assert_fixture(fixture);
        }
    }
}

#[test]
fn profile_float_receivers_round_and_keep_float_identity() {
    let _guard = lock_counter_test();
    for (name, profile) in profiles() {
        for fixture in profile_float_fixtures(name, profile) {
            assert_fixture(fixture);
        }
    }
}
