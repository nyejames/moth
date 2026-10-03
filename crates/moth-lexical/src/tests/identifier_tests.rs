//! Tests for shared identifier shape and reserved-name policy.

use super::*;

#[test]
fn identifier_shape_uses_unicode_alphabetic_and_alphanumeric_rules() {
    for identifier in [
        "é", "name7", "_", "__x", "é2", "import", "Import", "_copy", "block", "Number",
    ] {
        assert!(
            is_identifier(identifier),
            "{identifier:?} should be an identifier"
        );
    }

    for invalid in ["", "1name", "bad-name", "bad name"] {
        assert!(
            !is_identifier(invalid),
            "{invalid:?} should not be an identifier"
        );
    }

    assert!(is_identifier_start('é'));
    assert!(is_identifier_continue('7'));
    assert!(!is_identifier_start('7'));
    assert!(!is_identifier_continue('-'));
}

#[test]
fn reserved_user_names_ignore_case_and_leading_underscores() {
    for reserved in [
        "_true", "FALSE", "__LoOp", "Fn", "CONFIG", "_Config", "int", "INT", "u8", "_U8", "Byte",
        "F64", "this", "THIS", "dec", "Dec0", "Dec2", "Dec256", "dec01", "DEC257", "_Dec9",
        "__dEc2",
    ] {
        assert!(
            is_reserved_user_name(reserved),
            "{reserved:?} should be reserved"
        );
    }
}

#[test]
fn reserved_user_names_reject_whole_word_near_misses() {
    for ordinary in [
        "truth",
        "fnx",
        "DecBox",
        "Dec1a",
        "config_",
        "___",
        "ifx",
        "import",
        "IMPORT",
        "block",
        "_block",
        "block_value",
        "Number",
        "_Number",
        "Number0",
        "Number256",
        "i128",
        "I128",
        "Decimal",
        "dec2extra",
    ] {
        assert!(
            !is_reserved_user_name(ordinary),
            "{ordinary:?} should not be reserved"
        );
    }
}

#[test]
fn dec_family_policy_matches_only_bare_name_and_ascii_digit_suffixes() {
    for reserved in ["dec", "Dec2", "dec01", "DEC257", "_Dec9"] {
        assert!(is_reserved_dec_family_name(reserved));
    }

    for ordinary in ["DecBox", "Decimal", "Dec1a", "dec2extra", "Dec١", "dec_"] {
        assert!(!is_reserved_dec_family_name(ordinary));
    }
}
