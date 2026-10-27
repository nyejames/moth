//! Exact-width numeric operator domain, failure-discharge and failure-code policy tests.
//!
//! The scalar rows below are the regression oracle: equal fixed types keep their own type,
//! mixed fixed types use the smallest supported domain covering both complete operand ranges,
//! and failure discharge is judged against the selected result domain. `Dec`, `Byte` and
//! mixed-family rejection keep their explicit negative rows.

use crate::compiler_frontend::datatypes::numeric_operators::{
    NumericOperator, binary_operation_domain, common_fixed_integer, comparison_supported,
    negation_domain, numeric_failure_codes, numeric_operation_cannot_fail, range_iteration_domain,
};
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use moth_lexical::numeric::decimal::NumberScale;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarClass};

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

/// Complete inclusive value range of one fixed integer scalar, read from the scalar's own
/// canonical range facts rather than any promotion or carrier.
fn range(scalar: FixedScalar) -> (i128, i128) {
    match scalar.class() {
        FixedScalarClass::SignedInteger => {
            let (min, max) = scalar.signed_range().expect("signed scalars have a range");
            (i128::from(min), i128::from(max))
        }
        FixedScalarClass::UnsignedInteger => (
            0,
            i128::from(
                scalar
                    .unsigned_max()
                    .expect("unsigned scalars have a maximum"),
            ),
        ),
        FixedScalarClass::BinaryFloat | FixedScalarClass::Octet => {
            panic!("the common-integer oracle takes fixed integer scalars")
        }
    }
}

