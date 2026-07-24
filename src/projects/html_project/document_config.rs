//! Typed HTML document-shell configuration parsing.
//!
//! WHAT: parses HTML-shell-specific `config.moth` settings into a strict typed struct.
//! WHY: keeping document policy separate from routing config avoids one oversized parser and
//!      gives the HTML builder a single source of truth for shell defaults.

use crate::compiler_frontend::compiler_messages::InvalidConfigReason;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::projects::settings::{Config, ProjectConfigError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HtmlDocumentConfig {
    pub lang: String,
    pub title_prefix: String,
    pub title_postfix: String,
    pub favicon: Option<String>,
    pub inject_charset: bool,
    pub inject_viewport: bool,
    pub inject_color_scheme: bool,
    pub inject_core_css: bool,
    pub body_style: String,
}

impl Default for HtmlDocumentConfig {
    fn default() -> Self {
        Self {
            lang: String::from("en"),
            title_prefix: String::new(),
            title_postfix: String::new(),
            favicon: None,
            inject_charset: true,
            inject_viewport: true,
            inject_color_scheme: true,
            inject_core_css: true,
            body_style: String::new(),
        }
    }
}

pub(crate) fn parse_html_document_config(
    config: &Config,
    string_table: &mut StringTable,
) -> Result<HtmlDocumentConfig, ProjectConfigError> {
    Ok(HtmlDocumentConfig {
        lang: parse_required_string(config, "html_lang", "en", true, string_table)?,
        title_prefix: parse_required_string(config, "html_title_prefix", "", false, string_table)?,
        title_postfix: parse_required_string(
            config,
            "html_title_postfix",
            "",
            false,
            string_table,
        )?,
        favicon: parse_optional_string(config, "html_favicon", string_table)?,
        inject_charset: parse_bool(config, "html_inject_charset", true, string_table)?,
        inject_viewport: parse_bool(config, "html_inject_viewport", true, string_table)?,
        inject_color_scheme: parse_bool(config, "html_inject_color_scheme", true, string_table)?,
        inject_core_css: parse_bool(config, "html_inject_core_css", true, string_table)?,
        body_style: parse_required_string(config, "html_body_style", "", false, string_table)?,
    })
}

fn parse_required_string(
    config: &Config,
    key: &str,
    default: &str,
    reject_empty: bool,
    string_table: &mut StringTable,
) -> Result<String, ProjectConfigError> {
    let Some(raw_value) = config.settings.get(key) else {
        return Ok(default.to_string());
    };

    if reject_empty && raw_value.is_empty() {
        return Err(config_empty_error(config, key, string_table));
    }

    Ok(raw_value.to_owned())
}

fn parse_optional_string(
    config: &Config,
    key: &str,
    string_table: &mut StringTable,
) -> Result<Option<String>, ProjectConfigError> {
    let Some(raw_value) = config.settings.get(key) else {
        return Ok(None);
    };

    if raw_value.is_empty() {
        return Err(config_empty_error(config, key, string_table));
    }

    Ok(Some(raw_value.to_owned()))
}

fn parse_bool(
    config: &Config,
    key: &str,
    default: bool,
    string_table: &mut StringTable,
) -> Result<bool, ProjectConfigError> {
    let Some(raw_value) = config.settings.get(key) else {
        return Ok(default);
    };

    match raw_value.as_str() {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(config_value_error(
            config,
            key,
            raw_value,
            "'true' or 'false'",
            string_table,
        )),
    }
}

fn config_empty_error(
    config: &Config,
    key: &str,
    string_table: &mut StringTable,
) -> ProjectConfigError {
    config
        .config_diagnostic(key, InvalidConfigReason::EmptyProjectSetting, string_table)
        .into()
}

fn config_value_error(
    config: &Config,
    key: &str,
    value: &str,
    expected: &str,
    string_table: &mut StringTable,
) -> ProjectConfigError {
    let value = string_table.intern(value);
    let expected = string_table.intern(expected);
    config
        .config_diagnostic(
            key,
            InvalidConfigReason::InvalidProjectSettingValue { value, expected },
            string_table,
        )
        .into()
}

#[cfg(test)]
#[path = "tests/document_config_tests.rs"]
mod tests;
