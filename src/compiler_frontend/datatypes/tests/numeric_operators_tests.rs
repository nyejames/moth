//! Numeric operator promotion policy tests.

use crate::compiler_frontend::datatypes::numeric_operators::{
    NumericOperator, binary_operation_domain, common_fixed_integer, comparison_supported,
    negation_domain,
};
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use moth_lexical::numeric::decimal::NumberScale;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarClass};
use moth_lexical::numeric::profile::NumericProfile;

use FixedScalar::*;

const BINARY_OPERATORS: [NumericOperator; 7] = [
    NumericOperator::Add,
    NumericOperator::Subtract,
    NumericOperator::Multiply,
    NumericOperator::Divide,
    NumericOperator::IntegerDivide,
    NumericOperator::Remainder,
    NumericOperator::Power,
];

const FIXED_INTEGERS: [FixedScalar; 8] = [I8, I16, I32, I64, U8, U16, U32, U64];
const FIXED_FLOATS: [FixedScalar; 3] = [F16, F32, F64];

fn fixed(scalar: FixedScalar) -> NumericScalar {
    NumericScalar::Fixed(scalar)
}

fn number(scale: u16) -> NumericScalar {
    NumericScalar::Number(NumberScale::new(scale).expect("test Dec scale is valid"))
}

fn range(scalar: FixedScalar) -> (i128, i128) {
    fixed(scalar)
        .integer_range(NumericProfile::STANDARD)
        .expect("fixed integers have a range")
}

/// Independent oracle: the narrowest candidate of at least 32 bits, signed whenever either input
/// is signed, whose range covers both input ranges.
fn expected_common_integer(left: FixedScalar, right: FixedScalar) -> Option<FixedScalar> {
    let signed = [left, right]
        .iter()
        .any(|scalar| scalar.class() == FixedScalarClass::SignedInteger);
    let candidates: &[FixedScalar] = if signed { &[I32, I64] } else { &[U32, U64] };
    let (left_min, left_max) = range(left);
    let (right_min, right_max) = range(right);

    candidates.iter().copied().find(|candidate| {
        let (min, max) = range(*candidate);
        min <= left_min.min(right_min) && max >= left_max.max(right_max)
    })
}

#[test]
fn common_fixed_integer_matches_the_complete_range_rule_for_every_pair() {
    for left in FIXED_INTEGERS {
        for right in FIXED_INTEGERS {
            assert_eq!(
                common_fixed_integer(left, right),
                expected_common_integer(left, right),
                "{left:?} with {right:?}"
            );
        }
    }
}

#[test]
fn common_fixed_integer_matches_the_published_examples() {
    let cases = [
        (U8, U8, Some(U32)),
        (U8, U16, Some(U32)),
        (U32, U64, Some(U64)),
        (I32, I64, Some(I64)),
        (I8, I16, Some(I32)),
        (I16, U16, Some(I32)),
        (I32, U32, Some(I64)),
        (I64, U32, Some(I64)),
        (I64, U64, None),
        (I32, U64, None),
    ];

    for (left, right, expected) in cases {
        assert_eq!(
            common_fixed_integer(left, right),
            expected,
            "{left:?} + {right:?}"
        );
        assert_eq!(
            common_fixed_integer(right, left),
            expected,
            "{right:?} + {left:?}"
        );
    }
}

#[test]
fn fixed_integer_operators_compute_in_the_common_type_and_divide_in_f64() {
    for left in FIXED_INTEGERS {
        for right in FIXED_INTEGERS {
            let common = expected_common_integer(left, right);

            for operator in BINARY_OPERATORS {
                let expected = match (operator, common) {
                    (_, None) => None,
                    (NumericOperator::Divide, Some(_)) => Some(fixed(F64)),
                    (_, Some(common)) => Some(fixed(common)),
                };

                assert_eq!(
                    binary_operation_domain(operator, fixed(left), fixed(right)),
                    expected,
                    "{left:?} {operator:?} {right:?}"
                );
            }
        }
    }
}

#[test]
fn fixed_float_operators_use_the_wider_precision_with_an_f32_minimum() {
    let cases = [
        (F16, F16, F32),
        (F16, F32, F32),
        (F32, F32, F32),
        (F16, F64, F64),
        (F32, F64, F64),
        (F64, F64, F64),
    ];

    for (left, right, domain) in cases {
        for (left, right) in [(left, right), (right, left)] {
            for operator in BINARY_OPERATORS {
                let expected =
                    (operator != NumericOperator::IntegerDivide).then_some(fixed(domain));
                assert_eq!(
                    binary_operation_domain(operator, fixed(left), fixed(right)),
                    expected,
                    "{left:?} {operator:?} {right:?}"
                );
            }
        }
    }
}

#[test]
fn convenience_int_and_float_rules_are_unchanged() {
    use NumericScalar::{Float, Int};

    for operator in BINARY_OPERATORS {
        let int_result = match operator {
            NumericOperator::Divide => Float,
            _ => Int,
        };
        assert_eq!(
            binary_operation_domain(operator, Int, Int),
            Some(int_result)
        );

        let float_result = (operator != NumericOperator::IntegerDivide).then_some(Float);
        for (left, right) in [(Float, Float), (Int, Float), (Float, Int)] {
            assert_eq!(
                binary_operation_domain(operator, left, right),
                float_result,
                "{left:?} {operator:?} {right:?}"
            );
        }
    }
}

