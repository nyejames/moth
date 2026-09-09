//! Numeric literal tokenization.
//!
//! WHAT: consumes numeric literal text and delegates grammar validation to the shared
//!       `numeric_text` module.
//! WHY: keeping the grammar in one place makes separator, exponent, and sign rules
//!      consistent between source literals and future string casts.

use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
use crate::compiler_frontend::compiler_messages::NumberLiteralErrorReason;
use crate::compiler_frontend::numeric_text::parse::parse_numeric_literal;
use crate::compiler_frontend::numeric_text::token::{NumericLiteralSign, NumericLiteralToken};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::lexer::TokenizeResult;
use crate::compiler_frontend::tokenizer::tokens::{Token, TokenKind, TokenStream};
use crate::return_token;

/// Tokenize an integer or float literal starting with `first_digit`.
///
/// WHAT: consumes the full literal run, then asks `numeric_text` to validate it and
///       build the lexical token payload.
/// WHY: the tokenizer owns source-location tracking, while `numeric_text` owns the
///      grammar; this boundary avoids leaking stream state into the grammar module.
pub(super) fn tokenize_numeric_literal(
    first_digit: char,
    stream: &mut TokenStream<'_>,
    string_table: &mut StringTable,
    sign: NumericLiteralSign,
) -> TokenizeResult<Token> {
    let mut literal_text = String::new();
    literal_text.push(first_digit);

    // ------------------------
    //  Consume integer part
    // ------------------------
    consume_digit_run(&mut literal_text, stream);

    // ------------------------
    //  Consume decimal part
    // ------------------------
    if stream.peek() == Some(&'.') {
        let decimal_point =
            stream.advance_after_peek("Tokenizer peeked a decimal point but could not advance.");
        literal_text.push(decimal_point);
        let fractional_digits = consume_digit_run(&mut literal_text, stream);

        if fractional_digits > 0 && stream.peek() == Some(&'.') {
            // Report the authored source text so diagnostics preserve underscores and sign.
            let authored = authored_numeric_text(sign, &literal_text);
            let authored_id = string_table.intern(&authored);
            return Err(CompilerDiagnostic::invalid_number_literal(
                authored_id,
                NumberLiteralErrorReason::MultipleDecimalPoints,
                Some(stream.current_source_span()?),
            )
            .into());
        }
    }

    // ------------------------
    //  Consume exponent part
    // ------------------------
    if let Some(&next_char) = stream.peek()
        && (next_char == 'e' || next_char == 'E')
    {
        let exponent_marker =
            stream.advance_after_peek("Tokenizer peeked an exponent marker but could not advance.");
        literal_text.push(exponent_marker);

        if let Some(&exponent_sign_char) = stream.peek()
            && (exponent_sign_char == '+' || exponent_sign_char == '-')
        {
            let exponent_sign = stream
                .advance_after_peek("Tokenizer peeked an exponent sign but could not advance.");
            literal_text.push(exponent_sign);
        }

        consume_digit_run(&mut literal_text, stream);
    }

    // ------------------------
    //  Validate and build token
    // ------------------------
    let authored = authored_numeric_text(sign, &literal_text);

    match parse_numeric_literal(&literal_text) {
        Ok(parsed) => {
            // `source_text` preserves the authored form (separators, uppercase E);
            // `normalized_text` is unsigned, separator-free, lowercase `e`.
            let source_text = string_table.intern(&authored);
            let normalized_text = string_table.intern(&parsed.normalized_text);
            let token = NumericLiteralToken::new(
                sign,
                source_text,
                normalized_text,
                parsed.kind,
                parsed.digit_count,
                parsed.fractional_digit_count,
                parsed.exponent_digit_count,
                parsed.exponent_sign,
            );
            return_token!(TokenKind::NumericLiteral(token), stream);
        }

        Err(reason) => {
            // Report the authored source text so diagnostics preserve underscores and sign.
            let authored_id = string_table.intern(&authored);
            Err(CompilerDiagnostic::invalid_number_literal(
                authored_id,
                reason,
                Some(stream.current_source_span()?),
            )
            .into())
        }
    }
}

/// Build the authored source text for a numeric literal, including the attached sign.
///
/// WHAT: prepends `-` for negative literals and returns the unsigned text as-is for
///       positive literals. This preserves the exact form the author typed for diagnostics.
/// WHY: `source_text` should match the author's source so range errors, uppercase-exponent
///      rejections, and separator diagnostics show the original literal, not a normalized form.
fn authored_numeric_text(sign: NumericLiteralSign, unsigned_literal_text: &str) -> String {
    match sign {
        NumericLiteralSign::Positive => unsigned_literal_text.to_owned(),
        NumericLiteralSign::Negative => format!("-{unsigned_literal_text}"),
    }
}

/// Consume a run of digits and digit separators from the stream.
///
/// WHAT: advances the stream past all consecutive digits and `_` separators,
///       appending each to `literal_text`. Returns the count of actual digits
///       consumed (excluding separators).
/// WHY: the scanner needs to know how far the literal extends before the grammar
///      validates separator placement; this helper centralizes that simple loop.
///      The digit count lets callers detect empty fractional parts or exponents.
fn consume_digit_run(literal_text: &mut String, stream: &mut TokenStream<'_>) -> u32 {
    let mut digit_count = 0;

    while let Some(&character) = stream.peek() {
        if character.is_ascii_digit() || character == '_' {
            let digit_or_separator = stream
                .advance_after_peek("Tokenizer peeked a digit or separator but could not advance.");
            if digit_or_separator.is_ascii_digit() {
                digit_count += 1;
            }
            literal_text.push(digit_or_separator);
        } else {
            break;
        }
    }

    digit_count
}
