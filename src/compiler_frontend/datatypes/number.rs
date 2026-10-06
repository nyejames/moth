//! Immutable `Dec` values, exact arithmetic and conversions.
//!
//! WHAT: owns arbitrary-precision decimal coefficients, Dec arithmetic and the exact
//!       integer/rescaling conversions shared by frontend consumers.
//! WHY: AST, casts and constant folding must use one coefficient owner without a bounded or
//!      floating-point intermediate.
//!
//! Exclusions: numeric syntax, `NumberScale` and normalized-decimal facts belong to
//! `moth-lexical`; MON decimal text validation, budgets and error context belong to the
//! MON codec, which keeps its own text representation; runtime/backend representation
//! does not belong here.

use crate::compiler_frontend::datatypes::numeric_operators::NumericOperator;
use moth_lexical::numeric::decimal::{NormalizedDecimalFacts, NumberScale};
use moth_lexical::numeric::grammar::NumericLiteralSign;
use num_bigint::{BigInt, Sign};
use num_integer::Integer;
use num_traits::{One, ToPrimitive, Zero};
use std::fmt::{self, Display, Formatter};
use std::str::FromStr;
use std::sync::Arc;

/// One failure to materialize an exact `Dec` literal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NumberMaterializationError {
    /// The normalized decimal value needs more fractional places than the destination scale.
    InexactScale,
    /// A checked host-size or fallible text-buffer allocation could not represent the value.
    Capacity,
}

impl Display for NumberMaterializationError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            NumberMaterializationError::InexactScale => {
                formatter.write_str("the decimal value is not exact at this Dec scale")
            }

            NumberMaterializationError::Capacity => {
                formatter.write_str("the Dec value exceeds host representation capacity")
            }
        }
    }
}

impl std::error::Error for NumberMaterializationError {}

/// A Dec-to-integer conversion failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NumberIntegerConversionError {
    /// The value has nonzero fractional digits at its current scale.
    Inexact,
    /// The exact integral value does not fit the compiler's i128 conversion carrier.
    OutOfRange,
}

impl Display for NumberIntegerConversionError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            NumberIntegerConversionError::Inexact => {
                formatter.write_str("the Dec value has a fractional part")
            }
            NumberIntegerConversionError::OutOfRange => {
                formatter.write_str("the Dec integer value is outside the i128 range")
            }
        }
    }
}

impl std::error::Error for NumberIntegerConversionError {}

/// A checked Dec arithmetic failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NumberArithmeticError {
    /// Division or remainder received a zero divisor.
    DivideByZero,
    /// Exponentiation received a negative exponent.
    InvalidExponent,
    /// The operator, scale or operand shape is outside Dec's arithmetic contract.
    InvalidOperation,
}

impl Display for NumberArithmeticError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            NumberArithmeticError::DivideByZero => formatter.write_str("division by zero"),
            NumberArithmeticError::InvalidExponent => formatter.write_str("invalid exponent"),
            NumberArithmeticError::InvalidOperation => {
                formatter.write_str("invalid Dec arithmetic operation")
            }
        }
    }
}

impl std::error::Error for NumberArithmeticError {}

/// Immutable arbitrary-precision decimal coefficient and canonical scale.
///
/// WHAT: stores `coefficient / 10^scale` without a profile integer or floating-point intermediate.
/// WHY: frontend value handoffs share the exact coefficient while retaining one immutable value
///      identity; backends own the runtime representation.
///
/// `normalized` passed to [`NumberValue::from_normalized`] is already grammar-validated by
/// the shared lexical parser. This type consumes those facts but does not own or repeat syntax.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NumberValue {
    coefficient: Arc<BigInt>,
    scale: NumberScale,
}

