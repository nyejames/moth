//! Tests for the shared exact source-word inventory.

use super::{RESERVED_WORD_SPELLINGS, SourceWord, SourceWordClass, classify_source_word};
use std::collections::BTreeSet;

#[test]
fn every_inventory_identity_round_trips_and_non_words_stay_unclassified() {
    for word in SourceWord::ALL {
        assert_eq!(classify_source_word(word.spelling()), Some(*word));
    }

    for non_word in [
        "",
        "import",
        "Import",
        "Copy",
        "RETURN",
        "TRUE",
        "int",
        "fn",
        "config",
        "return!",
        "block",
        "_block",
        "block_value",
        "in",
        "group",
        "into",
        "where",
        "u8",
        "i64",
        "f32",
        "byte",
        "I128",
        "U08",
        "Bytes",
        "U8v2",
    ] {
        assert_eq!(
            classify_source_word(non_word),
            None,
            "{non_word:?} is not an exact word"
        );
    }
}

#[test]
fn exact_word_identity_preserves_case_and_literal_type_distinctions() {
    assert_eq!(classify_source_word("this"), Some(SourceWord::This));
    assert_eq!(classify_source_word("This"), Some(SourceWord::ThisType));
    assert_ne!(classify_source_word("this"), classify_source_word("This"));

    let true_literal = classify_source_word("true").expect("true is a literal word");
    let true_type = classify_source_word("True").expect("True is a builtin type word");
    assert_eq!(true_literal.class(), SourceWordClass::Literal);
    assert_eq!(true_literal.bool_literal_value(), Some(true));
    assert_eq!(true_type.class(), SourceWordClass::BuiltinType);
    assert_eq!(true_type.bool_literal_value(), None);
}

#[test]
fn reservation_inventory_is_the_case_folded_exact_word_union() {
    let actual = RESERVED_WORD_SPELLINGS
        .iter()
        .map(|spelling| spelling.to_ascii_lowercase())
        .collect::<BTreeSet<_>>();
    let expected = [
        "and", "as", "assert", "async", "bool", "break", "by", "byte", "cast", "catch", "char",
        "checked", "config", "continue", "copy", "else", "export", "f16", "f32", "f64", "false",
        "float", "fn", "i16", "i32", "i64", "i8", "if", "int", "is", "loop", "must", "none", "not",
        "of", "or", "return", "string", "then", "this", "to", "true", "type", "u16", "u32", "u64",
        "u8", "yield",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<BTreeSet<_>>();

    assert_eq!(actual, expected);
}
