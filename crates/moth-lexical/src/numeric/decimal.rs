//! Shared decimal scales and allocation-free facts from normalized decimal text.
//!
//! WHAT: owns the validated `Dec` scale identity and exact scale analysis of a borrowed normalized
//!       decimal spelling.
//! WHY: compiler `NumberValue` materialization and MON scale checks must interpret an exponent and
//!       coefficient trailing zeroes identically, with one scan and no duplicate text buffer.
//!
//! Arbitrary-precision coefficient construction and arithmetic remain in the compiler.

use std::fmt::{self, Display, Formatter};

/// One validated canonical `Dec` scale.
///
/// A `NumberScale` is the internal carried form of a scale in `0..=256`; the public MON schema
/// still spells scales as `u16` and preparation converts it here or rejects it. Construction is
/// the single validation point, so a prepared Decimal node can never hold an unvalidated scale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NumberScale(u16);

impl NumberScale {
    /// The canonical scale capacity, shared with the `Dec0`..`Dec256` source identities.
    pub const MAX: u16 = 256;

    /// Scale-zero identity shared by `Dec` and `Dec0`.
    pub const ZERO: Self = Self(0);

    /// Validate a declared scale against the canonical capacity.
    pub const fn new(scale: u16) -> Option<Self> {
        if scale <= Self::MAX {
            Some(Self(scale))
        } else {
            None
        }
    }

    /// Parse only canonical source type spellings, without allocating.
    pub fn from_name(name: &str) -> Option<Self> {
        if name == "Dec" {
            return Some(Self::ZERO);
        }

        let digits = name.strip_prefix("Dec")?;
        if digits.is_empty()
            || (digits.len() > 1 && digits.starts_with('0'))
            || !digits.bytes().all(|byte| byte.is_ascii_digit())
        {
            return None;
        }

        Self::new(digits.parse().ok()?)
    }

    /// The validated scale value.
    pub const fn get(self) -> u16 {
        self.0
    }
}

impl Display for NumberScale {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        if *self == Self::ZERO {
            formatter.write_str("Dec")
        } else {
            write!(formatter, "Dec{}", self.get())
        }
    }
}

/// Effective scale of an already validated unsigned normalized decimal spelling.
///
/// The spelling is normalized numeric text: unsigned, underscore-free, lowercase `e`
/// exponent with an optional `+`/`-` marker. The effective scale is the untrimmed fractional scale
/// (fraction digits plus a negative exponent magnitude) less the coefficient's trailing zero
/// digits, clamped at zero. An all-zero coefficient fits any scale at zero regardless of exponent,
/// and extremely large exponents saturate at host `usize` bounds instead of allocating arbitrary
/// precision.
///
/// Callers keep their own span, error code and detail context; this returns the bare fact.
///
/// WHY: MON receiving and schema-default validation must apply one exact-decimal scale policy. The
///      input borrows the normalized text, so scanning is allocation-free and copies nothing.
///
/// # Input contract
///
/// `normalized` must be the `normalized_text` produced by
/// [`parse_numeric_literal`](super::parse::parse_numeric_literal), or equivalent retained parser
/// facts. This low-level analysis does not validate grammar; unchecked text must pass the parser
/// first.
pub fn effective_decimal_scale(normalized: &str) -> usize {
    NormalizedDecimalFacts::analyze(normalized).effective_scale()
}

/// Borrowed facts shared by MON's exact-fit check and `NumberValue` coefficient materialization.
///
/// The normalized spelling has already passed the numeric grammar. This analysis only splits its
/// retained coefficient/exponent facts; it does not validate another numeric syntax.
#[derive(Debug, Clone, Copy)]
pub struct NormalizedDecimalFacts<'a> {
    integer_part: &'a str,
    fractional_part: &'a str,
    trailing_zeroes: usize,
    coefficient_is_zero: bool,
    exponent_magnitude: Option<usize>,
    negative_exponent: bool,
}

