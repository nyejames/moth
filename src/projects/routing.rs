//! Shared HTML page-routing policy parsing and defaults.
//!
//! WHAT: parses routing-related `config.moth` settings into typed values.
//! WHY: keeping one parser avoids drift between builder validation and dev-server runtime behavior.

use crate::compiler_frontend::compiler_messages::InvalidConfigReason;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::projects::settings::{Config, ProjectConfigError};

/// Canonical page URL style used for directory-backed HTML routes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageUrlStyle {
    /// `/about/` is canonical and `/about` redirects.
    TrailingSlash,
    /// `/about` is canonical and `/about/` redirects.
    NoTrailingSlash,
    /// Both `/about` and `/about/` are accepted without slash redirects.
    Ignore,
}

/// Effective HTML site configuration used by builders and the dev server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HtmlSiteConfig {
    pub origin: String,
    pub page_url_style: PageUrlStyle,
    pub redirect_index_html: bool,
}

impl Default for HtmlSiteConfig {
    fn default() -> Self {
        Self {
            origin: String::from("/"),
            page_url_style: PageUrlStyle::TrailingSlash,
            redirect_index_html: true,
        }
    }
}

/// Parse and validate HTML site config keys from the project config map.
///
/// WHAT: resolves defaults plus optional overrides from `config.moth`.
/// WHY: site configuration must be explicit and strict so all runtime/build tooling stays aligned.
pub fn parse_html_site_config(
    config: &Config,
    string_table: &mut StringTable,
) -> Result<HtmlSiteConfig, ProjectConfigError> {
    let origin = parse_origin(config, string_table)?;
    let page_url_style = parse_page_url_style(config, string_table)?;
    let redirect_index_html = parse_redirect_index_html(config, string_table)?;

    Ok(HtmlSiteConfig {
        origin,
        page_url_style,
        redirect_index_html,
    })
}

fn parse_origin(
    config: &Config,
    string_table: &mut StringTable,
) -> Result<String, ProjectConfigError> {
    let Some(raw_value) = config.settings.get("origin") else {
        return Ok(String::from("/"));
    };

    validate_origin(config, raw_value, string_table)?;

    Ok(raw_value.to_owned())
}

fn validate_origin(
    config: &Config,
    origin: &str,
    string_table: &mut StringTable,
) -> Result<(), ProjectConfigError> {
    if origin.is_empty() {
        return Err(config_empty_error(config, "origin", string_table));
    }

    if !origin.starts_with('/') {
        return Err(config_value_error(
            config,
            "origin",
            origin,
            "a path prefix that starts with '/', for example '/docs'",
            string_table,
        ));
    }

    if origin.len() > 1 && origin.ends_with('/') {
        return Err(config_value_error(
            config,
            "origin",
            origin,
            "either '/' or a path prefix without a trailing slash",
            string_table,
        ));
    }

    if origin.contains('?') || origin.contains('#') {
        return Err(config_value_error(
            config,
            "origin",
            origin,
            "only a path prefix, without query (?) or fragment (#) characters",
            string_table,
        ));
    }

    if origin.contains('\\') {
        return Err(config_value_error(
            config,
            "origin",
            origin,
            "a path prefix with forward slashes, for example '/docs'",
            string_table,
        ));
    }

    Ok(())
}

fn parse_page_url_style(
    config: &Config,
    string_table: &mut StringTable,
) -> Result<PageUrlStyle, ProjectConfigError> {
    let Some(raw_value) = config.settings.get("page_url_style") else {
        return Ok(PageUrlStyle::TrailingSlash);
    };

    match raw_value.as_str() {
        "trailing_slash" => Ok(PageUrlStyle::TrailingSlash),
        "no_trailing_slash" => Ok(PageUrlStyle::NoTrailingSlash),
        "ignore" => Ok(PageUrlStyle::Ignore),
        _ => Err(config_value_error(
            config,
            "page_url_style",
            raw_value,
            "'trailing_slash', 'no_trailing_slash', or 'ignore'",
            string_table,
        )),
    }
}

fn parse_redirect_index_html(
    config: &Config,
    string_table: &mut StringTable,
) -> Result<bool, ProjectConfigError> {
    let Some(raw_value) = config.settings.get("redirect_index_html") else {
        return Ok(true);
    };

    match raw_value.as_str() {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(config_value_error(
            config,
            "redirect_index_html",
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

/// WHAT: creates typed config diagnostics with precise setting locations when available.
/// WHY: project setting parsers should keep user-facing config errors out of `CompilerError`.
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

// --- Public Path Helpers ---

/// Prefixes a site-local path with the given origin.
pub fn prefix_origin(origin: &str, site_path: &str) -> String {
    if origin == "/" {
        return site_path.to_owned();
    }

    if site_path == "/" {
        return format!("{origin}/");
    }

    format!("{origin}{site_path}")
}

/// Strips the origin prefix from a public path, returning the site-local path.
pub fn strip_origin_prefix(public_path: &str, origin: &str) -> Option<String> {
    if origin == "/" {
        return Some(public_path.to_owned());
    }

    // Origin is like "/moth"
    // Public path must start with "/moth"
    if !public_path.starts_with(origin) {
        return None;
    }

    let stripped = &public_path[origin.len()..];

    // If public_path was "/moth", stripped is ""
    // If public_path was "/moth/", stripped is "/"
    // If public_path was "/moth/about", stripped is "/about"
    if stripped.is_empty() {
        return Some(String::from("/"));
    }

    if stripped.starts_with('/') {
        Some(stripped.to_owned())
    } else {
        // This handles cases like "/mothish" when origin is "/moth"
        None
    }
}

/// Returns the canonical public URL for the site root.
pub fn origin_root_url(origin: &str) -> String {
    if origin == "/" {
        String::from("/")
    } else {
        format!("{origin}/")
    }
}

#[cfg(test)]
#[path = "tests/routing_tests.rs"]
mod tests;
