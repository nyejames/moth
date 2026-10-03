use crate::compiler_frontend::symbols::identifier_policy::*;

#[test]
fn type_and_value_style_helpers_follow_policy() {
    assert!(is_camel_case_type_name("User"));
    assert!(is_camel_case_type_name("Http2Client"));
    assert!(!is_camel_case_type_name("user"));
    assert!(!is_camel_case_type_name("User_Name"));

    assert!(is_lowercase_with_underscores_name("user_name"));
    assert!(is_lowercase_with_underscores_name("_user_name"));
    assert!(is_lowercase_with_underscores_name("value2"));
    assert!(!is_lowercase_with_underscores_name("VALUE"));
    assert!(!is_lowercase_with_underscores_name("__"));

    assert!(is_uppercase_constant_name("SITE_NAME"));
    assert!(is_uppercase_constant_name("HTTP2_PORT"));
    assert!(!is_uppercase_constant_name("Site_Name"));
    assert!(!is_uppercase_constant_name("___"));
}

#[test]
fn unicode_identifier_style_helpers_follow_policy() {
    assert!(is_lowercase_with_underscores_name("café"));
    assert!(is_lowercase_with_underscores_name("naïve_value"));
    assert!(!is_lowercase_with_underscores_name("Café"));

    assert!(is_camel_case_type_name("Éclair"));
    assert!(is_uppercase_constant_name("É_CONST"));
}
