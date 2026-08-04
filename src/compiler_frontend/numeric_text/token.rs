//! Numeric literal token payload.
//!
//! WHAT: carries the lexical metadata extracted from a numeric literal without
//!       committing to a runtime representation such as `i32` or `f64`.
//! WHY: separating lexical classification from semantic materialization lets the
//!      tokenizer own source-shape facts while AST/config consumers decide how to
//!      interpret them.

use crate::compiler_frontend::symbols::string_interning::{StringId, StringIdRemap};

/// Lexical classification of a numeric literal token.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NumericLiteralKind {
    /// A whole-number literal such as `42` or `1_000`.
    WholeNumber,
    /// A decimal-point literal such as `3.14`.
    DecimalPoint,
    /// An exponent literal such as `1e6` or `1.0e-21`.
    Exponent,
}

/// Sign attached to a numeric literal token.
///
/// WHY: the tokenizer front-loads attached `-` for numeric literals, while normalized text stays
/// unsigned so materialization can apply range checks with the sign as explicit metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NumericLiteralSign {
    /// The literal is positive (no leading `-`).
    Positive,
    /// The literal has an attached leading `-` sign.
    ///
    /// WHY: the tokenizer front-loads the `-` operator into the numeric token so
    ///      materialization can apply range checks with the sign as explicit metadata.
    Negative,
}

/// Sign explicitly written on an exponent, if any.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NumericExponentSign {
    /// No explicit sign marker on the exponent (e.g. `1e6`).
    None,
    /// Explicit `+` after the exponent marker (e.g. `1e+21`).
    ///
    /// WHY: the normalized text preserves explicit exponent signs so materialization
    ///      can reconstruct the value from text alone.
    Positive,
    /// Explicit `-` after the exponent marker (e.g. `1e-21`).
    Negative,
}

/// The tokenizer's numeric-literal payload.
///
/// `source_text` preserves the authored source text (including underscores,
/// attached sign, and uppercase `E`) so diagnostics can report exactly what the
/// author wrote. `normalized_text` is unsigned, underscore-free, and uses
/// lowercase `e` for exponents; it preserves explicit exponent signs (`1e+21`)
/// so that materialization can reconstruct the value from text alone.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct NumericLiteralToken {
    /// Whether the literal carries an attached leading `-`.
    pub sign: NumericLiteralSign,
    /// Authored source text, including sign and original formatting.
    /// Used by diagnostics so the reported literal matches what the author typed.
    pub source_text: StringId,
    /// Unsigned, separator-free, lowercase-exponent text for materialization.
    pub normalized_text: StringId,
    /// Lexical shape of the literal: whole-number, decimal-point, or exponent.
    pub kind: NumericLiteralKind,
    /// Total number of significant digits in the integer part.
    ///
    /// WHY: digit counts let materialization and diagnostics reason about
    ///      precision and range without re-parsing the source text.
    pub digit_count: u32,
    /// Number of digits after the decimal point, if any.
    pub fractional_digit_count: u32,
    /// Number of digits in the exponent part, if any.
    pub exponent_digit_count: u32,
    /// Explicit sign on the exponent, if the literal has one.
    pub exponent_sign: NumericExponentSign,
}

impl NumericLiteralToken {
    /// Construct a numeric literal token with both source and normalized text.
    ///
    /// WHY: the constructor intentionally takes all lexical fields as arguments
    ///      so callers cannot accidentally omit `source_text` or `normalized_text`.
    ///      The argument count is stable because the token shape is finalized.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        sign: NumericLiteralSign,
        source_text: StringId,
        normalized_text: StringId,
        kind: NumericLiteralKind,
        digit_count: u32,
        fractional_digit_count: u32,
        exponent_digit_count: u32,
        exponent_sign: NumericExponentSign,
    ) -> Self {
        Self {
            sign,
            source_text,
            normalized_text,
            kind,
            digit_count,
            fractional_digit_count,
            exponent_digit_count,
            exponent_sign,
        }
    }

    /// Remap the interned source and normalized text after a string-table merge.
    ///
    /// WHAT: updates both `source_text` and `normalized_text` so diagnostic
    ///       reporting and materialization remain valid after per-file tables
    ///       merge into the module table.
    pub fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        self.source_text = remap.get(self.source_text);
        self.normalized_text = remap.get(self.normalized_text);
    }

    /// Remap both interned text payloads through one fallible string-ID mapping.
    pub fn try_map_string_ids<E>(
        &self,
        map: &mut impl FnMut(StringId) -> Result<StringId, E>,
    ) -> Result<Self, E> {
        Ok(Self::new(
            self.sign,
            map(self.source_text)?,
            map(self.normalized_text)?,
            self.kind,
            self.digit_count,
            self.fractional_digit_count,
            self.exponent_digit_count,
            self.exponent_sign,
        ))
    }

    /// Build a test numeric token from a valid source snippet.
    ///
    /// WHY: unit tests across the frontend need a concise way to construct
    ///      `TokenKind::NumericLiteral` payloads without hand-assembling digit
    ///      counts or re-parsing the source. For positive literals, `source_text`
    ///      and `normalized_text` differ only when the source contains separators
    ///      or uppercase exponents.
    #[cfg(test)]
    pub fn test_new(
        source: &str,
        string_table: &mut crate::compiler_frontend::symbols::string_interning::StringTable,
    ) -> Self {
        use crate::compiler_frontend::numeric_text::parse::parse_numeric_literal;

        let parsed = parse_numeric_literal(source).unwrap_or_else(|reason| {
            panic!("test numeric literal '{source}' is invalid: {reason:?}")
        });
        let source_text = string_table.intern(source);
        let normalized_text = string_table.intern(&parsed.normalized_text);

        Self::new(
            NumericLiteralSign::Positive,
            source_text,
            normalized_text,
            parsed.kind,
            parsed.digit_count,
            parsed.fractional_digit_count,
            parsed.exponent_digit_count,
            parsed.exponent_sign,
        )
    }
}
