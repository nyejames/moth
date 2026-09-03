use crate::compiler_frontend::symbols::identifier_policy::*;

#[test]
fn keyword_shadow_matching_ignores_case_and_leading_underscores() {
    assert_eq!(keyword_shadow_match("_true"), Some("true"));
    assert_eq!(keyword_shadow_match("FALSE"), Some("false"));
    assert_eq!(keyword_shadow_match("__LoOp"), Some("loop"));
    assert_eq!(keyword_shadow_match("block"), None);
    assert_eq!(keyword_shadow_match("Block"), None);
    assert_eq!(keyword_shadow_match("_BLOCK"), None);
    assert_eq!(keyword_shadow_match("checked"), Some("checked"));
    assert_eq!(keyword_shadow_match("Checked"), Some("checked"));
    assert_eq!(keyword_shadow_match("_async"), Some("async"));
    assert_eq!(keyword_shadow_match("type"), Some("type"));
    assert_eq!(keyword_shadow_match("_Of"), Some("of"));
    assert_eq!(keyword_shadow_match("this"), Some("this"));
    assert_eq!(keyword_shadow_match("This"), Some("this"));
    assert_eq!(keyword_shadow_match("_This"), Some("this"));
    assert_eq!(keyword_shadow_match("THIS"), Some("this"));
    assert_eq!(keyword_shadow_match("assert"), Some("assert"));
    assert_eq!(keyword_shadow_match("ASSERT"), Some("assert"));
    assert_eq!(keyword_shadow_match("_assert"), Some("assert"));

    assert_eq!(keyword_shadow_match("export"), Some("export"));
    assert_eq!(keyword_shadow_match("EXPORT"), Some("export"));
    assert_eq!(keyword_shadow_match("_export"), Some("export"));

    assert_eq!(keyword_shadow_match("config"), Some("config"));
    assert_eq!(keyword_shadow_match("Config"), Some("config"));
    assert_eq!(keyword_shadow_match("CONFIG"), Some("config"));
    assert_eq!(keyword_shadow_match("_config"), Some("config"));
    assert_eq!(keyword_shadow_match("_Config"), Some("config"));

    // Longer words that only start with the reserved spelling stay ordinary.
    assert_eq!(keyword_shadow_match("configure"), None);
    assert_eq!(keyword_shadow_match("Configured"), None);
    assert_eq!(keyword_shadow_match("_configuration"), None);
}

#[test]
fn keyword_shadow_matching_rejects_non_keywords() {
    assert_eq!(keyword_shadow_match("value"), None);
    assert_eq!(keyword_shadow_match("_"), None);
    assert_eq!(keyword_shadow_match("___"), None);
    assert_eq!(keyword_shadow_match("error"), None);
}

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
