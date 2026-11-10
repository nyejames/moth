//! Shared HTML document shell rendering.
//!
//! WHAT: merges builder config, slot-planned page metadata, body HTML, and runtime script HTML
//!       into one final HTML document.
//! WHY: JS-only and HTML+Wasm outputs must share one shell policy so they cannot drift.
//! Rendered HTML fragments remain opaque during assembly so the shell never rewrites
//! whitespace-sensitive content such as code blocks, scripts, import maps, or head markup.

use crate::compiler_frontend::ast::templates::styles::escape_html::push_escaped_html_text;
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::projects::html_project::document_config::HtmlDocumentConfig;
use crate::projects::html_project::html_slot_plan::HtmlSlotPlan;
use crate::projects::html_project::output_plan::CanonicalPageRoute;
use crate::timed_stage;
use std::ffi::OsStr;
const CORE_CSS: &str = include_str!("moth-css-core.css");

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedHtmlDocument<'a> {
    pub lang: &'a str,
    pub title: String,
    pub description: Option<&'a str>,
    pub favicon: Option<&'a str>,
    pub inject_charset: bool,
    pub inject_viewport: bool,
    pub inject_color_scheme: bool,
    pub stylesheet_hrefs: &'a [String],
    pub head_html: &'a str,
    pub body_style: &'a str,
    pub body_html: String,
    pub script_html: String,
    pub core_css: Option<&'static str>,
    pub import_map_html: Option<String>,
}

/// Inputs for one HTML document-shell render.
pub(crate) struct HtmlDocumentShellInput<'a> {
    pub config: &'a HtmlDocumentConfig,
    pub slot_plan: &'a HtmlSlotPlan,
    pub route: &'a CanonicalPageRoute,
    pub project_name: &'a str,
    pub body_html: String,
    pub script_html: String,
    pub import_map_html: Option<String>,
}

pub(crate) fn render_html_document_shell(
    input: HtmlDocumentShellInput<'_>,
) -> Result<String, InfrastructureFailure> {
    timed_stage!(crate::timing::TimingMetric::BackendHtmlRender, {
        let resolved = resolve_html_document(input)?;

        Ok(render_resolved_document(&resolved))
    })
}

fn resolve_html_document<'a>(
    input: HtmlDocumentShellInput<'a>,
) -> Result<ResolvedHtmlDocument<'a>, InfrastructureFailure> {
    let HtmlDocumentShellInput {
        config,
        slot_plan,
        route,
        project_name,
        body_html,
        script_html,
        import_map_html,
    } = input;
    let metadata = &slot_plan.metadata;

    // An explicitly empty contribution is supplied metadata, not an absent slot, so fallbacks
    // apply only while the planned value remains `None`.
    let fallback_title;
    let base_title = match metadata.title.as_deref() {
        Some(title) => title,
        None => {
            fallback_title = route_title_fallback(route.route_segment.as_deref())?;
            fallback_title.as_deref().unwrap_or(project_name)
        }
    };

    let lang = metadata.lang.as_deref().unwrap_or(&config.lang);
    let description = metadata.description.as_deref();
    let favicon = metadata.favicon.as_deref().or(config.favicon.as_deref());
    let body_style = metadata.body_style.as_deref().unwrap_or(&config.body_style);

    Ok(ResolvedHtmlDocument {
        lang,
        title: format!(
            "{}{}{}",
            config.title_prefix, base_title, config.title_postfix
        ),
        description,
        favicon,
        inject_charset: config.inject_charset,
        inject_viewport: config.inject_viewport,
        inject_color_scheme: config.inject_color_scheme,
        stylesheet_hrefs: &slot_plan.stylesheet_hrefs,
        head_html: &slot_plan.extra_head_html,
        body_style,
        body_html,
        script_html,
        core_css: config.inject_core_css.then_some(CORE_CSS),
        import_map_html,
    })
}

