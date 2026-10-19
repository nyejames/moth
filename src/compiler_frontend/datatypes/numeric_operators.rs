//! Backend-neutral numeric operator vocabulary and promotion policy.
//!
//! WHAT: the numeric operator set shared by AST typing, constant folding and HIR checked
//!       operations, plus the one rule table that selects each operation's computation domain.
//! WHY: operator typing, folding and lowering must agree on every operand pair. Deriving all three
//!      from this table keeps one promotion matrix instead of a per-stage classifier that every
//!      new width would have to re-teach.
//!
//! The computation domain is also the result type: both operands convert to it before the
//! operation runs, except `Dec ^ I32` keeps its exact `I32` exponent contract.
//! `Byte` is outside `NumericScalar` and never reaches this policy.

use crate::compiler_frontend::builtins::error_codes::BuiltinErrorCode;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use moth_lexical::numeric::decimal::NumberScale;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarClass};

/// One backend-neutral numeric operator.
///
/// WHAT: the arithmetic shape without its scalar domain. `Divide` is real `/` and
///       `IntegerDivide` is truncating `//`.
/// WHY: the operator vocabulary stays fixed while numeric domains grow; typing, folding, HIR
///      validation and backends combine it with `NumericScalar` to recover the full operation.
///
/// Declared `pub` inside the crate-private `datatypes::numeric_operators` module so public HIR
/// types such as `HirNumericOp` can carry it; the effective visibility stays crate-internal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumericOperator {
    Add,
    Subtract,
    Multiply,
    Divide,
    IntegerDivide,
    Remainder,
    Power,
    Negate,
}

impl NumericOperator {
    /// The canonical operator name used in HIR display (`I8.add`, `F32.divide`, ...).
    pub(crate) fn name(self) -> &'static str {
        match self {
            NumericOperator::Add => "add",
            NumericOperator::Subtract => "subtract",
            NumericOperator::Multiply => "multiply",
            NumericOperator::Divide => "divide",
            NumericOperator::IntegerDivide => "integer_divide",
            NumericOperator::Remainder => "remainder",
            NumericOperator::Power => "power",
            NumericOperator::Negate => "negate",
        }
    }

    /// Whether the operator takes one operand.
    pub(crate) fn is_unary(self) -> bool {
        matches!(self, NumericOperator::Negate)
    }
}

/// The computation (and result) domain of a binary arithmetic operation, or `None` when the
/// operand pair does not support the operator.
///
/// - Two fixed integers use their common integer type; eligible `/` computes in `F64`.
/// - Two fixed binary floats use the wider precision, with `F16` keeping its own precision;
///   `//` is invalid between binary floats.
/// - Fixed integers and binary floats never mix: those pairs need an explicit conversion.
/// - `Dec` operations retain the Dec operand's scale and mix with fixed integers, except
///   `Dec ^ I32`, whose exponent stays the exact `I32` contract. No other exponent is an
///   implicit `Dec` exponent.
/// - Different Dec scales, `Byte` and non-integer `Dec` mixes reject.
pub(crate) fn binary_operation_domain(
    operator: NumericOperator,
    left: NumericScalar,
    right: NumericScalar,
) -> Option<NumericScalar> {
    if operator.is_unary() {
        return None;
    }

    match (left, right) {
        (NumericScalar::Fixed(left), NumericScalar::Fixed(right)) => {
            fixed_binary_operation_domain(operator, left, right)
        }

        (NumericScalar::Number(scale), NumericScalar::Fixed(FixedScalar::I32))
            if operator == NumericOperator::Power =>
        {
            Some(NumericScalar::Number(scale))
        }
        (NumericScalar::Number(scale), NumericScalar::Number(right_scale))
            if scale == right_scale =>
        {
            number_binary_operation_domain(operator, scale)
        }
        (NumericScalar::Number(scale), right) if right.is_integer() => {
            number_binary_operation_domain(operator, scale)
        }
        (left, NumericScalar::Number(scale)) if left.is_integer() => {
            number_binary_operation_domain(operator, scale)
        }
        _ => None,
    }
}