/// Independent oracle: the narrowest supported type of the required signedness whose complete
/// range covers both complete operand ranges. Equal operands keep their own type, same-signedness
/// operands take the wider width, and mixed signedness always selects a signed type (none exists
/// for any signed operand paired with `U64`).
fn expected_common_integer(left: FixedScalar, right: FixedScalar) -> Option<FixedScalar> {
    let signed = [left, right]
        .iter()
        .any(|scalar| scalar.class() == FixedScalarClass::SignedInteger);
    let candidates: &[FixedScalar] = if signed {
        &[I8, I16, I32, I64]
    } else {
        &[U8, U16, U32, U64]
    };
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
fn common_fixed_integer_matches_the_published_d15_examples() {
    let cases = [
        (I8, I8, Some(I8)),
        (I16, I16, Some(I16)),
        (I8, I16, Some(I16)),
        (U8, U8, Some(U8)),
        (U16, U16, Some(U16)),
        (U8, U16, Some(U16)),
        (U32, U64, Some(U64)),
        (I32, I64, Some(I64)),
        (I8, U8, Some(I16)),
        (I16, U8, Some(I16)),
        (I8, U16, Some(I32)),
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
fn fixed_float_operators_preserve_the_wider_precision_without_an_f32_floor() {
    // Equal precisions keep their own type and `F16` has no minimum `F32` semantic domain.
    let cases = [
        (F16, F16, F16),
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
        for integer in FIXED_INTEGERS.into_iter().map(fixed) {
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
fn number_power_accepts_only_an_exact_i32_exponent() {
    let number_scale = number(2);

    // The exponent is the exact I32 identity.
    assert_eq!(
        binary_operation_domain(NumericOperator::Power, number_scale, fixed(I32)),
        Some(number_scale)
    );

    for exponent in FIXED_INTEGERS
        .into_iter()
        .filter(|scalar| *scalar != I32)
        .map(fixed)
    {
        assert_eq!(
            binary_operation_domain(NumericOperator::Power, number_scale, exponent),
            None,
            "{exponent:?} is not the I32 exponent"
        );
    }
    assert_eq!(
        binary_operation_domain(NumericOperator::Power, number_scale, number_scale),
        None
    );
    // `Dec` may be a base but is never an exponent: a fixed integer base with a `Dec`
    // exponent rejects, exactly like every non-`I32` exponent above.
    assert_eq!(
        binary_operation_domain(NumericOperator::Power, fixed(I32), number_scale),
        None
    );
}

#[test]
fn number_mixes_only_with_integer_domains_for_arithmetic_and_comparison() {
    let number_domain = number(2);
    for integer in FIXED_INTEGERS.into_iter().map(fixed) {
        assert!(comparison_supported(number_domain, integer));
        assert!(comparison_supported(integer, number_domain));
    }

    assert!(comparison_supported(number_domain, number_domain));
    assert!(!comparison_supported(number_domain, number(3)));
    for other in [fixed(F16), fixed(F32), fixed(F64), fixed(Byte)] {
        assert!(!comparison_supported(number_domain, other));
        assert!(!comparison_supported(other, number_domain));
        assert_eq!(
            binary_operation_domain(NumericOperator::Add, number_domain, other),
            None
        );
    }
}

#[test]
fn fixed_integer_and_binary_float_families_never_mix_implicitly() {
    for operator in BINARY_OPERATORS {
        for integer in FIXED_INTEGERS {
            for float in FIXED_FLOATS {
                assert_eq!(
                    binary_operation_domain(operator, fixed(integer), fixed(float)),
                    None,
                    "{integer:?} {operator:?} {float:?}"
                );
                assert_eq!(
                    binary_operation_domain(operator, fixed(float), fixed(integer)),
                    None,
                    "{float:?} {operator:?} {integer:?}"
                );
                assert!(!comparison_supported(fixed(integer), fixed(float)));
                assert!(!comparison_supported(fixed(float), fixed(integer)));
            }
        }
    }
}

#[test]
fn negate_is_never_a_binary_operation() {
    assert_eq!(
        binary_operation_domain(NumericOperator::Negate, fixed(I8), fixed(I8)),
        None
    );
}

#[test]
fn negation_preserves_signed_and_float_width_and_rejects_unsigned_and_byte() {
    // A fixed signed integer or binary float keeps its own type under unary minus, so
    // negating a signed minimum fails in that narrow type. Unsigned types and `Byte` stay
    // invalid.
    let cases = [
        (fixed(I8), Some(fixed(I8))),
        (fixed(I16), Some(fixed(I16))),
        (fixed(I32), Some(fixed(I32))),
        (fixed(I64), Some(fixed(I64))),
        (fixed(U8), None),
        (fixed(U16), None),
        (fixed(U32), None),
        (fixed(U64), None),
        (fixed(Byte), None),
        (fixed(F16), Some(fixed(F16))),
        (fixed(F32), Some(fixed(F32))),
        (fixed(F64), Some(fixed(F64))),
        // `Dec` keeps its own scale; fixed unsigned and `Byte` stay rejected above.
        (number(7), Some(number(7))),
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
}

#[test]
fn discharge_proves_only_range_contained_fixed_integer_add_sub_mul_negate() {
    use NumericOperator as Op;
    // (operator, left, right, domain, expected): domains are the result domains the policy must
    // select, so the table pins the predicate's contract inputs.
    let cases = [
        // True table: the selected result domain contains the complete mathematical result
        // range of the operation.
        (Op::Add, fixed(U8), Some(fixed(I8)), fixed(I16), true),
        (Op::Subtract, fixed(U8), Some(fixed(I8)), fixed(I16), true),
        (Op::Multiply, fixed(U8), Some(fixed(I8)), fixed(I16), true),
        (Op::Add, fixed(I8), Some(fixed(U8)), fixed(I16), true),
        // False table: equal narrow operands keep their own result domain, so the complete
        // result range exceeds it. `U8` plus `U8` can overflow `U8` and is not proven safe.
        (Op::Add, fixed(U8), Some(fixed(U8)), fixed(U8), false),
        (Op::Multiply, fixed(U8), Some(fixed(U8)), fixed(U8), false),
        (
            Op::Multiply,
            fixed(U16),
            Some(fixed(U16)),
            fixed(U16),
            false,
        ),
        (Op::Add, fixed(I8), Some(fixed(I8)), fixed(I8), false),
        (Op::Subtract, fixed(I8), Some(fixed(I8)), fixed(I8), false),
        // Negation keeps the operand type, so negating a signed minimum always overflows it.
        (Op::Negate, fixed(I8), None, fixed(I8), false),
        (Op::Negate, fixed(I16), None, fixed(I16), false),
        (Op::Subtract, fixed(U8), Some(fixed(U8)), fixed(U8), false),
        (Op::Add, fixed(U32), Some(fixed(U32)), fixed(U32), false),
        (Op::Add, fixed(U64), Some(fixed(U64)), fixed(U64), false),
        (
            Op::Multiply,
            fixed(I64),
            Some(fixed(I64)),
            fixed(I64),
            false,
        ),
        (Op::Negate, fixed(I32), None, fixed(I32), false),
        (Op::Negate, fixed(I64), None, fixed(I64), false),
        (Op::Divide, fixed(U8), Some(fixed(U8)), fixed(F64), false),
        (
            Op::IntegerDivide,
            fixed(I32),
            Some(fixed(I32)),
            fixed(I32),
            false,
        ),
        (
            Op::Remainder,
            fixed(U32),
            Some(fixed(U32)),
            fixed(U32),
            false,
        ),
        (Op::Power, fixed(U8), Some(fixed(U8)), fixed(U8), false),
        (Op::Add, fixed(F32), Some(fixed(F32)), fixed(F32), false),
        (Op::Add, number(2), Some(number(2)), number(2), false),
    ];

    for (operator, left, right, domain, expected) in cases {
        assert_eq!(
            numeric_operation_cannot_fail(operator, left, right, domain),
            expected,
            "{operator:?} {left:?} {right:?} -> {domain:?}"
        );
    }

    // The rows above pin the domains the policy must select for those operations: equal
    // narrow operands keep their own narrow domain, mixed signedness uses the smallest signed
    // type containing both complete ranges, and negation preserves the signed width.
    assert_eq!(
        binary_operation_domain(Op::Add, fixed(U8), fixed(U8)),
        Some(fixed(U8))
    );
    assert_eq!(
        binary_operation_domain(Op::Add, fixed(U8), fixed(I8)),
        Some(fixed(I16))
    );
    assert_eq!(negation_domain(fixed(I8)), Some(fixed(I8)));
    assert_eq!(negation_domain(fixed(I16)), Some(fixed(I16)));
}

#[test]
fn failure_codes_leave_exact_float_negation_and_dec_arithmetic_without_a_failure_edge() {
    use crate::compiler_frontend::builtins::error_codes::BuiltinErrorCode;
    use NumericOperator as Op;

    // Binary-float negation is exact, so it carries no codes while every other binary-float
    // operation keeps its non-finite guard.
    assert!(numeric_failure_codes(Op::Negate, fixed(F32)).is_empty());
    assert_eq!(
        numeric_failure_codes(Op::Add, fixed(F32)),
        &[BuiltinErrorCode::FloatNonFinite]
    );
    assert_eq!(
        numeric_failure_codes(Op::Divide, fixed(F32)),
        &[
            BuiltinErrorCode::DivideByZero,
            BuiltinErrorCode::FloatNonFinite
        ]
    );

    // `F16` keeps its own float semantic domain, so it retains the same finite-value guard and
    // exact negation rather than being computed and failed at `F32`.
    assert!(numeric_failure_codes(Op::Negate, fixed(F16)).is_empty());
    assert_eq!(
        numeric_failure_codes(Op::Add, fixed(F16)),
        &[BuiltinErrorCode::FloatNonFinite]
    );

    // Exact `Dec` arithmetic is exact at its scale; only dividing operations and power
    // retain codes.
    for operator in [Op::Add, Op::Subtract, Op::Multiply, Op::Negate] {
        assert!(
            numeric_failure_codes(operator, number(0)).is_empty(),
            "{operator:?} on Dec must carry no failure codes"
        );
    }
    assert_eq!(
        numeric_failure_codes(Op::Remainder, number(0)),
        &[BuiltinErrorCode::DivideByZero]
    );
    assert_eq!(
        numeric_failure_codes(Op::Power, number(1)),
        &[BuiltinErrorCode::InvalidExponent]
    );

    // Integer operations always retain at least one code; the discharge predicate may
    // still prove fixed-width cases unable to fail.
    for operator in BINARY_OPERATORS {
        assert!(
            !numeric_failure_codes(operator, fixed(I32)).is_empty(),
            "{operator:?} on I32 must retain failure codes"
        );
    }
    // Unary integer negation keeps its overflow edge: negating a signed minimum fails.
    assert_eq!(
        numeric_failure_codes(Op::Negate, fixed(I8)),
        &[BuiltinErrorCode::IntOverflow]
    );
}

#[test]
fn range_iteration_uses_the_fixed_pairwise_table_and_rejects_dec() {
    // Fixed-width operands keep the pairwise table, including mixed-width promotion.
    assert_eq!(
        range_iteration_domain(fixed(I8), fixed(U16), Some(fixed(U16))),
        Some(fixed(I32))
    );
    // An equal narrow fixed type keeps its own counter domain instead of a wider floor.
    assert_eq!(
        range_iteration_domain(fixed(U8), fixed(U8), Some(fixed(U8))),
        Some(fixed(U8))
    );
    assert_eq!(
        range_iteration_domain(fixed(I8), fixed(I8), None),
        Some(fixed(I8))
    );

    // Binary floats promote by precision, but an integer/float mixture stays rejected:
    // a float step cannot rescue it.
    assert_eq!(
        range_iteration_domain(fixed(F16), fixed(F32), None),
        Some(fixed(F32))
    );
    assert_eq!(
        range_iteration_domain(fixed(F16), fixed(U16), Some(fixed(F32))),
        None
    );

    // `Dec` operands never form a range counter.
    assert_eq!(range_iteration_domain(number(2), number(2), None), None);
    assert_eq!(
        range_iteration_domain(fixed(I8), number(2), Some(fixed(I8))),
        None
    );
}

#[test]
fn byte_is_outside_every_numeric_operator_domain() {
    let byte = fixed(Byte);

    for operator in BINARY_OPERATORS {
        assert_eq!(binary_operation_domain(operator, byte, byte), None);
        for scalar in FIXED_INTEGERS.into_iter().chain(FIXED_FLOATS) {
            assert_eq!(
                binary_operation_domain(operator, byte, fixed(scalar)),
                None,
                "Byte {operator:?} {scalar:?}"
            );
            assert_eq!(
                binary_operation_domain(operator, fixed(scalar), byte),
                None,
                "{scalar:?} {operator:?} Byte"
            );
            assert!(!comparison_supported(byte, fixed(scalar)));
            assert!(!comparison_supported(fixed(scalar), byte));
        }
    }

    assert!(!comparison_supported(byte, byte));
    assert_eq!(range_iteration_domain(byte, byte, None), None);
    assert!(!numeric_operation_cannot_fail(
        NumericOperator::Add,
        byte,
        Some(byte),
        byte
    ));
}
