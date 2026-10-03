//! Backend-neutral numeric operator vocabulary and promotion policy.
//!
//! WHAT: the numeric operator set shared by AST typing, constant folding and HIR checked
//!       operations, plus the one rule table that selects each operation's computation domain.
//! WHY: operator typing, folding and lowering must agree on every operand pair. Deriving all three
//!      from this table keeps one promotion matrix instead of an Int/Float-only classifier in each
//!      stage that later widths would have to re-teach.
//!
//! The computation domain is also the result type: both operands convert to it before the
//! operation runs, except `Dec ^ Int` keeps its exponent in the profile `Int` domain.
//! `Byte` is outside `NumericScalar` and never reaches this policy.

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
    /// The canonical operator name used in HIR display (`Int.add`, `Float.divide`, ...).
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
/// - `Int` and `Float` retain their existing convenience rules.
/// - Two fixed integers use their common integer type; compatible `/` uses `F64`.
/// - Two fixed binary floats use the wider precision with a minimum of `F32`.
/// - `Dec` operations retain the Dec operand's scale. They mix only with `Int` or fixed
///   integers, except `Dec ^ Int`, whose exponent remains in the profile `Int` domain.
/// - Different Dec scales, Byte, binary floats and ordinary Int/fixed mixtures reject.
pub(crate) fn binary_operation_domain(
    operator: NumericOperator,
    left: NumericScalar,
    right: NumericScalar,
) -> Option<NumericScalar> {
    if operator.is_unary() {
        return None;
    }

    match (left, right) {
        (NumericScalar::Int, NumericScalar::Int) => Some(match operator {
            NumericOperator::Divide => NumericScalar::Float,
            _ => NumericScalar::Int,
        }),

        (NumericScalar::Int | NumericScalar::Float, NumericScalar::Int | NumericScalar::Float) => {
            (operator != NumericOperator::IntegerDivide).then_some(NumericScalar::Float)
        }

        (NumericScalar::Fixed(left), NumericScalar::Fixed(right)) => {
            fixed_binary_operation_domain(operator, left, right)
        }

        (NumericScalar::Number(scale), NumericScalar::Int)
            if operator == NumericOperator::Power =>
        {
            Some(NumericScalar::Number(scale))
        }
        (NumericScalar::Number(scale), NumericScalar::Number(right_scale))
            if scale == right_scale =>
        {
            number_binary_operation_domain(operator, scale)
        }
        (NumericScalar::Number(scale), right) if is_integer_domain(right) => {
            number_binary_operation_domain(operator, scale)
        }
        (left, NumericScalar::Number(scale)) if is_integer_domain(left) => {
            number_binary_operation_domain(operator, scale)
        }
        _ => None,
    }
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
        let wider = if left.bit_width() >= right.bit_width() {
            left
        } else {
            right
        };
        return Some(NumericScalar::Fixed(fixed_float_domain(wider)));
    }

    None
}

/// `Int`, `Float` and `Dec` keep their own domain. Signed narrow integers use `I32`, `I64`
/// stays `I64`, and fixed floats use a minimum of `F32`. Every unsigned integer is rejected,
/// including a known zero: there is no automatic signed promotion for unary minus.
pub(crate) fn negation_domain(operand: NumericScalar) -> Option<NumericScalar> {
    match operand {
        NumericScalar::Int | NumericScalar::Float | NumericScalar::Number(_) => Some(operand),
        NumericScalar::Fixed(scalar) => match scalar.class() {
            FixedScalarClass::SignedInteger => {
                Some(NumericScalar::Fixed(if scalar == FixedScalar::I64 {
                    FixedScalar::I64
                } else {
                    FixedScalar::I32
                }))
            }
            FixedScalarClass::BinaryFloat => Some(NumericScalar::Fixed(fixed_float_domain(scalar))),
            FixedScalarClass::UnsignedInteger | FixedScalarClass::Octet => None,
        },
    }
}

/// Whether two numeric domains support equality and ordering with each other.
///
/// `Int` and `Float` compare in any mix. Fixed integer pairs compare exactly, including `I64`
/// with `U64`, and fixed binary floats compare by numeric value. `Dec` compares exactly at
/// equal scales and with `Int` or any fixed integer; different scales and non-integer families
/// reject.
pub(crate) fn comparison_supported(left: NumericScalar, right: NumericScalar) -> bool {
    match (left, right) {
        (NumericScalar::Int | NumericScalar::Float, NumericScalar::Int | NumericScalar::Float) => {
            true
        }
        (NumericScalar::Fixed(left), NumericScalar::Fixed(right)) => {
            (is_fixed_integer(left) && is_fixed_integer(right))
                || (is_fixed_float(left) && is_fixed_float(right))
        }
        (NumericScalar::Number(left_scale), NumericScalar::Number(right_scale)) => {
            left_scale == right_scale
        }
        (NumericScalar::Number(_), other) | (other, NumericScalar::Number(_)) => {
            is_integer_domain(other)
        }
        _ => false,
    }
}

/// The common arithmetic type of two fixed integers, or `None` when no fixed integer type
/// represents every value of both.
///
/// WHAT: same signedness selects the wider width with a minimum of 32 bits. Mixed signedness
///       selects the smallest signed type of at least 32 bits that holds both complete ranges,
///       so `I32 + U32 -> I64` and any `U64` pairing with a signed type has no common type.
/// WHY: the choice depends only on the operand types, never on known values or the receiving
///      context, so every stage reaches the same answer. Non-integer inputs return `None`.
pub(crate) fn common_fixed_integer(left: FixedScalar, right: FixedScalar) -> Option<FixedScalar> {
    let left_signed = left.class() == FixedScalarClass::SignedInteger;
    let right_signed = right.class() == FixedScalarClass::SignedInteger;

    if !is_fixed_integer(left) || !is_fixed_integer(right) {
        return None;
    }

    // A signed type needs one more bit than an unsigned type of the same width.
    let required_bits = match (left_signed, right_signed) {
        (true, true) | (false, false) => left.bit_width().max(right.bit_width()),
        (true, false) => left.bit_width().max(right.bit_width() + 1),
        (false, true) => right.bit_width().max(left.bit_width() + 1),
    };
    let signed = left_signed || right_signed;

    match (required_bits, signed) {
        (0..=32, true) => Some(FixedScalar::I32),
        (33..=64, true) => Some(FixedScalar::I64),
        (0..=32, false) => Some(FixedScalar::U32),
        (33..=64, false) => Some(FixedScalar::U64),
        _ => None,
    }
}

fn is_integer_domain(scalar: NumericScalar) -> bool {
    match scalar {
        NumericScalar::Int => true,
        NumericScalar::Fixed(scalar) => is_fixed_integer(scalar),
        NumericScalar::Float | NumericScalar::Number(_) => false,
    }
}

/// `F16` computes in `F32`; `F32` and `F64` keep their own precision.
fn fixed_float_domain(scalar: FixedScalar) -> FixedScalar {
    match scalar {
        FixedScalar::F16 => FixedScalar::F32,
        other => other,
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
