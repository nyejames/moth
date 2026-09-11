//! Newline normalization policy for tokenizer string/template bodies.
//!
//! WHAT: normalizes `\r` and `\r\n` into stable `\n` token text payloads.
//!
//! WHY these only consume: spans are byte-anchored; line and column come from
//! `source::line_index`, not from `TokenStream::next`. A helper that adjusted position too would
//! duplicate source mapping. What is left here is the token text policy plus consuming the second
//! half of a `\r\n` pair.

use crate::compiler_frontend::tokenizer::tokens::TokenStream;

/// Consume a newline that started with a `\r` still pending in the stream.
///
/// Handles both `\r\n` and a bare `\r`, and returns the canonical newline string emitted into
/// token text. If the caller already consumed the `\r`, use
/// [`normalize_consumed_carriage_return_newline`] instead.
pub fn consume_pending_carriage_return_newline(stream: &mut TokenStream) -> &'static str {
    stream.next();

    normalize_consumed_carriage_return_newline(stream)
}

/// Normalize a newline that started with a `\r` already consumed by the caller.
///
/// This variant is for tokenization loops that read chars with `stream.next()` first,
/// then branch on `'\r'`.
pub fn normalize_consumed_carriage_return_newline(stream: &mut TokenStream) -> &'static str {
    // A following `\n` completes one break with the `\r`, which already counted it.
    if matches!(stream.chars.peek(), Some('\n')) {
        stream.next();
    }

    "\n"
}
