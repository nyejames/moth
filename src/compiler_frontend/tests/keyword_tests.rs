use crate::compiler_frontend::keywords::{
    ClassifiedSourceWord, SourceWordClass, attached_bang_keyword_token_kind, classify_source_word,
    is_identifier_continue, is_keyword, is_valid_identifier, keyword_token_kind,
};
use crate::compiler_frontend::symbols::identifier_policy::keyword_shadow_match;
use crate::compiler_frontend::tokenizer::tokens::TokenKind;

#[test]
fn keyword_policy_maps_exact_tokenizer_spellings() {
    let exact_keywords = [
        ("export", TokenKind::Export),
        ("this", TokenKind::This),
        ("This", TokenKind::TraitThis),
        ("true", TokenKind::BoolLiteral(true)),
        ("True", TokenKind::DatatypeTrue),
        ("none", TokenKind::NoneLiteral),
        ("None", TokenKind::DatatypeNone),
        ("to", TokenKind::ExclusiveRange),
        ("copy", TokenKind::Copy),
        ("cast", TokenKind::Cast),
    ];

    for (source, expected_kind) in exact_keywords {
        assert_eq!(keyword_token_kind(source), Some(expected_kind));
        assert!(is_keyword(source));
    }
}

#[test]
fn keyword_policy_keeps_case_sensitive_non_keywords_as_identifiers() {
    assert_eq!(keyword_token_kind("import"), None);
    assert_eq!(keyword_token_kind("Import"), None);
    assert_eq!(keyword_token_kind("Copy"), None);

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
        ("export", TokenKind::Export),
        ("if", TokenKind::If),
        ("return", TokenKind::Return),
        ("catch", TokenKind::Catch),
        ("then", TokenKind::Then),
        ("else", TokenKind::Else),
        ("checked", TokenKind::Checked),
        ("cast", TokenKind::Cast),
        ("as", TokenKind::As),
        ("type", TokenKind::Type),
        ("of", TokenKind::Of),
        ("must", TokenKind::Must),
        ("this", TokenKind::This),
        ("This", TokenKind::TraitThis),
        ("async", TokenKind::Async),
        ("yield", TokenKind::Yield),
        ("loop", TokenKind::Loop),
        ("to", TokenKind::ExclusiveRange),
        ("by", TokenKind::By),
        ("break", TokenKind::Break),
        ("continue", TokenKind::Continue),
        ("copy", TokenKind::Copy),
        ("assert", TokenKind::Assert),
    ];

    for (source, expected_kind) in keywords {
        let classified = classify_source_word(source)
            .unwrap_or_else(|| panic!("expected {source:?} to classify as a keyword"));
        assert_eq!(classified.class, SourceWordClass::Keyword);
        assert_eq!(classified.token_kind, expected_kind);
        assert_eq!(keyword_token_kind(source), Some(expected_kind));
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
        ("is", TokenKind::Is),
        ("not", TokenKind::Not),
        ("and", TokenKind::And),
        ("or", TokenKind::Or),
    ];

    for (source, expected_kind) in operators {
        let classified = classify_source_word(source)
            .unwrap_or_else(|| panic!("expected {source:?} to classify as a word operator"));
        assert_eq!(classified.class, SourceWordClass::WordOperator);
        assert_eq!(classified.token_kind, expected_kind);
        assert_eq!(keyword_token_kind(source), Some(expected_kind));
    }
}

#[test]
fn source_word_classifier_maps_literal_words() {
    let literals = [
        ("true", TokenKind::BoolLiteral(true)),
        ("false", TokenKind::BoolLiteral(false)),
        ("none", TokenKind::NoneLiteral),
    ];

    for (source, expected_kind) in literals {
        let classified = classify_source_word(source)
            .unwrap_or_else(|| panic!("expected {source:?} to classify as a literal"));
        assert_eq!(classified.class, SourceWordClass::Literal);
        assert_eq!(classified.token_kind, expected_kind);
        assert_eq!(keyword_token_kind(source), Some(expected_kind));
    }
}

#[test]
fn source_word_classifier_maps_builtin_type_words() {
    let types = [
        ("Int", TokenKind::DatatypeInt),
        ("Float", TokenKind::DatatypeFloat),
        ("Bool", TokenKind::DatatypeBool),
        ("String", TokenKind::DatatypeString),
        ("Char", TokenKind::DatatypeChar),
        ("None", TokenKind::DatatypeNone),
        ("True", TokenKind::DatatypeTrue),
        ("False", TokenKind::DatatypeFalse),
    ];

    for (source, expected_kind) in types {
        let classified = classify_source_word(source)
            .unwrap_or_else(|| panic!("expected {source:?} to classify as a builtin type"));
        assert_eq!(classified.class, SourceWordClass::BuiltinType);
        assert_eq!(classified.token_kind, expected_kind);
        assert_eq!(keyword_token_kind(source), Some(expected_kind));
    }
}

#[test]
fn source_word_classifier_is_case_sensitive() {
    assert_eq!(classify_source_word("Import"), None);
    assert_eq!(classify_source_word("RETURN"), None);
    assert_eq!(
        classify_source_word("Int"),
        Some(ClassifiedSourceWord {
            token_kind: TokenKind::DatatypeInt,
            class: SourceWordClass::BuiltinType,
        })
    );
    assert_eq!(classify_source_word("int"), None);
    assert_eq!(classify_source_word("TRUE"), None);
    assert_eq!(
        classify_source_word("none"),
        Some(ClassifiedSourceWord {
            token_kind: TokenKind::NoneLiteral,
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
        assert_eq!(keyword_token_kind(source), None);
    }
}

#[test]
fn attached_bang_keyword_authority_covers_return_and_cast() {
    assert_eq!(
        attached_bang_keyword_token_kind("return"),
        Some(TokenKind::ReturnBang)
    );
    assert_eq!(
        attached_bang_keyword_token_kind("cast"),
        Some(TokenKind::CastBang)
    );
    assert_eq!(attached_bang_keyword_token_kind("if"), None);
    assert_eq!(attached_bang_keyword_token_kind("return!"), None);
}
