//! Tests for shared routing policy parsing.

use super::{
    HtmlSiteConfig, PageUrlStyle, parse_html_site_config, prefix_origin, strip_origin_prefix,
};
use crate::compiler_frontend::compiler_messages::{DiagnosticPayload, InvalidConfigReason};
use crate::compiler_frontend::source::{ExtendedSpanBuilder, LocalSpan, SourceId, SourceSpan};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::projects::settings::{Config, ProjectConfigError};
use std::path::PathBuf;

/// Extract the typed `InvalidConfig` payload from a config parse error.
///
/// WHAT: returns the setting key and reason carried by the rejection diagnostic.
/// WHY: routing rejection tests assert exact typed key/value facts rather than rendered prose.
fn invalid_config_payload(error: &ProjectConfigError) -> (&Option<StringId>, &InvalidConfigReason) {
    let diagnostic = error.diagnostic().expect("config error should be typed");
    match &diagnostic.payload {
        DiagnosticPayload::InvalidConfig { key, reason } => (key, reason),
        _ => panic!("expected InvalidConfig payload"),
    }
}

/// Assert that parsing `config` rejects `origin` with a typed value diagnostic.
///
/// WHAT: confirms the rejection carries the authored key and the exact invalid value.
/// WHY: origin validation reports the offending input through `InvalidProjectSettingValue`.
fn assert_origin_value_rejection(
    config: &Config,
    string_table: &mut StringTable,
    invalid_value: &str,
) {
    let error =
        parse_html_site_config(config, string_table).expect_err("invalid origin should fail");
    let (key, reason) = invalid_config_payload(&error);
    assert_eq!(
        string_table.resolve(key.expect("origin key should be present")),
        "origin",
    );
    match reason {
        InvalidConfigReason::InvalidProjectSettingValue { value, .. } => {
            assert_eq!(
                string_table.resolve(*value),
                invalid_value,
                "origin rejection should report the exact invalid value",
            );
        }
        _ => panic!("expected InvalidProjectSettingValue for invalid origin"),
    }
}

#[test]
fn defaults_are_applied_when_settings_are_missing() {
    let config = Config::new(PathBuf::from("project"));
    let mut string_table = StringTable::new();
    assert_eq!(
        parse_html_site_config(&config, &mut string_table).expect("defaults should parse"),
        HtmlSiteConfig::default()
    );
}

#[test]
fn parser_accepts_valid_overrides() {
    let mut config = Config::new(PathBuf::from("project"));
    config.html_section.origin = Some(String::from("/moth"));
    config.html_section.page_url_style = Some(String::from("no_trailing_slash"));
    config.html_section.redirect_index_html = Some(false);

    let mut string_table = StringTable::new();
    let parsed =
        parse_html_site_config(&config, &mut string_table).expect("valid settings should parse");
    assert_eq!(parsed.origin, "/moth");
    assert_eq!(parsed.page_url_style, PageUrlStyle::NoTrailingSlash);
    assert!(!parsed.redirect_index_html);
}

#[test]
fn parser_rejects_invalid_origin() {
    let mut config = Config::new(PathBuf::from("project"));
    let mut string_table = StringTable::new();

    // No leading slash.
    config.html_section.origin = Some(String::from("moth"));
    assert_origin_value_rejection(&config, &mut string_table, "moth");

    // Trailing slash on a non-root prefix.
    config.html_section.origin = Some(String::from("/moth/"));
    assert_origin_value_rejection(&config, &mut string_table, "/moth/");

    // Empty origin is a separate empty-setting reason.
    config.html_section.origin = Some(String::from(""));
    let empty_error =
        parse_html_site_config(&config, &mut string_table).expect_err("empty origin should fail");
    let (key, reason) = invalid_config_payload(&empty_error);
    assert_eq!(
        string_table.resolve(key.expect("origin key should be present")),
        "origin",
    );
    assert!(
        matches!(reason, InvalidConfigReason::EmptyProjectSetting),
        "empty origin should report EmptyProjectSetting",
    );

    // Query or fragment characters are rejected as non-path content.
    config.html_section.origin = Some(String::from("/?x=1"));
    assert_origin_value_rejection(&config, &mut string_table, "/?x=1");
}

#[test]
fn parser_rejects_invalid_page_url_style() {
    let mut config = Config::new(PathBuf::from("project"));
    config.html_section.page_url_style = Some(String::from("slashy"));

    let mut string_table = StringTable::new();
    let error =
        parse_html_site_config(&config, &mut string_table).expect_err("invalid value should fail");
    let (key, reason) = invalid_config_payload(&error);
    assert_eq!(
        string_table.resolve(key.expect("page_url_style key should be present")),
        "page_url_style",
    );
    let InvalidConfigReason::InvalidProjectSettingValue { value, expected } = reason else {
        panic!("expected InvalidProjectSettingValue for invalid page_url_style");
    };
    assert_eq!(
        string_table.resolve(*value),
        "slashy",
        "page_url_style rejection should report the exact invalid value",
    );
    let expected_values = string_table.resolve(*expected);
    assert!(expected_values.contains("trailing_slash"));
    assert!(expected_values.contains("no_trailing_slash"));
    assert!(expected_values.contains("ignore"));
}

#[test]
fn parser_uses_precise_span_from_setting_spans() {
    let mut config = Config::new(PathBuf::from("project"));
    config.html_section.origin = Some(String::from("invalid"));

    // Store an authored span for the origin key.
    let mut string_table = StringTable::new();
    let mut span_builder = ExtendedSpanBuilder::new();
    let precise_span = SourceSpan::new(
        SourceId::from_index(2),
        LocalSpan::exact(0, 7, &mut span_builder).expect("test span should fit"),
    );
    config
        .setting_spans
        .insert(String::from("origin"), precise_span);

    let error =
        parse_html_site_config(&config, &mut string_table).expect_err("invalid origin should fail");

    let diagnostic = error.diagnostic().expect("config error should be typed");
    assert_eq!(diagnostic.primary_span, Some(precise_span));
}

#[test]
fn parser_uses_no_span_when_setting_has_no_authored_source() {
    let mut config = Config::new(PathBuf::from("project"));
    config.html_section.origin = Some(String::from("invalid"));

    let mut string_table = StringTable::new();
    let error =
        parse_html_site_config(&config, &mut string_table).expect_err("invalid origin should fail");

    let diagnostic = error.diagnostic().expect("config error should be typed");
    assert_eq!(diagnostic.primary_span, None);
}

#[test]
fn prefix_origin_works() {
    assert_eq!(prefix_origin("/", "/docs/"), "/docs/");
    assert_eq!(prefix_origin("/moth", "/docs/"), "/moth/docs/");
    assert_eq!(prefix_origin("/moth", "/"), "/moth/");
}

#[test]
fn strip_origin_prefix_works() {
    assert_eq!(
        strip_origin_prefix("/moth/docs/", "/moth"),
        Some(String::from("/docs/"))
    );
    assert_eq!(
        strip_origin_prefix("/moth/", "/moth"),
        Some(String::from("/"))
    );
    assert_eq!(
        strip_origin_prefix("/moth", "/moth"),
        Some(String::from("/"))
    );
    assert_eq!(strip_origin_prefix("/docs/", "/moth"), None);
    assert_eq!(strip_origin_prefix("/mothish/", "/moth"), None);
}