impl<'a> NormalizedDecimalFacts<'a> {
    /// Analyze a grammar-validated normalized spelling without allocation.
    ///
    /// `normalized` must be the unsigned, separator-free `normalized_text` produced by
    /// [`parse_numeric_literal`](super::parse::parse_numeric_literal), or equivalent retained
    /// parser facts. Its decimal point and lowercase exponent must follow that grammar.
    pub fn analyze(normalized: &'a str) -> Self {
        let (coefficient, exponent) = match normalized.split_once('e') {
            Some((coefficient, exponent)) => (coefficient, Some(exponent)),
            None => (normalized, None),
        };
        let (integer_part, fractional_part) = match coefficient.split_once('.') {
            Some((integer_part, fractional_part)) => (integer_part, fractional_part),
            None => (coefficient, ""),
        };

        // Scanning backward finds both the trailing-zero count and the zero fast path without
        // reading exponent digits or walking the whole coefficient after a nonzero tail.
        let mut trailing_zeroes = 0usize;
        let mut coefficient_is_zero = true;
        for digit in integer_part.bytes().chain(fractional_part.bytes()).rev() {
            if digit == b'0' {
                trailing_zeroes += 1;
            } else {
                coefficient_is_zero = false;
                break;
            }
        }

        // Signed zero ignores its exponent magnitude and sign.
        if coefficient_is_zero {
            return Self {
                integer_part,
                fractional_part,
                trailing_zeroes,
                coefficient_is_zero,
                exponent_magnitude: Some(0),
                negative_exponent: false,
            };
        }

        let (exponent_magnitude, negative_exponent) = match exponent {
            Some(exponent) => {
                let negative_exponent = exponent.starts_with('-');
                let digits = exponent
                    .strip_prefix('+')
                    .or_else(|| exponent.strip_prefix('-'))
                    .unwrap_or(exponent);
                (checked_decimal_usize(digits), negative_exponent)
            }
            None => (Some(0), false),
        };

        Self {
            integer_part,
            fractional_part,
            trailing_zeroes,
            coefficient_is_zero,
            exponent_magnitude,
            negative_exponent,
        }
    }

    /// Exact fractional scale after exponent adjustment and coefficient trailing-zero removal.
    pub fn effective_scale(&self) -> usize {
        if self.coefficient_is_zero {
            return 0;
        }

        let exponent_magnitude = self.exponent_magnitude.unwrap_or(usize::MAX);
        let untrimmed_scale = if self.negative_exponent {
            self.fractional_part
                .len()
                .saturating_add(exponent_magnitude)
        } else {
            self.fractional_part
                .len()
                .saturating_sub(exponent_magnitude)
        };
        let trailing_zeroes = self.trailing_zeroes.min(untrimmed_scale);
        untrimmed_scale.saturating_sub(trailing_zeroes)
    }

    /// Whether the coefficient contains only zero digits.
    pub fn coefficient_is_zero(&self) -> bool {
        self.coefficient_is_zero
    }

    /// Exponent magnitude, or `None` when its validated digits exceed host `usize` capacity.
    pub fn exponent_magnitude(&self) -> Option<usize> {
        self.exponent_magnitude
    }

    /// Whether the normalized exponent had a negative sign.
    pub fn negative_exponent(&self) -> bool {
        self.negative_exponent
    }

    /// Borrowed integer coefficient digits.
    pub fn integer_part(&self) -> &'a str {
        self.integer_part
    }

    /// Borrowed fractional coefficient digits.
    pub fn fractional_part(&self) -> &'a str {
        self.fractional_part
    }

    /// Number of zero digits at the end of the coefficient.
    pub fn trailing_zeroes(&self) -> usize {
        self.trailing_zeroes
    }
}

/// Parse a validated exponent magnitude once, returning `None` if it exceeds `usize`.
///
/// The normalized spelling has already been grammar-validated; overflow is retained as a missing
/// exact magnitude so MON can derive its saturated scale view without a second exponent scan.
fn checked_decimal_usize(digits: &str) -> Option<usize> {
    let mut value = 0usize;
    for byte in digits.bytes() {
        let digit = usize::from(byte.saturating_sub(b'0'));
        value = value.checked_mul(10)?.checked_add(digit)?;
    }
    Some(value)
}

#[cfg(test)]
#[path = "tests/decimal_tests.rs"]
mod tests;