impl NumberValue {
    /// Materialize normalized numeric text exactly at the requested scale.
    ///
    /// The shared borrowed facts decide whether the value fits before this method creates an
    /// expanded coefficient buffer. A zero coefficient is returned first, without examining or
    /// expanding its exponent, so even a huge signed zero remains cheap.
    pub fn from_normalized(
        normalized: &str,
        sign: NumericLiteralSign,
        scale: NumberScale,
    ) -> Result<Self, NumberMaterializationError> {
        let facts = NormalizedDecimalFacts::analyze(normalized);
        if facts.coefficient_is_zero() {
            return Ok(Self::new(BigInt::zero(), scale));
        }

        // The shared borrowed facts decide exact fit before any coefficient expansion. The MON
        // codec validates the same trailing-zero and exponent interpretation for decimal text,
        // keeping its own representation.
        if facts.effective_scale() > usize::from(scale.get()) {
            return Err(NumberMaterializationError::InexactScale);
        }

        let exponent_magnitude = facts
            .exponent_magnitude()
            .ok_or(NumberMaterializationError::Capacity)?;
        let negative_exponent = facts.negative_exponent();
        let integer_part = facts.integer_part();
        let fractional_part = facts.fractional_part();

        // `scale + exponent - fractional length` is the decimal shift applied to the
        // coefficient. Separate addition and removal keep it non-negative and checked.
        let positive_shift = if negative_exponent {
            usize::from(scale.get())
        } else {
            usize::from(scale.get())
                .checked_add(exponent_magnitude)
                .ok_or(NumberMaterializationError::Capacity)?
        };
        let negative_shift = if negative_exponent {
            fractional_part
                .len()
                .checked_add(exponent_magnitude)
                .ok_or(NumberMaterializationError::Capacity)?
        } else {
            fractional_part.len()
        };
        let (appended_zeroes, removed_zeroes) = if positive_shift >= negative_shift {
            (positive_shift - negative_shift, 0)
        } else {
            (0, negative_shift - positive_shift)
        };

        let coefficient_digit_count = integer_part
            .len()
            .checked_add(fractional_part.len())
            .ok_or(NumberMaterializationError::Capacity)?;
        if removed_zeroes > facts.trailing_zeroes() {
            // The shared exact-scale check should make this impossible. Preserve a safe
            // failure if the scale policy and transformation ever drift apart.
            return Err(NumberMaterializationError::InexactScale);
        }

        let retained_digit_count = coefficient_digit_count
            .checked_sub(removed_zeroes)
            .ok_or(NumberMaterializationError::Capacity)?;
        let expanded_digit_count = retained_digit_count
            .checked_add(appended_zeroes)
            .ok_or(NumberMaterializationError::Capacity)?;

        // Whole coefficients with no shift can be parsed from their borrowed source bytes.
        let coefficient =
            if fractional_part.is_empty() && appended_zeroes == 0 && removed_zeroes == 0 {
                parse_coefficient(integer_part)?
            } else {
                let mut expanded_digits = String::new();
                expanded_digits
                    .try_reserve_exact(expanded_digit_count)
                    .map_err(|_| NumberMaterializationError::Capacity)?;

                let integer_digit_count = integer_part.len().min(retained_digit_count);
                expanded_digits.push_str(&integer_part[..integer_digit_count]);

                if retained_digit_count > integer_part.len() {
                    let retained_fraction_digits = retained_digit_count - integer_part.len();
                    expanded_digits.push_str(&fractional_part[..retained_fraction_digits]);
                }

                for _ in 0..appended_zeroes {
                    expanded_digits.push('0');
                }

                parse_coefficient(&expanded_digits)?
            };

        let coefficient = match sign {
            NumericLiteralSign::Positive => coefficient,
            NumericLiteralSign::Negative => -coefficient,
        };
        Ok(Self::new(coefficient, scale))
    }

    fn new(coefficient: BigInt, scale: NumberScale) -> Self {
        Self {
            coefficient: Arc::new(coefficient),
            scale,
        }
    }

    /// Construct the exact scaled coefficient for an integer in the destination Dec domain.
    pub fn from_integer(value: i128, scale: NumberScale) -> Self {
        if value == 0 || scale == NumberScale::ZERO {
            return Self::new(BigInt::from(value), scale);
        }

        Self::new(BigInt::from(value) * decimal_scale_factor(scale), scale)
    }

    /// Convert this value exactly to another canonical scale.
    ///
    /// Narrowing removes decimal places only when every removed coefficient digit is zero.
    /// Widening multiplies by a power of ten and never passes through a bounded integer carrier.
    pub fn rescale(&self, target: NumberScale) -> Result<Self, NumberMaterializationError> {
        if target == self.scale {
            return Ok(self.clone());
        }

        // Zero's coefficient is scale-invariant, so rescaling can preserve its shared allocation.
        if self.coefficient.is_zero() {
            return Ok(Self {
                coefficient: Arc::clone(&self.coefficient),
                scale: target,
            });
        }

        if target > self.scale {
            let shift = NumberScale::new(target.get() - self.scale.get())
                .expect("a scale difference cannot exceed NumberScale::MAX");
            return Ok(Self::new(
                self.coefficient.as_ref() * decimal_scale_factor(shift),
                target,
            ));
        }

        let shift = NumberScale::new(self.scale.get() - target.get())
            .expect("a scale difference cannot exceed NumberScale::MAX");
        let divisor = decimal_scale_factor(shift);
        let (coefficient, remainder) = self.coefficient.div_rem(&divisor);
        if !remainder.is_zero() {
            return Err(NumberMaterializationError::InexactScale);
        }

        Ok(Self::new(coefficient, target))
    }

