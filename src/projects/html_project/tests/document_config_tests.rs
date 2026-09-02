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
    config.html_section.html_lang = Some(String::from("en-GB"));
    config.html_section.html_title_prefix = Some(String::from("Docs | "));
    config.html_section.html_title_postfix = Some(String::from(" | Moth"));
    config.html_section.html_favicon = Some(String::from("/assets/favicon.ico"));
    config.html_section.html_inject_charset = Some(false);
    config.html_section.html_inject_viewport = Some(false);
    config.html_section.html_inject_color_scheme = Some(false);
    config.html_section.html_inject_core_css = Some(false);
    config.html_section.html_body_style = Some(String::from("margin: 0;"));

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
    config.html_section.html_lang = Some(String::new());

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
fn parser_uses_precise_location_from_setting_locations() {
    let mut config = project_config();
    config.html_section.html_lang = Some(String::new());
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
    config.html_section.html_lang = Some(String::new());

    // Don't add the key to setting_locations

    let mut string_table = StringTable::new();
    let error =
        parse_html_document_config(&config, &mut string_table).expect_err("empty lang should fail");
    let diagnostic = error.diagnostic().expect("config error should be typed");
    assert_eq!(
        diagnostic.primary_location.scope.to_path_buf(&string_table),
        PathBuf::from("project/config.moth")
    );
}