/// The selected iteration domain for a range header, or `None` when the operand
/// mix has no range domain.
///
/// WHAT: fixed-width operands combine through the pairwise `Add` table, so mixed
///       widths promote through their common integer/float type (e.g. `I8`/`U16`)
///       while integer/float mixtures stay rejected. `Dec` operands reject
///       outright: a range counter is not an ordinary Dec-mixing expression.
/// WHY: a range header combines three operands at once, while `Add` only sees
///      pairs; reusing that one table keeps a single shared domain selector.
///      Endpoint, no-progress and termination semantics stay range-owned and are
///      not ordinary checked arithmetic. AST parsing selects the header's
///      iteration domain through this selector; HIR lowering and const folding
///      then consume that selected domain.
pub(crate) fn range_iteration_domain(
    start: NumericScalar,
    end: NumericScalar,
    step: Option<NumericScalar>,
) -> Option<NumericScalar> {
    if [Some(start), Some(end), step]
        .into_iter()
        .flatten()
        .any(|operand| matches!(operand, NumericScalar::Number(_)))
    {
        return None;
    }

    // Fixed-width operands keep the pairwise table: mixed widths promote through their
    // common integer/float type, while integer/float mixtures stay rejected.
    binary_operation_domain(
        NumericOperator::Add,
        binary_operation_domain(NumericOperator::Add, start, end)?,
        step.unwrap_or(start),
    )
}
fn number_binary_operation_domain(
    operator: NumericOperator,
    scale: NumberScale,
) -> Option<NumericScalar> {
    let supported = match operator {
        NumericOperator::Add
        | NumericOperator::Subtract
        | NumericOperator::Multiply
        | NumericOperator::Remainder => true,
        NumericOperator::Divide => scale != NumberScale::ZERO,
        NumericOperator::IntegerDivide => scale == NumberScale::ZERO,
        NumericOperator::Power | NumericOperator::Negate => false,
    };
    supported.then_some(NumericScalar::Number(scale))
}

fn fixed_binary_operation_domain(
    operator: NumericOperator,
    left: FixedScalar,
    right: FixedScalar,
) -> Option<NumericScalar> {
    if is_fixed_integer(left) && is_fixed_integer(right) {
        let common = common_fixed_integer(left, right)?;
        let domain = match operator {
            NumericOperator::Divide => FixedScalar::F64,
            _ => common,
        };
        return Some(NumericScalar::Fixed(domain));
    }

    if is_fixed_float(left) && is_fixed_float(right) {
        if operator == NumericOperator::IntegerDivide {
            return None;
        }
        // D15: different precisions keep the wider one and equal precisions keep their own
        // type, so `F16` has no minimum-`F32` semantic domain.
        let wider = if left.bit_width() >= right.bit_width() {
            left
        } else {
            right
        };
        return Some(NumericScalar::Fixed(wider));
    }

    None
}

/// `Dec` keeps its scale, and a fixed signed integer or binary float keeps its own type:
/// `-I8` is `I8`, `-I64` is `I64` and `-F16` is `F16`. Every unsigned integer and `Byte` is
/// rejected, including a known zero: there is no automatic signed promotion for unary minus.
pub(crate) fn negation_domain(operand: NumericScalar) -> Option<NumericScalar> {
    match operand {
        NumericScalar::Number(_) => Some(operand),
        NumericScalar::Fixed(scalar) => match scalar.class() {
            FixedScalarClass::SignedInteger | FixedScalarClass::BinaryFloat => Some(operand),
            FixedScalarClass::UnsignedInteger | FixedScalarClass::Octet => None,
        },
    }
}

/// Whether two numeric domains support equality and ordering with each other.
///
/// Fixed integer pairs compare exactly, including `I64` with `U64`, and fixed binary floats
/// compare by numeric value. Fixed integers and binary floats never compare: those pairs need an
/// explicit conversion. `Dec` compares exactly at equal scales and with any fixed integer;
/// different scales, non-integer `Dec` mixes and `Byte` reject.
pub(crate) fn comparison_supported(left: NumericScalar, right: NumericScalar) -> bool {
    match (left, right) {
        (NumericScalar::Fixed(left), NumericScalar::Fixed(right)) => {
            (is_fixed_integer(left) && is_fixed_integer(right))
                || (is_fixed_float(left) && is_fixed_float(right))
        }
        (NumericScalar::Number(_), NumericScalar::Fixed(fixed))
        | (NumericScalar::Fixed(fixed), NumericScalar::Number(_)) => is_fixed_integer(fixed),
        (NumericScalar::Number(left_scale), NumericScalar::Number(right_scale)) => {
            left_scale == right_scale
        }
    }
}