    /// Return the exact integer value, distinguishing fractional values from carrier overflow.
    pub fn to_integer_exact(&self) -> Result<i128, NumberIntegerConversionError> {
        if self.scale == NumberScale::ZERO {
            return self
                .coefficient
                .to_i128()
                .ok_or(NumberIntegerConversionError::OutOfRange);
        }
        if self.coefficient.is_zero() {
            return Ok(0);
        }

        let divisor = decimal_scale_factor(self.scale);
        let (integer, remainder) = self.coefficient.div_rem(&divisor);
        if !remainder.is_zero() {
            return Err(NumberIntegerConversionError::Inexact);
        }

        integer
            .to_i128()
            .ok_or(NumberIntegerConversionError::OutOfRange)
    }

    /// Negate the coefficient without changing its scale.
    pub fn negated(&self) -> Self {
        Self::new(-self.coefficient.as_ref(), self.scale)
    }

    /// Apply one supported non-power binary operation at this Dec scale.
    ///
    /// Multiplication and division use exact rational intermediates and round once to the
    /// result scale. Division and remainder retain BigInt's truncation-toward-zero quotient.
    pub fn checked_binary(
        &self,
        operator: NumericOperator,
        right: &Self,
    ) -> Result<Self, NumberArithmeticError> {
        if self.scale != right.scale {
            return Err(NumberArithmeticError::InvalidOperation);
        }

        let coefficient = match operator {
            NumericOperator::Add => self.coefficient.as_ref() + right.coefficient.as_ref(),
            NumericOperator::Subtract => self.coefficient.as_ref() - right.coefficient.as_ref(),
            NumericOperator::Multiply if self.scale == NumberScale::ZERO => {
                self.coefficient.as_ref() * right.coefficient.as_ref()
            }
            NumericOperator::Multiply => round_rational_half_even(
                self.coefficient.as_ref() * right.coefficient.as_ref(),
                &decimal_scale_factor(self.scale),
            ),
            NumericOperator::Divide => {
                if self.scale == NumberScale::ZERO {
                    return Err(NumberArithmeticError::InvalidOperation);
                }
                if right.coefficient.is_zero() {
                    return Err(NumberArithmeticError::DivideByZero);
                }

                round_rational_half_even(
                    self.coefficient.as_ref() * decimal_scale_factor(self.scale),
                    right.coefficient.as_ref(),
                )
            }
            NumericOperator::IntegerDivide => {
                if self.scale != NumberScale::ZERO {
                    return Err(NumberArithmeticError::InvalidOperation);
                }
                if right.coefficient.is_zero() {
                    return Err(NumberArithmeticError::DivideByZero);
                }

                self.coefficient.as_ref() / right.coefficient.as_ref()
            }
            NumericOperator::Remainder => {
                if right.coefficient.is_zero() {
                    return Err(NumberArithmeticError::DivideByZero);
                }

                self.coefficient.as_ref() % right.coefficient.as_ref()
            }
            NumericOperator::Power | NumericOperator::Negate => {
                return Err(NumberArithmeticError::InvalidOperation);
            }
        };

        Ok(Self::new(coefficient, self.scale))
    }

