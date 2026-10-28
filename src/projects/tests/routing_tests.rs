//! Tests for shared routing policy parsing.

use super::{
    HtmlSiteConfig, PageUrlStyle, parse_html_site_config, prefix_origin, strip_origin_prefix,
};
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
fn parse_site_config(
    config: &Config,
    string_table: &mut StringTable,
) -> Result<HtmlSiteConfig, ProjectConfigError> {
    let mut diagnostic_strings = DiagnosticStringBuilder::new(string_table);
    parse_html_site_config(config, &mut diagnostic_strings)
}

/// Compact the typed `InvalidConfig` diagnostic from a config parse error.
///
/// WHAT: exposes the stable family view used by routing rejection assertions.
/// WHY: routing tests assert compact config facts directly from the stable schema.
fn invalid_config_diagnostics(error: ProjectConfigError) -> CompactDiagnosticBagForTest {
    let diagnostic = error
        .into_diagnostic()
        .expect("config error should be typed");
    compact_bag_for_test(DiagnosticBag::from(diagnostic), None)
}

fn invalid_config_primary_span(error: ProjectConfigError) -> Option<SourceSpan> {
    match invalid_config_diagnostics(error)
        .first()
        .expect("config error should have a compact diagnostic")
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

/// Assert that parsing `config` rejects `origin` with a typed value diagnostic.
///
/// WHAT: confirms the rejection carries the authored key and the exact invalid value.
/// WHY: origin validation reports the offending input through `InvalidProjectSettingValue`.
fn assert_origin_value_rejection(
    config: &Config,
    string_table: &mut StringTable,
    invalid_value: &str,
) {
    let error = parse_site_config(config, string_table).expect_err("invalid origin should fail");
    let compact = invalid_config_diagnostics(error);
    let diagnostic = compact.first().expect("expected one config diagnostic");
    let facts = diagnostic
        .family_view::<InvalidConfigView>()
        .expect("config error should retain typed invalid-config facts");
    assert_eq!(facts.reason, InvalidConfigCode::InvalidProjectSettingValue);
    let strings = diagnostic.string_ids();
    assert_eq!(
        string_table.resolve(*strings.last().expect("origin key should be present")),
        "origin",
    );
    assert_eq!(
        string_table.resolve(strings[0]),
        invalid_value,
        "origin rejection should report the exact invalid value",
    );
}

#[test]
fn defaults_are_applied_when_settings_are_missing() {
    let config = Config::new(PathBuf::from("project"));
    let mut string_table = StringTable::new();
    assert_eq!(
        parse_site_config(&config, &mut string_table).expect("defaults should parse"),
        HtmlSiteConfig::default()
    );
}

#[test]
fn parser_accepts_valid_overrides() {
    let mut config = Config::new(PathBuf::from("project"));
    config
        .builder_settings
        .set_string("html", "origin", String::from("/moth"), None);
    config.builder_settings.set_string(
        "html",
        "page_url_style",
        String::from("no_trailing_slash"),
        None,
    );
    config
        .builder_settings
        .set_bool("html", "redirect_index_html", false, None);

    let mut string_table = StringTable::new();
    let parsed =
        parse_site_config(&config, &mut string_table).expect("valid settings should parse");
    assert_eq!(parsed.origin, "/moth");
    assert_eq!(parsed.page_url_style, PageUrlStyle::NoTrailingSlash);
    assert!(!parsed.redirect_index_html);
}

#[test]
fn parser_rejects_invalid_origin() {
    let mut config = Config::new(PathBuf::from("project"));
    let mut string_table = StringTable::new();

    // No leading slash.
    config
        .builder_settings
        .set_string("html", "origin", String::from("moth"), None);
    assert_origin_value_rejection(&config, &mut string_table, "moth");

    // Trailing slash on a non-root prefix.
    config
        .builder_settings
        .set_string("html", "origin", String::from("/moth/"), None);
    assert_origin_value_rejection(&config, &mut string_table, "/moth/");

    // Empty origin is a separate empty-setting reason.
    config
        .builder_settings
        .set_string("html", "origin", String::new(), None);
    let empty_error =
        parse_site_config(&config, &mut string_table).expect_err("empty origin should fail");
    let compact = invalid_config_diagnostics(empty_error);
    let diagnostic = compact.first().expect("expected one config diagnostic");
    let facts = diagnostic
        .family_view::<InvalidConfigView>()
        .expect("config error should retain typed invalid-config facts");
    assert_eq!(facts.reason, InvalidConfigCode::EmptyProjectSetting);
    assert_eq!(
        string_table.resolve(
            *diagnostic
                .string_ids()
                .last()
                .expect("origin key should be present")
        ),
        "origin",
    );

    // Query or fragment characters are rejected as non-path content.
    config
        .builder_settings
        .set_string("html", "origin", String::from("/?x=1"), None);
    assert_origin_value_rejection(&config, &mut string_table, "/?x=1");
}

#[test]
fn parser_rejects_invalid_page_url_style() {
    let mut config = Config::new(PathBuf::from("project"));
    config
        .builder_settings
        .set_string("html", "page_url_style", String::from("slashy"), None);

    let mut string_table = StringTable::new();
    let error =
        parse_site_config(&config, &mut string_table).expect_err("invalid value should fail");
    let compact = invalid_config_diagnostics(error);
    let diagnostic = compact.first().expect("expected one config diagnostic");
    let facts = diagnostic
        .family_view::<InvalidConfigView>()
        .expect("config error should retain typed invalid-config facts");
    assert_eq!(facts.reason, InvalidConfigCode::InvalidProjectSettingValue);
    let strings = diagnostic.string_ids();
    assert_eq!(
        string_table.resolve(
            *strings
                .last()
                .expect("page_url_style key should be present")
        ),
        "page_url_style",
    );
    assert_eq!(
        string_table.resolve(strings[0]),
        "slashy",
        "page_url_style rejection should report the exact invalid value",
    );
    let expected_values = string_table.resolve(strings[1]);
    assert!(expected_values.contains("trailing_slash"));
    assert!(expected_values.contains("no_trailing_slash"));
    assert!(expected_values.contains("ignore"));
}

#[test]
fn parser_uses_precise_span_from_setting_spans() {
    let mut config = Config::new(PathBuf::from("project"));

    // Store an authored span alongside the generic builder value.
    let mut string_table = StringTable::new();
    let mut span_builder = ExtendedSpanBuilder::new();
    let precise_span = SourceSpan::new(
        SourceId::from_index(2),
        LocalSpan::exact(0, 7, &mut span_builder).expect("test span should fit"),
    );
    config.builder_settings.set_string(
        "html",
        "origin",
        String::from("invalid"),
        Some(precise_span),
    );

    let error =
        parse_site_config(&config, &mut string_table).expect_err("invalid origin should fail");

    assert_eq!(invalid_config_primary_span(error), Some(precise_span));
}

#[test]
fn parser_uses_no_span_when_setting_has_no_authored_source() {
    let mut config = Config::new(PathBuf::from("project"));
    config
        .builder_settings
        .set_string("html", "origin", String::from("invalid"), None);

    let mut string_table = StringTable::new();
    let error =
        parse_site_config(&config, &mut string_table).expect_err("invalid origin should fail");

    assert_eq!(invalid_config_primary_span(error), None);
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