/// Exact comparison semantics, distinct from the arithmetic common-domain policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumericComparisonDomain {
    ExactInteger,
    BinaryFloat,
    Number(NumberScale),
}

pub(crate) fn comparison_domain(
    left: NumericScalar,
    right: NumericScalar,
) -> Option<NumericComparisonDomain> {
    if !comparison_supported(left, right) {
        return None;
    }
    match (left, right) {
        (NumericScalar::Number(scale), _) | (_, NumericScalar::Number(scale)) => {
            Some(NumericComparisonDomain::Number(scale))
        }
        _ if left.is_integer() && right.is_integer() => Some(NumericComparisonDomain::ExactInteger),
        _ => Some(NumericComparisonDomain::BinaryFloat),
    }
}

/// The common arithmetic type of two fixed integers, or `None` when no fixed integer type
/// represents every value of both.
///
/// WHAT: the narrowest supported fixed integer type whose complete range contains both complete
///       operand ranges. Equal types keep their own type (`I8 + I8 -> I8`, `U8 + U8 -> U8`),
///       same-signedness operands take the wider width, and mixed signedness takes the smallest
///       signed type, so `I32 + U32 -> I64` and any `U64` pairing with a signed type has no
///       common type. No 32-bit minimum applies.
/// WHY: the choice depends only on the operand types, never on known values or the receiving
///      context, so every stage reaches the same answer. Non-integer inputs return `None`.
pub(crate) fn common_fixed_integer(left: FixedScalar, right: FixedScalar) -> Option<FixedScalar> {
    if !is_fixed_integer(left) || !is_fixed_integer(right) {
        return None;
    }

    let left_signed = left.class() == FixedScalarClass::SignedInteger;
    let right_signed = right.class() == FixedScalarClass::SignedInteger;

    // A signed type needs one more bit than an unsigned type of the same width, so the required
    // width is the larger operand width, plus one when a signed type must cover an unsigned one.
    let required_bits = match (left_signed, right_signed) {
        (true, true) | (false, false) => left.bit_width().max(right.bit_width()),
        (true, false) => left.bit_width().max(right.bit_width() + 1),
        (false, true) => right.bit_width().max(left.bit_width() + 1),
    };

    // D15: the smallest supported width that contains both complete operand ranges. A `U64`
    // paired with a signed type needs 65 bits and therefore has no common type.
    match (required_bits, left_signed || right_signed) {
        (0..=8, true) => Some(FixedScalar::I8),
        (9..=16, true) => Some(FixedScalar::I16),
        (17..=32, true) => Some(FixedScalar::I32),
        (33..=64, true) => Some(FixedScalar::I64),
        (0..=8, false) => Some(FixedScalar::U8),
        (9..=16, false) => Some(FixedScalar::U16),
        (17..=32, false) => Some(FixedScalar::U32),
        (33..=64, false) => Some(FixedScalar::U64),
        _ => None,
    }
}

/// Whether a checked numeric operation provably cannot fail from its operand types alone.
/// WHAT: the single backend-independent semantic discharge predicate. True only when every
///       operand and the result domain are fixed-width integer scalars and the complete
///       canonical result range of `Add` / `Subtract` / `Multiply` / `Negate` lies inside the
///       result domain's range, computed with `i128` endpoint arithmetic. `Divide`,
///       `IntegerDivide`, `Remainder` and `Power` always return false, as does any operation
///       involving a binary float, `Dec`/`Number` or `Byte`.
/// WHY: AST failure facts and HIR emission share one proof of infallibility so debug and
///      release accept the same source. Optional backend check elision stays in
///      `analysis/numeric_proofs` and never feeds this predicate.
pub(crate) fn numeric_operation_cannot_fail(
    operator: NumericOperator,
    left: NumericScalar,
    right: Option<NumericScalar>,
    domain: NumericScalar,
) -> bool {
    let result_range = match domain.integer_range() {
        Some(range) => range,
        None => return false,
    };
    match operator {
        NumericOperator::Add | NumericOperator::Subtract | NumericOperator::Multiply => {
            let Some(right) = right else {
                return false;
            };
            let (Some(left_range), Some(right_range)) =
                (left.integer_range(), right.integer_range())
            else {
                return false;
            };
            let Some(result) = checked_result_range(operator, left_range, right_range) else {
                return false;
            };
            result.0 >= result_range.0 && result.1 <= result_range.1
        }
        NumericOperator::Negate => {
            if right.is_some() {
                return false;
            }
            let Some(operand_range) = left.integer_range() else {
                return false;
            };
            let Some(result) = operand_range
                .1
                .checked_neg()
                .and_then(|high| operand_range.0.checked_neg().map(|low| (high, low)))
            else {
                return false;
            };
            // Negation consumes its promoted domain: the operand already carries the result
            // type, so the mathematical range must fit the same domain.
            result.0 >= result_range.0 && result.1 <= result_range.1
        }
        NumericOperator::Divide
        | NumericOperator::IntegerDivide
        | NumericOperator::Remainder
        | NumericOperator::Power => false,
    }
}

