//! Tests for HTML document-shell config parsing.

use super::*;
use crate::compiler_frontend::compiler_errors::SourceLocation;
use crate::compiler_frontend::compiler_messages::{DiagnosticPayload, InvalidConfigReason};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::projects::settings::Config;
use std::path::PathBuf;

fn project_config() -> Config {
    Config::new(PathBuf::from("project"))
}

fn set_setting(config: &mut Config, key: &str, value: &str) {
    config.settings.insert(key.to_owned(), value.to_owned());
}

#[test]
fn defaults_are_applied_when_settings_are_missing() {
    let config = project_config();
    let mut string_table = StringTable::new();
    assert_eq!(
        parse_html_document_config(&config, &mut string_table).expect("defaults should parse"),
        HtmlDocumentConfig::default()
    );
}

#[test]
fn parser_accepts_valid_overrides() {
    let mut config = project_config();
    set_setting(&mut config, "html_lang", "en-GB");
    set_setting(&mut config, "html_title_prefix", "Docs | ");
    set_setting(&mut config, "html_title_postfix", " | Moth");
    set_setting(&mut config, "html_favicon", "/assets/favicon.ico");
    set_setting(&mut config, "html_inject_charset", "false");
    set_setting(&mut config, "html_inject_viewport", "false");
    set_setting(&mut config, "html_inject_color_scheme", "false");
    set_setting(&mut config, "html_inject_core_css", "false");
    set_setting(&mut config, "html_body_style", "margin: 0;");

    let mut string_table = StringTable::new();
    let parsed = parse_html_document_config(&config, &mut string_table)
        .expect("valid settings should parse");
    assert_eq!(parsed.lang, "en-GB");
    assert_eq!(parsed.title_prefix, "Docs | ");
    assert_eq!(parsed.title_postfix, " | Moth");
    assert_eq!(parsed.favicon, Some(String::from("/assets/favicon.ico")));
    assert!(!parsed.inject_charset);
    assert!(!parsed.inject_viewport);
    assert!(!parsed.inject_color_scheme);
    assert!(!parsed.inject_core_css);
    assert_eq!(parsed.body_style, "margin: 0;");
}

#[test]
fn parser_rejects_empty_lang() {
    let mut config = project_config();
    set_setting(&mut config, "html_lang", "");

    let mut string_table = StringTable::new();
    let error =
        parse_html_document_config(&config, &mut string_table).expect_err("empty lang should fail");
    let diagnostic = error.diagnostic().expect("config error should be typed");
    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidConfig {
            reason: InvalidConfigReason::EmptyProjectSetting,
            ..
        }
    ));
}

#[test]
fn parser_rejects_invalid_bool_values() {
    let mut config = project_config();
    set_setting(&mut config, "html_inject_core_css", "yes");

    let mut string_table = StringTable::new();
    let error = parse_html_document_config(&config, &mut string_table)
        .expect_err("invalid bool should fail");
    let diagnostic = error.diagnostic().expect("config error should be typed");
    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidConfig {
            reason: InvalidConfigReason::InvalidProjectSettingValue { .. },
            ..
        }
    ));
}

#[test]
fn parser_uses_precise_location_from_setting_locations() {
    let mut config = project_config();
    set_setting(&mut config, "html_lang", "");
    let mut string_table = StringTable::new();
    let precise_location = SourceLocation::new(
        InternedPath::try_from_filesystem_path(
            PathBuf::from("project/config.moth").as_path(),
            &mut string_table,
        )
        .expect("test path should be UTF-8"),
        Default::default(),
        Default::default(),
    );
    config
        .setting_locations
        .insert(String::from("html_lang"), precise_location.clone());

    let error = parse_html_document_config(&config, &mut string_table)
        .expect_err("invalid lang should fail");
    let diagnostic = error.diagnostic().expect("config error should be typed");
    assert_eq!(diagnostic.primary_location.scope, precise_location.scope);
}

#[test]
fn parser_falls_back_to_config_file_location() {
    let mut config = project_config();
    set_setting(&mut config, "html_inject_core_css", "invalid");

    let mut string_table = StringTable::new();
    let error = parse_html_document_config(&config, &mut string_table)
        .expect_err("invalid bool should fail");
    let diagnostic = error.diagnostic().expect("config error should be typed");
    assert_eq!(
        diagnostic.primary_location.scope.to_path_buf(&string_table),
        PathBuf::from("project/config.moth")
    );
}
