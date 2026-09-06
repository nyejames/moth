//! String and template-body tokenization.
//!
//! WHAT: owns lexing for raw strings, quoted strings, and template body text
//! modes that treat most source characters as string content.
//! WHY: these modes are delimiter-state machines rather than ordinary token
//! dispatch, so keeping them outside `lexer.rs` makes the main lexer easier to
//! audit.

use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, InvalidStringEscapeReason};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::newline_handling::{
    consume_pending_carriage_return_newline, normalize_consumed_carriage_return_newline,
};
use crate::compiler_frontend::tokenizer::tokens::{
    CharPosition, SourceLocation, Token, TokenKind, TokenStream,
};
use crate::return_token;

/// Boxed diagnostic result shared by every text-mode function in this file.
///
/// WHAT: one file-local alias for the boxed `CompilerDiagnostic` error variant returned by
/// `tokenize_raw_string`, `tokenize_string`, `tokenize_template_body`,
/// `tokenize_code_template_body` and `tokenize_discard_template_body`.
/// WHY: every text mode returns directly into the lexer's boxed dispatch family. Sharing that
/// error shape keeps mode transitions direct and leaves one owner for string diagnostics.
type TextModeResult<T> = Result<T, Box<CompilerDiagnostic>>;

pub(super) fn tokenize_raw_string(
    stream: &mut TokenStream<'_>,
    string_table: &mut StringTable,
) -> TextModeResult<Token> {
    let mut token_value = String::new();

    while let Some(ch) = stream.next() {
        if ch == '`' {
            let interned_string = string_table.intern(&token_value);
            return_token!(TokenKind::RawStringLiteral(interned_string), stream);
        }

        if ch == '\r' {
            let normalized_char = normalize_consumed_carriage_return_newline(stream);
            token_value.push_str(normalized_char);
            continue;
        }

        token_value.push(ch);
    }

    Err(Box::new(CompilerDiagnostic::unterminated_string_literal(
        stream.new_location(),
    )))
}

/// WHAT: lexes a double-quoted string slice, decoding only the supported escapes.
/// WHY: quoted strings implement one defined escape grammar. Unsupported escapes, physical
/// line continuation and trailing backslashes are rejected with typed diagnostics so the
/// span points at the actual mistake rather than the whole literal.
pub(super) fn tokenize_string(
    stream: &mut TokenStream<'_>,
    string_table: &mut StringTable,
) -> TextModeResult<Token> {
    let mut token_value = String::new();

    loop {
        // Capture the position of the next source character before consuming it so escape
        // diagnostics can span the backslash and its escape body precisely.
        let span_start = stream.position;

        let Some(ch) = stream.next() else {
            return Err(Box::new(CompilerDiagnostic::unterminated_string_literal(
                stream.new_location(),
            )));
        };

        if ch == '\\' {
            // The backslash has been consumed, so the stream now points at the escaped character.
            let after_backslash = stream.position;

            let Some(&escaped_char) = stream.peek() else {
                // A backslash at end of source never received an escaped character.
                return Err(Box::new(CompilerDiagnostic::invalid_string_escape(
                    InvalidStringEscapeReason::TrailingBackslash,
                    escape_span(stream, span_start, after_backslash),
                )));
            };

            // A physical newline after a backslash is a line-continuation attempt, not a
            // supported escape. LF and CRLF continuation are the same source mistake.
            if escaped_char == '\n' || escaped_char == '\r' {
                return Err(Box::new(CompilerDiagnostic::invalid_string_escape(
                    InvalidStringEscapeReason::PhysicalNewline,
                    escape_span(stream, span_start, after_backslash),
                )));
            }

            // Consume the escaped character so the unsupported-escape span covers both chars.
            let escaped_char = stream.advance_after_peek(
                "Tokenizer peeked an escaped string character but could not advance the stream.",
            );

            match escaped_char {
                '\\' => token_value.push('\\'),
                '"' => token_value.push('"'),
                'n' => token_value.push('\n'),
                'r' => token_value.push('\r'),
                't' => token_value.push('\t'),
                _ => {
                    return Err(Box::new(CompilerDiagnostic::invalid_string_escape(
                        InvalidStringEscapeReason::UnsupportedEscape {
                            escaped: escaped_char,
                        },
                        escape_span(stream, span_start, stream.position),
                    )));
                }
            }

            continue;
        }

        if ch == '"' {
            let interned_string = string_table.intern(&token_value);
            return_token!(TokenKind::StringSliceLiteral(interned_string), stream);
        }

        if ch == '\r' {
            let normalized_char = normalize_consumed_carriage_return_newline(stream);
            token_value.push_str(normalized_char);
            continue;
        }

        token_value.push(ch);
    }
}