/// Mathematical result range of one dischargeable binary operator over closed `i128` intervals.
///
/// WHAT: `None` when a checked `i128` intermediate cannot represent the result; callers treat
///       that as unproved, never wrapping.
fn checked_result_range(
    operator: NumericOperator,
    left: (i128, i128),
    right: (i128, i128),
) -> Option<(i128, i128)> {
    match operator {
        NumericOperator::Add => Some((left.0.checked_add(right.0)?, left.1.checked_add(right.1)?)),
        NumericOperator::Subtract => {
            Some((left.0.checked_sub(right.1)?, left.1.checked_sub(right.0)?))
        }
        NumericOperator::Multiply => {
            let mut low = i128::MAX;
            let mut high = i128::MIN;
            for left_endpoint in [left.0, left.1] {
                for right_endpoint in [right.0, right.1] {
                    let product = left_endpoint.checked_mul(right_endpoint)?;
                    low = low.min(product);
                    high = high.max(product);
                }
            }
            Some((low, high))
        }
        _ => None,
    }
}

fn is_fixed_integer(scalar: FixedScalar) -> bool {
    matches!(
        scalar.class(),
        FixedScalarClass::SignedInteger | FixedScalarClass::UnsignedInteger
    )
}

fn is_fixed_float(scalar: FixedScalar) -> bool {
    scalar.class() == FixedScalarClass::BinaryFloat
}

/// The recoverable builtin failure codes of one checked numeric operation.
///
/// WHAT: the single owner of the operator/domain code set. Empty means the operation has no
///       runtime failure edge: binary-float negation is exact, and non-dividing, non-power
///       `Dec` arithmetic is exact at its scale.
/// WHY: AST failure facts, the AST active-contributor verdict, HIR lane installation and
///      convergence must agree on which operations can fail. A second table would let one
///      stage split or reject what another stage calls infallible.
pub(crate) fn numeric_failure_codes(
    operator: NumericOperator,
    domain: NumericScalar,
) -> &'static [BuiltinErrorCode] {
    let divides = matches!(
        operator,
        NumericOperator::Divide | NumericOperator::IntegerDivide | NumericOperator::Remainder
    );
    if domain.is_binary_float() {
        if divides {
            return &[
                BuiltinErrorCode::DivideByZero,
                BuiltinErrorCode::FloatNonFinite,
            ];
        }
        return if operator == NumericOperator::Negate {
            &[]
        } else {
            &[BuiltinErrorCode::FloatNonFinite]
        };
    }
    if domain.is_integer() {
        return match operator {
            NumericOperator::Divide | NumericOperator::Remainder => {
                &[BuiltinErrorCode::DivideByZero]
            }
            NumericOperator::IntegerDivide => &[
                BuiltinErrorCode::DivideByZero,
                BuiltinErrorCode::IntOverflow,
            ],
            NumericOperator::Power => &[
                BuiltinErrorCode::IntOverflow,
                BuiltinErrorCode::InvalidExponent,
            ],
            _ => &[BuiltinErrorCode::IntOverflow],
        };
    }
    if divides {
        &[BuiltinErrorCode::DivideByZero]
    } else if operator == NumericOperator::Power {
        &[BuiltinErrorCode::InvalidExponent]
    } else {
        &[]
    }
}
