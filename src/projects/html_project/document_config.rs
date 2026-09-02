//! Typed HTML document-shell configuration parsing.
//!
//! WHAT: parses HTML-shell-specific `config.moth` settings from the typed html builder
//!       section into a strict typed struct.
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
    let section = &config.html_section;

    Ok(HtmlDocumentConfig {
        lang: parse_required_string(
            section.html_lang.as_deref(),
            "html_lang",
            "en",
            true,
            config,
            string_table,
        )?,
        title_prefix: parse_required_string(
            section.html_title_prefix.as_deref(),
            "html_title_prefix",
            "",
            false,
            config,
            string_table,
        )?,
        title_postfix: parse_required_string(
            section.html_title_postfix.as_deref(),
            "html_title_postfix",
            "",
            false,
            config,
            string_table,
        )?,
        favicon: parse_optional_string(
            section.html_favicon.as_deref(),
            "html_favicon",
            config,
            string_table,
        )?,
        inject_charset: section.html_inject_charset.unwrap_or(true),
        inject_viewport: section.html_inject_viewport.unwrap_or(true),
        inject_color_scheme: section.html_inject_color_scheme.unwrap_or(true),
        inject_core_css: section.html_inject_core_css.unwrap_or(true),
        body_style: parse_required_string(
            section.html_body_style.as_deref(),
            "html_body_style",
            "",
            false,
            config,
            string_table,
        )?,
    })
}

fn parse_required_string(
    raw_value: Option<&str>,
    key: &str,
    default: &str,
    reject_empty: bool,
    config: &Config,
    string_table: &mut StringTable,
) -> Result<String, ProjectConfigError> {
    let Some(raw_value) = raw_value else {
        return Ok(default.to_string());
    };

    if reject_empty && raw_value.is_empty() {
        return Err(config_empty_error(config, key, string_table));
    }

    Ok(raw_value.to_owned())
}

fn parse_optional_string(
    raw_value: Option<&str>,
    key: &str,
    config: &Config,
    string_table: &mut StringTable,
) -> Result<Option<String>, ProjectConfigError> {
    let Some(raw_value) = raw_value else {
        return Ok(None);
    };

    if raw_value.is_empty() {
        return Err(config_empty_error(config, key, string_table));
    }

    Ok(Some(raw_value.to_owned()))
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

#[cfg(test)]
#[path = "tests/document_config_tests.rs"]
mod tests;
