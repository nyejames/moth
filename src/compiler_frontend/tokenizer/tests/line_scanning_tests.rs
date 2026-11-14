//! Physical-line evidence used by scope-closing layout.
//!
//! WHAT: pins the lexer's token line-break flag and the token-line predicates that scope owners
//!       share.
//! WHY: an inline/multiline decision must be exact for every authored newline shape and must not
//!       rescan source text.

use super::*;
use crate::compiler_frontend::source::{ExtendedSpanBuilder, SourceId};
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::lexer::{LexedSource, tokenize};
use crate::compiler_frontend::tokenizer::tokens::{TokenRef, TokenizerEntryMode};
use crate::compiler_tests::test_support::frontend_test_style_directives;

fn tokenize_source(source: &str) -> LexedSource {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let source_path = path_fork
        .try_intern_portable_path("line-scanning.moth", &mut string_table)
        .expect("test path fits");
    let style_directives: StyleDirectiveRegistry = frontend_test_style_directives();
    let mut span_builder = ExtendedSpanBuilder::new();
    tokenize(
        source,
        source_path,
        TokenizerEntryMode::SourceFile,
        &style_directives,
        &mut string_table,
        &mut path_fork,
        SourceId::COMPILATION_ROOT,
        &mut span_builder,
    )
    .expect("test source should tokenize")
}

fn token_at(lexed: &LexedSource, index: usize) -> TokenRef<'_> {
    lexed
        .tokens
        .token(TokenIndex::try_from_index(index).expect("test token index fits"))
        .expect("test token index should resolve")
}

fn literal_index(lexed: &LexedSource) -> usize {
    (0..lexed.tokens.len())
        .find(|&index| {
            matches!(
                token_at(lexed, index).tag(),
                TokenTag::STRING_SLICE_LITERAL | TokenTag::RAW_STRING_LITERAL
            )
        })
        .expect("fixture should contain one literal token")
}

fn index_of(lexed: &LexedSource, tag: TokenTag, occurrence: usize) -> usize {
    (0..lexed.tokens.len())
        .filter(|&index| token_at(lexed, index).tag() == tag)
        .nth(occurrence)
        .expect("fixture should contain the requested token")
}

fn index_of_nth_end(lexed: &LexedSource, occurrence: usize) -> usize {
    index_of(lexed, TokenTag::END, occurrence)
}

fn token_index(index: usize) -> TokenIndex {
    TokenIndex::try_from_index(index).expect("test token index fits")
}

#[test]
fn quoted_string_flag_records_every_physical_newline_shape() {
    for (source, expected) in [
        ("\"a\nb\"", true),
        ("\"a\r\nb\"", true),
        ("\"a\rb\"", true),
        ("\"ab\"", false),
        ("\"a\\nb\"", false),
    ] {
        let lexed = tokenize_source(source);
        let literal = token_at(&lexed, literal_index(&lexed));
        assert_eq!(
            literal.shape().contains_line_break(),
            expected,
            "quoted literal flag for {source:?}"
        );
    }
}

#[test]
fn raw_string_flag_records_every_physical_newline_shape() {
    for (source, expected) in [
        ("`a\nb`", true),
        ("`a\r\nb`", true),
        ("`a\rb`", true),
        ("`ab`", false),
    ] {
        let lexed = tokenize_source(source);
        let literal = token_at(&lexed, literal_index(&lexed));
        assert_eq!(
            literal.shape().contains_line_break(),
            expected,
            "raw literal flag for {source:?}"
        );
    }
}

#[test]
fn template_body_flag_records_every_physical_newline_shape() {
    for (source, expected) in [
        ("[:a\nb]", true),
        ("[:a\r\nb]", true),
        ("[:a\rb]", true),
        ("[:ab]", false),
    ] {
        let lexed = tokenize_source(source);
        let literal = token_at(&lexed, literal_index(&lexed));
        assert_eq!(
            literal.shape().contains_line_break(),
            expected,
            "template-body literal flag for {source:?}"
        );
    }
}

#[test]
fn discarded_template_body_flag_records_every_physical_newline_shape() {
    // `$note` discards its body, so the closing `]` is the only token that can own the line
    // breaks the lexer consumed. Without the flag the body would look inline to every caller.
    for (source, expected) in [
        ("[$note: a\nb]", true),
        ("[$note: a\r\nb]", true),
        ("[$note: a\rb]", true),
        ("[$note: ab]", false),
    ] {
        let lexed = tokenize_source(source);
        let close = token_at(&lexed, index_of(&lexed, TokenTag::TEMPLATE_CLOSE, 0));
        assert_eq!(
            close.shape().contains_line_break(),
            expected,
            "discarded template close flag for {source:?}"
        );
    }
}