    /// Raise this Dec to a non-negative Int exponent, rounding only the final result.
    pub fn checked_power(&self, exponent: i64) -> Result<Self, NumberArithmeticError> {
        if exponent < 0 {
            return Err(NumberArithmeticError::InvalidExponent);
        }

        if exponent == 0 {
            return Ok(Self::new(decimal_scale_factor(self.scale), self.scale));
        }
        if exponent == 1 || self.coefficient.is_zero() {
            return Ok(self.clone());
        }

        let unity = decimal_scale_factor(self.scale);
        if self.coefficient.as_ref() == &unity {
            return Ok(self.clone());
        }

        if self.coefficient.sign() == Sign::Minus
            && self.coefficient.magnitude() == unity.magnitude()
        {
            return if exponent % 2 == 1 {
                Ok(self.clone())
            } else {
                Ok(Self::new(unity, self.scale))
            };
        }

        let exponent = u64::try_from(exponent)
            .expect("a non-negative i64 exponent always fits the exact u64 power loop");
        if self.scale == NumberScale::ZERO {
            return Ok(Self::new(
                bigint_power(self.coefficient.as_ref(), exponent),
                self.scale,
            ));
        }

        // At a positive scale, a coefficient of +/-1 has magnitude at most 0.1. Its square and
        // every higher power round to zero at this scale, without expanding a huge denominator.
        if self.coefficient.magnitude().is_one() {
            return Ok(Self::new(BigInt::zero(), self.scale));
        }

        // Integral bases divide out the removable scale factor first: `(c / F)^e * F` is exact.
        // For Dec256 `2 ^ 10000`, raise the unscaled base 2, then restore the scale factor. The
        // final coefficient has 3,267 digits, without multi-million-digit cancelling
        // intermediates. The quotient keeps the base sign, so negative integral bases stay
        // exact. Unity quotients returned above.
        let (quotient, remainder) = self.coefficient.as_ref().div_rem(&unity);
        if remainder.is_zero() {
            return Ok(Self::new(
                bigint_power(&quotient, exponent) * &unity,
                self.scale,
            ));
        }

        let exact_numerator = bigint_power(self.coefficient.as_ref(), exponent);
        let exact_denominator = bigint_power(&unity, exponent - 1);
        let coefficient = round_rational_half_even(exact_numerator, &exact_denominator);
        Ok(Self::new(coefficient, self.scale))
    }

    /// The canonical scale carried by this value.
    pub const fn scale(&self) -> NumberScale {
        self.scale
    }

    /// The immutable exact coefficient carried by this value.
    pub fn coefficient(&self) -> &BigInt {
        self.coefficient.as_ref()
    }
}

fn decimal_scale_factor(scale: NumberScale) -> BigInt {
    BigInt::from(10_u8).pow(u32::from(scale.get()))
}

/// Exact binary exponentiation keeps intermediate Dec powers unrounded.
fn bigint_power(base: &BigInt, exponent: u64) -> BigInt {
    let mut power = exponent;
    let mut factor = base.clone();
    let mut result = BigInt::one();

    while power != 0 {
        if power & 1 == 1 {
            result *= &factor;
        }

        power >>= 1;
        if power != 0 {
            factor = &factor * &factor;
        }
    }

    result
}

/// Round one exact rational toward the nearest integer, resolving exact ties to even.
fn round_rational_half_even(numerator: BigInt, denominator: &BigInt) -> BigInt {
    // Unit denominators are exact and need no quotient/remainder pair.
    if denominator.magnitude().is_one() {
        return if denominator.sign() == Sign::Minus {
            -numerator
        } else {
            numerator
        };
    }

    let (mut quotient, remainder) = numerator.div_rem(denominator);
    if remainder.is_zero() {
        return quotient;
    }

    let doubled_remainder = remainder.magnitude() * 2_u8;
    match doubled_remainder.cmp(denominator.magnitude()) {
        std::cmp::Ordering::Less => quotient,
        std::cmp::Ordering::Equal if !quotient.is_odd() => quotient,
        std::cmp::Ordering::Equal | std::cmp::Ordering::Greater => {
            if numerator.sign() == denominator.sign() {
                quotient += 1;
            } else {
                quotient -= 1;
            }
            quotient
        }
    }
}

fn parse_coefficient(digits: &str) -> Result<BigInt, NumberMaterializationError> {
    BigInt::from_str(digits).map_err(|_| NumberMaterializationError::Capacity)
}

impl Display for NumberValue {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        let scale = usize::from(self.scale.get());
        if scale == 0 {
            return Display::fmt(self.coefficient.as_ref(), formatter);
        }

        if self.coefficient.is_zero() {
            return formatter.write_str("0");
        }

        let coefficient_text = self.coefficient.to_str_radix(10);
        let negative = self.coefficient.sign() == Sign::Minus;
        let digits = coefficient_text
            .strip_prefix('-')
            .unwrap_or(&coefficient_text);
        let integer_digit_count = digits.len().saturating_sub(scale);
        let mut fractional_end = digits.len();
        while fractional_end > integer_digit_count && digits.as_bytes()[fractional_end - 1] == b'0'
        {
            fractional_end -= 1;
        }

        if negative {
            formatter.write_str("-")?;
        }

        if integer_digit_count == 0 {
            formatter.write_str("0")?;
        } else {
            formatter.write_str(&digits[..integer_digit_count])?;
        }

        if fractional_end > integer_digit_count {
            formatter.write_str(".")?;
            for _ in digits.len()..scale {
                formatter.write_str("0")?;
            }
            formatter.write_str(&digits[integer_digit_count..fractional_end])?;
        }

        Ok(())
    }
}