/// WHAT: builds a source span for an invalid-escape diagnostic.
/// WHY: the stream cursor is exclusive while `SourceLocation` ends are inclusive. Converting in
/// one place keeps one-character and two-character escape underlines exact.
fn escape_span(stream: &TokenStream<'_>, start: CharPosition, end: CharPosition) -> SourceLocation {
    let inclusive_end = CharPosition {
        char_column: end.char_column.saturating_sub(1),
        ..end
    };

    SourceLocation::new(stream.file_path.to_owned(), start, inclusive_end)
}

pub(super) fn tokenize_template_body(
    current_char: char,
    stream: &mut TokenStream<'_>,
    string_table: &mut StringTable,
) -> TextModeResult<Token> {
    let mut token_value = String::new();
    append_template_body_char(current_char, &mut token_value, stream);

    while let Some(ch) = stream.peek() {
        match ch {
            '[' | ']' => {
                let interned_string = string_table.intern(&token_value);
                return_token!(TokenKind::StringSliceLiteral(interned_string), stream);
            }

            '\r' => {
                let normalized_char = consume_pending_carriage_return_newline(stream);
                token_value.push_str(normalized_char);
            }

            _ => {
                let next_char = stream.advance_after_peek(
                    "Tokenizer peeked a template-body character but could not advance the stream.",
                );
                token_value.push(next_char);
            }
        }
    }

    let interned_string = string_table.intern(&token_value);
    return_token!(TokenKind::StringSliceLiteral(interned_string), stream);
}

fn append_template_body_char(
    current_char: char,
    token_value: &mut String,
    stream: &mut TokenStream<'_>,
) {
    match current_char {
        '\r' => token_value.push_str(normalize_consumed_carriage_return_newline(stream)),
        _ => token_value.push(current_char),
    }
}

pub(super) fn tokenize_code_template_body(
    current_char: char,
    stream: &mut TokenStream<'_>,
    string_table: &mut StringTable,
) -> TextModeResult<Token> {
    // `$code` template bodies treat square brackets as literal code characters.
    // The template only closes when the running bracket counts become balanced.
    if current_char == ']' && stream.template_body_next_close_balances_brackets() {
        stream.register_template_body_close_square_bracket();
        stream.pop_template_mode();
        return_token!(TokenKind::TemplateClose, stream);
    }

    let mut token_value = String::new();
    append_code_template_body_char(current_char, &mut token_value, stream);

    while let Some(&ch) = stream.peek() {
        if ch == ']' && stream.template_body_next_close_balances_brackets() {
            break;
        }

        let next_char = stream.advance_after_peek(
            "Tokenizer peeked a code-template body character but could not advance the stream.",
        );

        append_code_template_body_char(next_char, &mut token_value, stream);
    }

    let interned_string = string_table.intern(&token_value);
    return_token!(TokenKind::StringSliceLiteral(interned_string), stream);
}

/// Consume a discarded template body, emitting only its closing bracket.
///
/// The body text is skipped rather than tokenized, so the emitted `TemplateClose` and the
/// unterminated-body `Eof` are re-anchored: their spans denote the bracket and the insertion
/// point, never the discarded run that preceded them.
pub(super) fn tokenize_discard_template_body(
    current_char: char,
    stream: &mut TokenStream<'_>,
) -> TextModeResult<Token> {
    match current_char {
        '[' => stream.register_template_body_open_square_bracket(),
        ']' => {
            if stream.template_body_next_close_balances_brackets() {
                stream.register_template_body_close_square_bracket();
                stream.pop_template_mode();
                return_token!(TokenKind::TemplateClose, stream);
            }
            stream.register_template_body_close_square_bracket();
        }
        _ => {}
    }

    while let Some(&ch) = stream.peek() {
        match ch {
            '[' => {
                stream.next();
                stream.register_template_body_open_square_bracket();
            }
            ']' => {
                if stream.template_body_next_close_balances_brackets() {
                    stream.next();
                    stream.begin_token_bytes_at_consumed_char();
                    stream.register_template_body_close_square_bracket();
                    stream.pop_template_mode();
                    return_token!(TokenKind::TemplateClose, stream);
                }
                stream.next();
                stream.register_template_body_close_square_bracket();
            }
            _ => {
                stream.next();
            }
        }
    }

    stream.begin_token_bytes_at_cursor();
    return_token!(TokenKind::Eof, stream)
}

fn append_code_template_body_char(
    ch: char,
    token_value: &mut String,
    stream: &mut TokenStream<'_>,
) {
    match ch {
        '[' => stream.register_template_body_open_square_bracket(),
        ']' => stream.register_template_body_close_square_bracket(),
        '\r' => {
            let normalized_char = normalize_consumed_carriage_return_newline(stream);
            token_value.push_str(normalized_char);
            return;
        }
        _ => {}
    }

    token_value.push(ch);
}