#[test]
fn line_break_walk_uses_newline_tokens_and_multibyte_text() {
    let lexed = tokenize_source("alpha café\n\ngamma\r\ndelta\r-- only a comment\nomega\n");
    let tokens = lexed.tokens.as_ref();
    let alpha = index_of(&lexed, TokenTag::SYMBOL, 0);
    let multibyte = index_of(&lexed, TokenTag::SYMBOL, 1);
    let gamma = index_of(&lexed, TokenTag::SYMBOL, 2);
    let delta = index_of(&lexed, TokenTag::SYMBOL, 3);
    let omega = index_of(&lexed, TokenTag::SYMBOL, 4);

    assert!(!has_physical_line_break_between(
        tokens,
        token_index(alpha),
        token_index(multibyte)
    ));
    assert!(has_physical_line_break_between(
        tokens,
        token_index(alpha),
        token_index(gamma)
    ));
    assert!(has_physical_line_break_between(
        tokens,
        token_index(multibyte),
        token_index(gamma)
    ));
    assert!(has_physical_line_break_between(
        tokens,
        token_index(gamma),
        token_index(delta)
    ));
    assert!(
        has_physical_line_break_between(tokens, token_index(delta), token_index(omega)),
        "a comment-only line still contributes its authored line break"
    );
}

#[test]
fn line_break_walk_stops_at_a_multiline_literal() {
    let lexed = tokenize_source("call(1, \"a\nb\");\n");
    let tokens = lexed.tokens.as_ref();
    let call = index_of(&lexed, TokenTag::SYMBOL, 0);
    let closer = index_of_nth_end(&lexed, 0);

    assert!(
        has_physical_line_break_between(tokens, token_index(call), token_index(closer)),
        "a multiline string literal inside the call crosses a physical line"
    );
}

#[test]
fn closer_placement_classifies_inline_standalone_additional_and_after_code() {
    let inline = tokenize_source("head: body;\n");
    let inline_tokens = inline.tokens.as_ref();
    assert_eq!(
        closer_placement(
            inline_tokens,
            token_index(index_of(&inline, TokenTag::COLON, 0)),
            token_index(index_of_nth_end(&inline, 0)),
        ),
        Some(CloserPlacement::Inline)
    );

    let standalone = tokenize_source("head: body\n;\n");
    let standalone_tokens = standalone.tokens.as_ref();
    assert_eq!(
        closer_placement(
            standalone_tokens,
            token_index(index_of(&standalone, TokenTag::COLON, 0)),
            token_index(index_of_nth_end(&standalone, 0)),
        ),
        Some(CloserPlacement::Standalone)
    );

    let additional = tokenize_source("head: body;;\n");
    let additional_tokens = additional.tokens.as_ref();
    let first_closer = index_of_nth_end(&additional, 0);
    let second_closer = index_of_nth_end(&additional, 1);
    let placement = closer_placement(
        additional_tokens,
        token_index(index_of(&additional, TokenTag::COLON, 0)),
        token_index(second_closer),
    );
    assert_eq!(
        placement,
        Some(CloserPlacement::AdditionalCloser {
            previous_closer: token_at(&additional, first_closer).source_span(),
        })
    );

    let after_code = tokenize_source("head: body\nnext;\n");
    let after_code_tokens = after_code.tokens.as_ref();
    assert_eq!(
        closer_placement(
            after_code_tokens,
            token_index(index_of(&after_code, TokenTag::COLON, 0)),
            token_index(index_of_nth_end(&after_code, 0)),
        ),
        Some(CloserPlacement::AfterCode)
    );
}

#[test]
fn closer_trailing_classifies_line_end_additional_and_content() {
    let line_end = tokenize_source("head: body; -- trailing comment\n");
    assert_eq!(
        closer_trailing(
            line_end.tokens.as_ref(),
            token_index(index_of_nth_end(&line_end, 0))
        ),
        CloserTrailing::LineEnd
    );

    let crlf_line_end = tokenize_source("head: body;\r\nnext\r\n");
    assert_eq!(
        closer_trailing(
            crlf_line_end.tokens.as_ref(),
            token_index(index_of_nth_end(&crlf_line_end, 0))
        ),
        CloserTrailing::LineEnd
    );

    let additional = tokenize_source("head: body;;\n");
    let second_closer = index_of_nth_end(&additional, 1);
    assert_eq!(
        closer_trailing(
            additional.tokens.as_ref(),
            token_index(index_of_nth_end(&additional, 0))
        ),
        CloserTrailing::AdditionalCloser {
            closer: token_at(&additional, second_closer).source_span(),
        }
    );

    let content = tokenize_source("head: body; next\n");
    let next = index_of(&content, TokenTag::SYMBOL, 2);
    assert_eq!(
        closer_trailing(
            content.tokens.as_ref(),
            token_index(index_of_nth_end(&content, 0))
        ),
        CloserTrailing::Content {
            first: token_at(&content, next).source_span(),
        }
    );
}
