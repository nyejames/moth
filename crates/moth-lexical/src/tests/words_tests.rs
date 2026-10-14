//! Tests for the shared exact source-word inventory.

use super::{SourceWord, SourceWordClass, classify_source_word};

#[test]
fn non_word_spellings_stay_unclassified() {
    for non_word in [
        "",
        "import",
        "Import",
        "Copy",
        "RETURN",
        "TRUE",
        "Int",
        "Uint",
        "Float",
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