#[test]
fn number_arithmetic_keeps_its_scale_and_rejects_invalid_scale_operators() {
    let scale_zero = number(0);
    let scale_three = number(3);
    let same_scale_operations = [
        NumericOperator::Add,
        NumericOperator::Subtract,
        NumericOperator::Multiply,
        NumericOperator::Remainder,
    ];

    for operator in same_scale_operations {
        assert_eq!(
            binary_operation_domain(operator, scale_three, scale_three),
            Some(scale_three)
        );
        for integer in [NumericScalar::Int]
            .into_iter()
            .chain(FIXED_INTEGERS.into_iter().map(fixed))
        {
            assert_eq!(
                binary_operation_domain(operator, scale_three, integer),
                Some(scale_three)
            );
            assert_eq!(
                binary_operation_domain(operator, integer, scale_three),
                Some(scale_three)
            );
        }
    }

    assert_eq!(
        binary_operation_domain(NumericOperator::Divide, scale_three, scale_three),
        Some(scale_three)
    );
    assert_eq!(
        binary_operation_domain(NumericOperator::IntegerDivide, scale_zero, scale_zero),
        Some(scale_zero)
    );
    assert_eq!(
        binary_operation_domain(NumericOperator::Divide, scale_zero, scale_zero),
        None
    );
    assert_eq!(
        binary_operation_domain(NumericOperator::IntegerDivide, scale_three, scale_three),
        None
    );

    let other_scale = number(4);
    for operator in BINARY_OPERATORS {
        assert_eq!(
            binary_operation_domain(operator, scale_three, other_scale),
            None
        );
    }
}

#[test]
fn number_power_accepts_only_a_profile_int_exponent() {
    let number_scale = number(2);
    assert_eq!(
        binary_operation_domain(NumericOperator::Power, number_scale, NumericScalar::Int),
        Some(number_scale)
    );

    for exponent in FIXED_INTEGERS.into_iter().map(fixed) {
        assert_eq!(
            binary_operation_domain(NumericOperator::Power, number_scale, exponent),
            None
        );
    }
    assert_eq!(
        binary_operation_domain(NumericOperator::Power, number_scale, number_scale),
        None
    );
    assert_eq!(
        binary_operation_domain(NumericOperator::Power, NumericScalar::Int, number_scale),
        None
    );
}

#[test]
fn number_mixes_only_with_integer_domains_for_arithmetic_and_comparison() {
    let number_domain = number(2);
    for integer in [NumericScalar::Int]
        .into_iter()
        .chain(FIXED_INTEGERS.into_iter().map(fixed))
    {
        assert!(comparison_supported(number_domain, integer));
        assert!(comparison_supported(integer, number_domain));
    }

    assert!(comparison_supported(number_domain, number_domain));
    assert!(!comparison_supported(number_domain, number(3)));
    for other in [
        NumericScalar::Float,
        fixed(F16),
        fixed(F32),
        fixed(F64),
        fixed(Byte),
    ] {
        assert!(!comparison_supported(number_domain, other));
        assert!(!comparison_supported(other, number_domain));
        assert_eq!(
            binary_operation_domain(NumericOperator::Add, number_domain, other),
            None
        );
    }
}

#[test]
fn families_never_mix_implicitly() {
    let convenience = [NumericScalar::Int, NumericScalar::Float];

    for operator in BINARY_OPERATORS {
        for scalar in FIXED_INTEGERS.into_iter().chain(FIXED_FLOATS) {
            for other in convenience {
                assert_eq!(
                    binary_operation_domain(operator, fixed(scalar), other),
                    None
                );
                assert_eq!(
                    binary_operation_domain(operator, other, fixed(scalar)),
                    None
                );
                assert!(!comparison_supported(fixed(scalar), other));
                assert!(!comparison_supported(other, fixed(scalar)));
            }
        }

        for integer in FIXED_INTEGERS {
            for float in FIXED_FLOATS {
                assert_eq!(
                    binary_operation_domain(operator, fixed(integer), fixed(float)),
                    None
                );
                assert_eq!(
                    binary_operation_domain(operator, fixed(float), fixed(integer)),
                    None
                );
                assert!(!comparison_supported(fixed(integer), fixed(float)));
            }
        }
    }
}

#[test]
fn negate_is_never_a_binary_operation() {
    assert_eq!(
        binary_operation_domain(
            NumericOperator::Negate,
            NumericScalar::Int,
            NumericScalar::Int
        ),
        None
    );
}

#[test]
fn negation_rejects_every_unsigned_type_and_widens_narrow_operands() {
    let cases = [
        (NumericScalar::Int, Some(NumericScalar::Int)),
        (number(7), Some(number(7))),
        (NumericScalar::Float, Some(NumericScalar::Float)),
        (fixed(I8), Some(fixed(I32))),
        (fixed(I16), Some(fixed(I32))),
        (fixed(I32), Some(fixed(I32))),
        (fixed(I64), Some(fixed(I64))),
        (fixed(U8), None),
        (fixed(U16), None),
        (fixed(U32), None),
        (fixed(U64), None),
        (fixed(F16), Some(fixed(F32))),
        (fixed(F32), Some(fixed(F32))),
        (fixed(F64), Some(fixed(F64))),
    ];

    for (operand, expected) in cases {
        assert_eq!(negation_domain(operand), expected, "-{operand:?}");
    }
}

#[test]
fn every_fixed_integer_pair_compares_including_i64_with_u64() {
    for left in FIXED_INTEGERS {
        for right in FIXED_INTEGERS {
            assert!(
                comparison_supported(fixed(left), fixed(right)),
                "{left:?} {right:?}"
            );
        }
    }

    for left in FIXED_FLOATS {
        for right in FIXED_FLOATS {
            assert!(
                comparison_supported(fixed(left), fixed(right)),
                "{left:?} {right:?}"
            );
        }
    }

    assert!(comparison_supported(
        NumericScalar::Int,
        NumericScalar::Float
    ));
}
