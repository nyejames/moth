use crate::compiler_frontend::keywords::{
    ClassifiedSourceWord, SourceWordClass, attached_bang_keyword_token_tag,
    classify_source_word, is_identifier_continue, is_keyword, is_valid_identifier,
    keyword_token_tag,
};
use crate::compiler_frontend::symbols::identifier_policy::keyword_shadow_match;
use crate::compiler_frontend::tokenizer::tokens::TokenTag;

#[test]
fn keyword_policy_maps_exact_tokenizer_spellings() {
    let exact_keywords = [
        ("export", TokenTag::EXPORT, None),
        ("this", TokenTag::THIS, None),
        ("This", TokenTag::TRAIT_THIS, None),
        ("true", TokenTag::BOOL_LITERAL, Some(true)),
        ("True", TokenTag::DATATYPE_TRUE, None),
        ("none", TokenTag::NONE_LITERAL, None),
        ("None", TokenTag::DATATYPE_NONE, None),
        ("to", TokenTag::EXCLUSIVE_RANGE, None),
        ("copy", TokenTag::COPY, None),
        ("cast", TokenTag::CAST, None),
    ];

    for (source, expected_tag, expected_bool) in exact_keywords {
        assert_eq!(keyword_token_tag(source), Some(expected_tag));
        let classified = classify_source_word(source)
            .expect("exact keyword spelling should classify as a source word");
        assert_eq!(classified.token_tag, expected_tag);
        assert_eq!(classified.bool_value, expected_bool);
        assert!(is_keyword(source));
    }
}

#[test]
fn keyword_policy_keeps_case_sensitive_non_keywords_as_identifiers() {
    assert_eq!(keyword_token_tag("import"), None);
    assert_eq!(keyword_token_tag("Import"), None);
    assert_eq!(keyword_token_tag("Copy"), None);

    assert!(is_valid_identifier("import"));
    assert!(is_valid_identifier("Import"));
    assert!(is_valid_identifier("_copy"));
}

#[test]
fn keyword_shadow_policy_shares_the_canonical_keyword_set() {
    assert_eq!(keyword_shadow_match("__LoOp"), Some("loop"));
    assert_eq!(keyword_shadow_match("_FALSE"), Some("false"));
    assert_eq!(keyword_shadow_match("_not_a_keyword"), None);

    assert_eq!(keyword_shadow_match("export"), Some("export"));
    assert_eq!(keyword_shadow_match("EXPORT"), Some("export"));
    assert_eq!(keyword_shadow_match("_export"), Some("export"));

    assert_eq!(keyword_shadow_match("cast"), Some("cast"));
    assert_eq!(keyword_shadow_match("CAST"), Some("cast"));
    assert_eq!(keyword_shadow_match("_cast"), Some("cast"));

    // The `config` identifier family is reserved for the compiler-owned build-config
    // vocabulary without becoming a tokenizer keyword.
    assert_eq!(keyword_shadow_match("config"), Some("config"));
    assert_eq!(keyword_shadow_match("Config"), Some("config"));
    assert_eq!(keyword_shadow_match("CONFIG"), Some("config"));
    assert_eq!(keyword_shadow_match("_config"), Some("config"));
    assert_eq!(keyword_shadow_match("_Config"), Some("config"));
    assert!(!is_keyword("config"));
    assert!(!is_keyword("Config"));
}

#[test]
fn identifier_policy_matches_tokenizer_identifier_characters() {
    assert!(is_identifier_continue('a'));
    assert!(is_identifier_continue('9'));
    assert!(is_identifier_continue('_'));
    assert!(!is_identifier_continue('-'));

    assert!(is_valid_identifier("_valid_12"));
    assert!(!is_valid_identifier("12_invalid"));
    assert!(!is_valid_identifier("bad-name"));
}

#[test]
fn source_word_classifier_maps_keyword_words() {
    let keywords = [
        ("export", TokenTag::EXPORT),
        ("if", TokenTag::IF),
        ("return", TokenTag::RETURN),
        ("catch", TokenTag::CATCH),
        ("then", TokenTag::THEN),
        ("else", TokenTag::ELSE),
        ("checked", TokenTag::CHECKED),
        ("cast", TokenTag::CAST),
        ("as", TokenTag::AS),
        ("type", TokenTag::TYPE),
        ("of", TokenTag::OF),
        ("must", TokenTag::MUST),
        ("this", TokenTag::THIS),
        ("This", TokenTag::TRAIT_THIS),
        ("async", TokenTag::ASYNC),
        ("yield", TokenTag::YIELD),
        ("loop", TokenTag::LOOP),
        ("to", TokenTag::EXCLUSIVE_RANGE),
        ("by", TokenTag::BY),
        ("break", TokenTag::BREAK),
        ("continue", TokenTag::CONTINUE),
        ("copy", TokenTag::COPY),
        ("assert", TokenTag::ASSERT),
    ];

    for (source, expected_tag) in keywords {
        let classified = classify_source_word(source)
            .unwrap_or_else(|| panic!("expected {source:?} to classify as a keyword"));
        assert_eq!(classified.class, SourceWordClass::Keyword);
        assert_eq!(classified.token_tag, expected_tag);
        assert_eq!(classified.bool_value, None);
        assert_eq!(keyword_token_tag(source), Some(expected_tag));
    }
}