fn render_resolved_document(document: &ResolvedHtmlDocument<'_>) -> String {
    let mut html = String::new();
    html.push_str("<!DOCTYPE html>\n");

    html.push_str("<html lang=\"");
    push_escaped_html_text(&mut html, document.lang);
    html.push_str("\">\n");

    html.push_str("  <head>\n");
    if document.inject_charset {
        html.push_str("    <meta charset=\"UTF-8\">\n");
    }
    if document.inject_viewport {
        html.push_str(
            "    <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n",
        );
    }
    if document.inject_color_scheme {
        html.push_str("    <meta name=\"color-scheme\" content=\"light dark\">\n");
    }

    html.push_str("    <title>");
    push_escaped_html_text(&mut html, &document.title);
    html.push_str("</title>\n");

    if let Some(description) = document.description {
        html.push_str("    <meta name=\"description\" content=\"");
        push_escaped_html_text(&mut html, description);
        html.push_str("\">\n");
    }

    if let Some(favicon) = document.favicon {
        html.push_str("    <link rel=\"icon\" href=\"");
        push_escaped_html_text(&mut html, favicon);
        html.push_str("\">\n");
    }

    if let Some(core_css) = document.core_css {
        html.push_str("    <style>\n");
        html.push_str(core_css);
        if !core_css.ends_with('\n') {
            html.push('\n');
        }
        html.push_str("    </style>\n");
    }

    for href in document.stylesheet_hrefs {
        html.push_str("    <link rel=\"stylesheet\" href=\"");
        push_escaped_html_text(&mut html, href);
        html.push_str("\">\n");
    }

    if let Some(import_map) = &document.import_map_html {
        append_rendered_html_fragment(&mut html, import_map);
    }

    if !document.head_html.is_empty() {
        append_rendered_html_fragment(&mut html, document.head_html);
    }

    html.push_str("  </head>\n");
    html.push_str("  <body style=\"");
    push_escaped_html_text(&mut html, document.body_style);
    html.push_str("\">\n");
    if !document.body_html.is_empty() {
        append_rendered_html_fragment(&mut html, &document.body_html);
    }
    if !document.script_html.is_empty() {
        append_rendered_html_fragment(&mut html, &document.script_html);
    }
    if !html.ends_with('\n') {
        html.push('\n');
    }
    html.push_str("  </body>\n");
    html.push_str("</html>\n");

    html
}

fn append_rendered_html_fragment(output: &mut String, fragment: &str) {
    output.push_str(fragment);

    if !fragment.ends_with('\n') {
        output.push('\n');
    }
}

/// Format the planned route segment for the page-title fallback.
///
/// WHAT: turns `-`, `_` and `/` boundaries into spaces and title-cases each following word.
/// WHY: route semantics are owned by `CanonicalPageRoute`; this function only projects its raw
///      segment into display text and never inspects a path.
fn route_title_fallback(
    route_segment: Option<&OsStr>,
) -> Result<Option<String>, InfrastructureFailure> {
    let Some(route_segment) = route_segment else {
        return Ok(None);
    };
    let route_segment = route_segment.to_str().ok_or_else(|| {
        InfrastructureFailure::internal(CompilerStage::ProjectBuildAssembly, format!(
            "HTML planned route segment {route_segment:?} is not valid UTF-8; route components must be validated before title fallback."
        ))
    })?;

    if route_segment.is_empty() {
        return Ok(None);
    }

    let mut formatted = String::with_capacity(route_segment.len());
    let mut uppercase_next = true;
    for ch in route_segment.chars() {
        if matches!(ch, '-' | '_' | '/') {
            formatted.push(' ');
            uppercase_next = true;
            continue;
        }

        if uppercase_next {
            for upper in ch.to_uppercase() {
                formatted.push(upper);
            }
            uppercase_next = false;
        } else {
            formatted.push(ch);
        }
    }

    Ok(Some(formatted))
}

#[cfg(test)]
#[path = "tests/document_shell_tests.rs"]
mod tests;
