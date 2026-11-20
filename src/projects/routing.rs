//! Shared HTML page-routing policy parsing and defaults.
//!
//! WHAT: parses routing-related `@config.moth` settings into typed values.
//! WHY: keeping one parser avoids drift between builder validation and dev-server runtime behavior.

use crate::build_system::output::percent_encode_url_path;
use crate::compiler_frontend::compiler_messages::InvalidConfigReason;
use crate::compiler_frontend::compiler_messages::diagnostics::diagnostic_string_builder::DiagnosticStringBuilder;
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
    /// Canonical, percent-encoded public path prefix, or `/` for the site root.
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

/// Parse and validate HTML site config keys from the generic builder settings.
///
/// WHAT: resolves defaults plus optional overrides authored on the grouped `html` section.
/// WHY: site configuration must be explicit and strict so all runtime/build tooling stays aligned.
pub fn parse_html_site_config(
    config: &Config,
    string_table: &mut DiagnosticStringBuilder,
) -> Result<HtmlSiteConfig, ProjectConfigError> {
    let origin = parse_origin(config, string_table)?;
    let page_url_style = parse_page_url_style(config, string_table)?;
    let redirect_index_html = config
        .builder_settings
        .bool("html", "redirect_index_html")
        .unwrap_or(true);

    Ok(HtmlSiteConfig {
        origin,
        page_url_style,
        redirect_index_html,
    })
}

fn parse_origin(
    config: &Config,
    string_table: &mut DiagnosticStringBuilder,
) -> Result<String, ProjectConfigError> {
    let Some(raw_value) = config.builder_settings.string("html", "origin") else {
        return Ok(String::from("/"));
    };

    validate_origin(config, raw_value, string_table)?;

    Ok(percent_encode_url_path(raw_value))
}

fn validate_origin(
    config: &Config,
    origin: &str,
    string_table: &mut DiagnosticStringBuilder,
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

    if origin != "/"
        && origin[1..].split('/').any(|segment| {
            segment.is_empty()
                || matches!(segment, "." | "..")
                || segment.chars().any(char::is_control)
        })
    {
        return Err(config_value_error(
            config,
            "origin",
            origin,
            "a path prefix with nonempty segments and no dot segments or control characters",
            string_table,
        ));
    }

    Ok(())
}

fn parse_page_url_style(
    config: &Config,
    string_table: &mut DiagnosticStringBuilder,
) -> Result<PageUrlStyle, ProjectConfigError> {
    let Some(raw_value) = config.builder_settings.string("html", "page_url_style") else {
        return Ok(PageUrlStyle::TrailingSlash);
    };

    match raw_value {
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

fn config_empty_error(
    config: &Config,
    key: &str,
    string_table: &mut DiagnosticStringBuilder,
) -> ProjectConfigError {
    config
        .builder_config_diagnostic(
            "html",
            key,
            InvalidConfigReason::EmptyProjectSetting,
            string_table,
        )
        .into()
}

/// WHAT: creates typed config diagnostics with precise authored spans when available.
/// WHY: project setting parsers should keep user-facing config errors out of `InfrastructureFailure`.
fn config_value_error(
    config: &Config,
    key: &str,
    value: &str,
    expected: &str,
    string_table: &mut DiagnosticStringBuilder,
) -> ProjectConfigError {
    let value = string_table.intern(value);
    let expected = string_table.intern(expected);
    config
        .builder_config_diagnostic(
            "html",
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

    let (prefix, stripped) = public_path.split_at_checked(origin.len())?;
    if prefix != origin && !matches_origin_escape_spelling(prefix, origin) {
        return None;
    }

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

/// Returns whether the request addresses the origin root without its trailing slash.
///
/// The dev server redirects this form so relative links resolve under the origin instead of
/// above it. An origin of `/` is already the canonical root, so it never matches and cannot
/// redirect to itself. Origin matching keeps the same percent-escape spelling rules as
/// `strip_origin_prefix`.
pub fn is_bare_origin_request(request_path: &str, origin: &str) -> bool {
    origin != "/"
        && request_path.len() == origin.len()
        && matches_origin_escape_spelling(request_path, origin)
}

/// Percent escapes may use lowercase hex, but literal path bytes are case-sensitive.
/// Never decode here: an escaped slash must not become a prefix separator.
fn matches_origin_escape_spelling(request: &str, origin: &str) -> bool {
    let mut request = request.bytes();
    let mut origin = origin.bytes();
    while let Some(expected) = origin.next() {
        if request.next() != Some(expected) {
            return false;
        }
        if expected == b'%' {
            for _ in 0..2 {
                if request.next().map(|byte| byte.to_ascii_uppercase()) != origin.next() {
                    return false;
                }
            }
        }
    }
    true
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
