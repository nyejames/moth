//! Self-tests for the code/non-code mask.
//!
//! Each case is a shape that carries text resembling code. The mask exists so a source-reading
//! gate can tell "this file does the banned thing" apart from "this file talks about the banned
//! thing", and these prove it draws that line where it claims to.

use super::{code_mask, is_identifier_character, matches_at};

/// The characters of `source` the mask calls code, as a string.
fn code_of(source: &str) -> String {
    let characters: Vec<char> = source.chars().collect();
    let mask = code_mask(&characters);

    characters
        .iter()
        .zip(mask)
        .filter_map(|(character, is_code)| is_code.then_some(*character))
        .collect()
}

#[test]
fn plain_code_is_all_code() {
    assert_eq!(code_of("let value = 1;"), "let value = 1;");
}

#[test]
fn a_line_comment_is_not_code() {
    assert_eq!(
        code_of("let a = 1; // let b = 2;\nlet c = 3;"),
        "let a = 1; \nlet c = 3;"
    );
}

#[test]
fn a_nested_block_comment_is_not_code_through_its_inner_close() {
    assert_eq!(code_of("a/* outer /* inner */ still */b"), "ab");
}

#[test]
fn a_string_literal_is_not_code() {
    assert_eq!(code_of(r#"call("let x = 1;")"#), "call()");
}

#[test]
fn a_raw_string_is_not_code_and_its_quotes_do_not_end_it() {
    assert_eq!(code_of(r##"call(r#"a "quoted" b"#)"##), "call()");
}

/// A byte string's text is not code; the `b` prefix in front of it still is.
///
/// The mask exists to keep literal text from being read as code, not to erase the tokens around
/// it, so the prefix stays where a consumer scanning for identifiers would expect it.
#[test]
fn a_byte_string_literal_is_not_code_but_its_prefix_is() {
    assert_eq!(code_of(r#"write(b"bytes")"#), "write(b)");
}

#[test]
fn an_escaped_quote_does_not_end_a_string() {
    assert_eq!(code_of(r#"a("x\"y")b"#), "a()b");
}

#[test]
fn a_character_literal_is_not_code() {
    assert_eq!(code_of("matches(c, '\"')"), "matches(c, )");
}

/// A lifetime is not a character literal, so the code after it must stay code.
#[test]
fn a_lifetime_does_not_open_a_character_literal() {
    assert_eq!(
        code_of("fn f<'a>(x: &'a str) {}"),
        "fn f<'a>(x: &'a str) {}"
    );
}

/// `r` and `b` only open a literal when they are not continuing an identifier.
#[test]
fn an_identifier_ending_in_r_does_not_open_a_raw_string() {
    assert_eq!(code_of(r##"for x in y {}"##), "for x in y {}");
}

#[test]
fn matches_at_compares_from_the_given_index() {
    let characters: Vec<char> = "abcdef".chars().collect();

    assert!(matches_at(&characters, 2, "cde"));
    assert!(!matches_at(&characters, 2, "cdx"));
    assert!(
        !matches_at(&characters, 4, "efg"),
        "a needle running past the end cannot match"
    );
}

#[test]
fn identifier_characters_are_alphanumeric_or_underscore() {
    assert!(is_identifier_character('a'));
    assert!(is_identifier_character('9'));
    assert!(is_identifier_character('_'));
    assert!(!is_identifier_character('-'));
    assert!(!is_identifier_character('('));
}