#[test]
fn import_is_an_ordinary_identifier() {
    assert_eq!(classify_source_word("import"), None);
    assert_eq!(keyword_shadow_match("import"), None);
    assert_eq!(keyword_shadow_match("IMPORT"), None);
}

#[test]
fn block_spellings_follow_ordinary_identifier_policy() {
    for identifier in ["block", "_block", "block_value"] {
        assert_eq!(classify_source_word(identifier), None);
        assert_eq!(keyword_shadow_match(identifier), None);
        assert!(is_valid_identifier(identifier));
    }
}

#[test]
fn source_word_classifier_maps_word_operator_words() {
    let operators = [
        ("is", TokenTag::IS),
        ("not", TokenTag::NOT),
        ("and", TokenTag::AND),
        ("or", TokenTag::OR),
    ];

    for (source, expected_tag) in operators {
        let classified = classify_source_word(source)
            .unwrap_or_else(|| panic!("expected {source:?} to classify as a word operator"));
        assert_eq!(classified.class, SourceWordClass::WordOperator);
        assert_eq!(classified.token_tag, expected_tag);
        assert_eq!(classified.bool_value, None);
        assert_eq!(keyword_token_tag(source), Some(expected_tag));
    }
}

#[test]
fn source_word_classifier_maps_literal_words() {
    let literals = [
        ("true", TokenTag::BOOL_LITERAL, Some(true)),
        ("false", TokenTag::BOOL_LITERAL, Some(false)),
        ("none", TokenTag::NONE_LITERAL, None),
    ];

    for (source, expected_tag, expected_bool) in literals {
        let classified = classify_source_word(source)
            .unwrap_or_else(|| panic!("expected {source:?} to classify as a literal"));
        assert_eq!(classified.class, SourceWordClass::Literal);
        assert_eq!(classified.token_tag, expected_tag);
        assert_eq!(classified.bool_value, expected_bool);
        assert_eq!(keyword_token_tag(source), Some(expected_tag));
    }
}

#[test]
fn source_word_classifier_maps_builtin_type_words() {
    let types = [
        ("Int", TokenTag::DATATYPE_INT),
        ("Float", TokenTag::DATATYPE_FLOAT),
        ("Bool", TokenTag::DATATYPE_BOOL),
        ("String", TokenTag::DATATYPE_STRING),
        ("Char", TokenTag::DATATYPE_CHAR),
        ("None", TokenTag::DATATYPE_NONE),
        ("True", TokenTag::DATATYPE_TRUE),
        ("False", TokenTag::DATATYPE_FALSE),
    ];

    for (source, expected_tag) in types {
        let classified = classify_source_word(source)
            .unwrap_or_else(|| panic!("expected {source:?} to classify as a builtin type"));
        assert_eq!(classified.class, SourceWordClass::BuiltinType);
        assert_eq!(classified.token_tag, expected_tag);
        assert_eq!(classified.bool_value, None);
        assert_eq!(keyword_token_tag(source), Some(expected_tag));
    }
}

fn source_word_classifier_is_case_sensitive() {
    assert_eq!(classify_source_word("Import"), None);
    assert_eq!(classify_source_word("RETURN"), None);
    assert_eq!(
        classify_source_word("Int"),
        Some(ClassifiedSourceWord {
            token_tag: TokenTag::DATATYPE_INT,
            bool_value: None,
            class: SourceWordClass::BuiltinType,
        })
    );
    assert_eq!(classify_source_word("int"), None);
    assert_eq!(classify_source_word("TRUE"), None);
    assert_eq!(
        classify_source_word("none"),
        Some(ClassifiedSourceWord {
            token_tag: TokenTag::NONE_LITERAL,
            bool_value: None,
            class: SourceWordClass::Literal,
        })
    );
}

#[test]
fn source_word_classifier_keeps_planned_and_invalid_words_unclassified() {
    for source in ["in", "fn", "group", "into", "where"] {
        assert_eq!(
            classify_source_word(source),
            None,
            "{source:?} must stay unclassified"
        );
        assert_eq!(keyword_token_tag(source), None);
    }
}

#[test]
fn attached_bang_keyword_authority_covers_return_and_cast() {
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

#[test]
fn keyword_tag_projection_matches_taxonomy() {
    for (source, expected_tag) in [
        ("export", TokenTag::EXPORT),
        ("if", TokenTag::IF),
        ("return", TokenTag::RETURN),
        ("cast", TokenTag::CAST),
        ("is", TokenTag::IS),
        ("copy", TokenTag::COPY),
        ("Int", TokenTag::DATATYPE_INT),
        ("true", TokenTag::BOOL_LITERAL),
        ("none", TokenTag::NONE_LITERAL),
    ] {
        assert_eq!(keyword_token_tag(source), Some(expected_tag));
    }
    assert_eq!(keyword_token_tag("import"), None);
}
