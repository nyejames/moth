//! Compiler token mapping for exact source spellings.
//!
//! WHAT: drives authored spellings through the shared classifier and the compiler token mapping.
//! WHY: the expected table is written independently of the inventory, so swapping two inventory
//!      spellings or token mappings fails here.

use crate::compiler_frontend::keywords::{
    attached_bang_keyword_token_tag, token_tag_for_source_word,
};
use crate::compiler_frontend::tokenizer::tokens::TokenTag;
use moth_lexical::words::{SourceWord, SourceWordClass, classify_source_word};

#[test]
fn each_source_spelling_maps_to_its_exact_compiler_token() {
    use SourceWordClass::{BuiltinType, Keyword, Literal, WordOperator};

    let expected: &[(&str, TokenTag, SourceWordClass, Option<bool>)] = &[
        ("export", TokenTag::EXPORT, Keyword, None),
        ("type", TokenTag::TYPE, Keyword, None),
        ("of", TokenTag::OF, Keyword, None),
        ("as", TokenTag::AS, Keyword, None),
        ("copy", TokenTag::COPY, Keyword, None),
        ("if", TokenTag::IF, Keyword, None),
        ("return", TokenTag::RETURN, Keyword, None),
        ("catch", TokenTag::CATCH, Keyword, None),
        ("then", TokenTag::THEN, Keyword, None),
        ("else", TokenTag::ELSE, Keyword, None),
        ("checked", TokenTag::CHECKED, Keyword, None),
        ("cast", TokenTag::CAST, Keyword, None),
        ("break", TokenTag::BREAK, Keyword, None),
        ("continue", TokenTag::CONTINUE, Keyword, None),
        ("must", TokenTag::MUST, Keyword, None),
        ("this", TokenTag::THIS, Keyword, None),
        ("This", TokenTag::TRAIT_THIS, Keyword, None),
        ("assert", TokenTag::ASSERT, Keyword, None),
        ("async", TokenTag::ASYNC, Keyword, None),
        ("yield", TokenTag::YIELD, Keyword, None),
        ("loop", TokenTag::LOOP, Keyword, None),
        ("to", TokenTag::EXCLUSIVE_RANGE, Keyword, None),
        ("by", TokenTag::BY, Keyword, None),
        ("is", TokenTag::IS, WordOperator, None),
        ("not", TokenTag::NOT, WordOperator, None),
        ("and", TokenTag::AND, WordOperator, None),
        ("or", TokenTag::OR, WordOperator, None),
        ("true", TokenTag::BOOL_LITERAL, Literal, Some(true)),
        ("false", TokenTag::BOOL_LITERAL, Literal, Some(false)),
        ("none", TokenTag::NONE_LITERAL, Literal, None),
        ("Int", TokenTag::DATATYPE_INT, BuiltinType, None),
        ("Float", TokenTag::DATATYPE_FLOAT, BuiltinType, None),
        ("Bool", TokenTag::DATATYPE_BOOL, BuiltinType, None),
        ("String", TokenTag::DATATYPE_STRING, BuiltinType, None),
        ("Char", TokenTag::DATATYPE_CHAR, BuiltinType, None),
        ("None", TokenTag::DATATYPE_NONE, BuiltinType, None),
        ("True", TokenTag::DATATYPE_TRUE, BuiltinType, None),
        ("False", TokenTag::DATATYPE_FALSE, BuiltinType, None),
        ("I8", TokenTag::DATATYPE_I8, BuiltinType, None),
        ("I16", TokenTag::DATATYPE_I16, BuiltinType, None),
        ("I32", TokenTag::DATATYPE_I32, BuiltinType, None),
        ("I64", TokenTag::DATATYPE_I64, BuiltinType, None),
        ("U8", TokenTag::DATATYPE_U8, BuiltinType, None),
        ("U16", TokenTag::DATATYPE_U16, BuiltinType, None),
        ("U32", TokenTag::DATATYPE_U32, BuiltinType, None),
        ("U64", TokenTag::DATATYPE_U64, BuiltinType, None),
        ("F16", TokenTag::DATATYPE_F16, BuiltinType, None),
        ("F32", TokenTag::DATATYPE_F32, BuiltinType, None),
        ("F64", TokenTag::DATATYPE_F64, BuiltinType, None),
        ("Byte", TokenTag::DATATYPE_BYTE, BuiltinType, None),
    ];
    assert_eq!(
        expected.len(),
        SourceWord::ALL.len(),
        "every exact source word needs an authored expectation"
    );

    for &(spelling, expected_tag, expected_class, expected_bool) in expected {
        let word = classify_source_word(spelling)
            .unwrap_or_else(|| panic!("{spelling:?} must be an exact source word"));
        assert_eq!(word.class(), expected_class, "wrong class for {spelling:?}");
        assert_eq!(
            word.bool_literal_value(),
            expected_bool,
            "wrong literal payload for {spelling:?}"
        );

        let token_tag = token_tag_for_source_word(word);
        assert_eq!(token_tag, expected_tag, "wrong token for {spelling:?}");
        let category_matches = match expected_class {
            Keyword => token_tag.is_keyword(),
            WordOperator => token_tag.is_word_operator(),
            Literal => token_tag.is_literal(),
            BuiltinType => token_tag.is_builtin_type(),
        };
        assert!(category_matches, "wrong token category for {spelling:?}");
    }
}

#[test]
fn attached_bang_words_keep_their_dedicated_token_tags() {
    assert_eq!(
        attached_bang_keyword_token_tag("return"),
        Some(TokenTag::RETURN_BANG)
    );
    assert_eq!(
        attached_bang_keyword_token_tag("cast"),
        Some(TokenTag::CAST_BANG)
    );
    assert_eq!(attached_bang_keyword_token_tag("if"), None);
    assert_eq!(attached_bang_keyword_token_tag("return!"), None);
}
