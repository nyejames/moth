//! Newline normalization policy for tokenizer string/template bodies.
//!
//! WHAT: normalizes `\r` and `\r\n` into stable `\n` token text payloads.

use crate::compiler_frontend::tokenizer::tokens::TokenStream;

/// Consume a newline that started with `\r`.
///
/// Automatically handles both `\r\n` and bare `\r` newlines,
/// and advances the stream position accordingly.
/// Returns the canonical newline string emitted into token text.
///
/// IMPORTANT:
/// - This variant is for call sites where `\r` is still pending in the stream.
/// - If the caller already consumed `\r` via `stream.next()`, use
///   `normalize_consumed_carriage_return_newline` instead.
pub fn consume_pending_carriage_return_newline(stream: &mut TokenStream) -> &'static str {
    // Consume the \r without advancing the column; the byte cursor still moves.
    stream.consume_char_without_column_advance();

    let has_following_lf = matches!(stream.chars.peek(), Some('\n'));

    if has_following_lf {
        stream.next(); // consume the '\n' in a CRLF pair (also advanced the line)
    } else {
        // Advance the line number for a bare \r, but don't consume any more chars.
        // The byte cursor already includes this `\r`.
        stream.position.line_number += 1;
        stream.position.char_column = 0;
    }

    "\n"
}

/// Normalize a newline that started with a `\r` already consumed by the caller.
///
/// This variant is for tokenization loops that read chars with `stream.next()` first,
/// then branch on `'\r'`.
pub fn normalize_consumed_carriage_return_newline(stream: &mut TokenStream) -> &'static str {
    let has_following_lf = matches!(stream.chars.peek(), Some('\n'));

    if has_following_lf {
        stream.next(); // consume the '\n' in a CRLF pair (also advances the line)
    } else {
        // Caller already consumed '\r' as one character, so finalize newline position now.
        // The byte cursor already includes this `\r`.
        stream.position.line_number += 1;
        stream.position.char_column = 0;
    }

    "\n"
}
