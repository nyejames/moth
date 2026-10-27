//! Tests for HTML document-shell config parsing.

use super::HtmlDocumentConfig;
use crate::compiler_frontend::compiler_messages::DiagnosticBag;
use crate::compiler_frontend::compiler_messages::diagnostics::diagnostic_string_builder::DiagnosticStringBuilder;
use crate::compiler_frontend::compiler_messages::diagnostics::families::project_config::{
    InvalidConfigCode, InvalidConfigView,
};
use crate::compiler_frontend::compiler_messages::diagnostics::test_support::{
    CompactDiagnosticBagForTest, compact_bag_for_test,
};
use crate::compiler_frontend::source::{ExtendedSpanBuilder, LocalSpan, SourceId, SourceSpan};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::projects::settings::{Config, ProjectConfigError};

use std::path::PathBuf;

fn compact_config_error(error: ProjectConfigError) -> CompactDiagnosticBagForTest {
    let diagnostic = error
        .into_diagnostic()
        .expect("config error should be typed");
    compact_bag_for_test(DiagnosticBag::from(diagnostic), None)
}

fn config_error_primary_span(error: ProjectConfigError) -> Option<SourceSpan> {
    let compact = compact_config_error(error);
    match compact
        .first()
        .expect("config error should have a compact view")
        .primary_site()
    {
        crate::compiler_frontend::compiler_messages::diagnostics::foreign::SiteRef::Absent => None,
        crate::compiler_frontend::compiler_messages::diagnostics::foreign::SiteRef::Local(span)
        | crate::compiler_frontend::compiler_messages::diagnostics::foreign::SiteRef::Foreign {
            span,
            ..
        } => Some(span),
    }
}
fn project_config() -> Config {
    Config::new(PathBuf::from("project"))
}

fn parse_html_document_config(
    config: &Config,
    string_table: &mut StringTable,
) -> Result<HtmlDocumentConfig, ProjectConfigError> {
    let mut diagnostic_strings = DiagnosticStringBuilder::new(string_table);
    super::parse_html_document_config(config, &mut diagnostic_strings)
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
    let compact = compact_config_error(error);
    let diagnostic = compact
        .first()
        .expect("config error should have a compact view");
    let facts = diagnostic
        .family_view::<InvalidConfigView>()
        .expect("config error should retain typed facts");
    assert_eq!(facts.reason, InvalidConfigCode::EmptyProjectSetting);
}

#[test]
fn parser_uses_precise_span_from_setting_spans() {
    let mut config = project_config();
    config.html_section.html_lang = Some(String::new());
    let mut string_table = StringTable::new();
    let mut extended = ExtendedSpanBuilder::default();
    let precise_span = SourceSpan::new(
        SourceId::from_index(0),
        LocalSpan::exact(4, 2, &mut extended).expect("fixture span should fit"),
    );
    config
        .setting_spans
        .insert(String::from("html_lang"), precise_span);

    let error = parse_html_document_config(&config, &mut string_table)
        .expect_err("invalid lang should fail");
    assert_eq!(config_error_primary_span(error), Some(precise_span));
}

#[test]
fn parser_falls_back_to_spanless_config_file() {
    let mut config = project_config();
    config.html_section.html_lang = Some(String::new());

    // Don't add the key to setting_spans.

    let mut string_table = StringTable::new();
    let error =
        parse_html_document_config(&config, &mut string_table).expect_err("empty lang should fail");
    assert_eq!(config_error_primary_span(error), None);
}
