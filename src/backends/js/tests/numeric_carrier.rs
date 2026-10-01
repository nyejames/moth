use crate::backends::js::numeric_carrier::{JsNumericCarrier, JsNumericConversion};
use crate::compiler_frontend::builtins::casts::evidence::numeric_scalars;
use crate::compiler_frontend::datatypes::fixed_scalar::{FixedScalar, FixedScalarValue};
use crate::compiler_frontend::datatypes::numeric_profile::{
    FloatPrecision, IntWidth, NumericProfile,
};
use crate::compiler_frontend::datatypes::numeric_scalar::{BinaryFloatPrecision, NumericScalar};

#[test]
fn carrier_selection_follows_profile_and_complete_scalar_range() {
    let profile = NumericProfile {
        int_width: IntWidth::Bits64,
        float_precision: FloatPrecision::Bits32,
    };

    assert_eq!(
        JsNumericCarrier::for_scalar(NumericScalar::Int, profile),
        Some(JsNumericCarrier::BigInteger {
            min: i64::MIN as i128,
            max: i64::MAX as i128,
        })
    );
    assert_eq!(
        JsNumericCarrier::for_scalar(NumericScalar::Float, profile),
        Some(JsNumericCarrier::BinaryFloat {
            precision: BinaryFloatPrecision::Binary32,
        })
    );
    assert!(matches!(
        JsNumericCarrier::for_scalar(
            NumericScalar::Fixed(FixedScalar::U32),
            NumericProfile::STANDARD
        ),
        Some(JsNumericCarrier::ExactInteger {
            min: 0,
            max: 4_294_967_295
        })
    ));
    assert!(matches!(
        JsNumericCarrier::for_scalar(
            NumericScalar::Fixed(FixedScalar::U64),
            NumericProfile::STANDARD
        ),
        Some(JsNumericCarrier::BigInteger {
            min: 0,
            max: 18_446_744_073_709_551_615
        })
    ));
}

#[test]
fn fixed_literals_use_their_javascript_carrier_without_losing_bits() {
    for (value, expected) in [
        (
            FixedScalarValue::signed(FixedScalar::I8, i8::MIN.into()).unwrap(),
            "-128",
        ),
        (
            FixedScalarValue::unsigned(FixedScalar::U32, u32::MAX.into()).unwrap(),
            "4294967295",
        ),
        (
            FixedScalarValue::signed(FixedScalar::I64, i64::MIN).unwrap(),
            "-9223372036854775808n",
        ),
        (
            FixedScalarValue::unsigned(FixedScalar::U64, u64::MAX).unwrap(),
            "18446744073709551615n",
        ),
        (
            FixedScalarValue::binary_float(FixedScalar::F16, -0.0).unwrap(),
            "-0",
        ),
        (
            FixedScalarValue::binary_float(FixedScalar::F32, 1.5).unwrap(),
            "1.5",
        ),
        (
            FixedScalarValue::binary_float(FixedScalar::F64, 0.1).unwrap(),
            "0.1",
        ),
        (
            FixedScalarValue::unsigned(FixedScalar::Byte, u8::MAX.into()).unwrap(),
            "255",
        ),
    ] {
        assert_eq!(
            JsNumericCarrier::fixed_literal(value, NumericProfile::STANDARD).as_deref(),
            Some(expected)
        );
    }
}

#[test]
fn numeric_conversion_calls_only_required_helpers_in_every_profile() {
    let profiles = [
        NumericProfile {
            int_width: IntWidth::Bits32,
            float_precision: FloatPrecision::Bits32,
        },
        NumericProfile {
            int_width: IntWidth::Bits32,
            float_precision: FloatPrecision::Bits64,
        },
        NumericProfile {
            int_width: IntWidth::Bits64,
            float_precision: FloatPrecision::Bits32,
        },
        NumericProfile {
            int_width: IntWidth::Bits64,
            float_precision: FloatPrecision::Bits64,
        },
    ];
    let scalars = numeric_scalars().collect::<Vec<_>>();

    for profile in profiles {
        for source in &scalars {
            for target in &scalars {
                let conversion = JsNumericConversion::classify(*source, *target, profile)
                    .expect("every canonical numeric pair has a JS conversion");
                let expression = conversion.expression("value", *source, *target);
                let required = conversion
                    .required_helpers()
                    .iter()
                    .map(|helper| helper.function_name())
                    .collect::<Vec<_>>();

                for called in expression
                    .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
                    .filter(|word| word.starts_with("__moth_"))
                {
                    assert!(
                        required.contains(&called),
                        "{source:?} -> {target:?} under {profile} calls {called}, which is not a required helper"
                    );
                }
            }
        }
    }
}
