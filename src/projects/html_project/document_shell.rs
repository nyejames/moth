//! Shared HTML document shell rendering.
//!
//! WHAT: merges builder config, page metadata, body HTML, and runtime script HTML into one final
//!       HTML document.
//! WHY: JS-only and HTML+Wasm outputs must share one shell policy so they cannot drift.
//! Rendered HTML fragments remain opaque during assembly so the shell never rewrites
//! whitespace-sensitive content such as code blocks, scripts, import maps, or head markup.

use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::folded_value::OwnedFoldedString;
use crate::projects::html_project::document_config::HtmlDocumentConfig;
use crate::projects::html_project::page_metadata::HtmlPageMetadata;
use crate::projects::html_project::structural_url_renderer::StructuralUrlRenderer;
use crate::timed_stage;
use std::fmt::Write as _;
use std::path::Path;
const CORE_CSS: &str = include_str!("moth-css-core.css");

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedHtmlDocument {
    pub lang: String,
    pub title: String,
    pub description: Option<String>,
    pub favicon: Option<String>,
    pub inject_charset: bool,
    pub inject_viewport: bool,
    pub inject_color_scheme: bool,
    pub head_html: String,
    pub body_style: String,
    pub body_html: String,
    pub script_html: String,
    pub core_css: Option<String>,
    pub import_map_html: Option<String>,
}

/// Inputs for one HTML document-shell render.
pub(crate) struct HtmlDocumentShellInput<'a> {
    pub config: &'a HtmlDocumentConfig,
    pub page_metadata: &'a HtmlPageMetadata,
    pub structural_url_renderer: &'a StructuralUrlRenderer<'a>,
    pub logical_html_path: &'a Path,
    pub project_name: &'a str,
    pub body_html: String,
    pub script_html: String,
    pub import_map_html: Option<String>,
}

pub(crate) fn render_html_document_shell(
    input: HtmlDocumentShellInput<'_>,
) -> Result<String, CompilerError> {
    timed_stage!(crate::timing::TimingMetric::BackendHtmlRender, {
        let resolved = resolve_html_document(input)?;

        Ok(render_resolved_document(&resolved))
    })
}

fn resolve_html_document(
    input: HtmlDocumentShellInput<'_>,
) -> Result<ResolvedHtmlDocument, CompilerError> {
    let HtmlDocumentShellInput {
        config,
        page_metadata,
        structural_url_renderer,
        logical_html_path,
        project_name,
        body_html,
        script_html,
        import_map_html,
    } = input;

    let mut base_title =
        render_metadata_value(page_metadata.title.as_ref(), structural_url_renderer)?;
    if base_title.is_none() {
        base_title = route_title_fallback(logical_html_path)?;
    }
    if base_title.is_none() && !project_name.is_empty() {
        base_title = Some(project_name.to_string());
    }
    let base_title = base_title.unwrap_or_default();

    let lang = render_metadata_value(page_metadata.lang.as_ref(), structural_url_renderer)?
        .unwrap_or_else(|| config.lang.clone());
    let description =
        render_metadata_value(page_metadata.description.as_ref(), structural_url_renderer)?;
    let favicon = render_metadata_value(page_metadata.favicon.as_ref(), structural_url_renderer)?
        .or_else(|| config.favicon.clone());
    let head_html = render_metadata_value(
        page_metadata.extra_head_html.as_ref(),
        structural_url_renderer,
    )?
    .unwrap_or_default();
    let body_style =
        render_metadata_value(page_metadata.body_style.as_ref(), structural_url_renderer)?
            .unwrap_or_else(|| config.body_style.clone());

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
        head_html,
        body_style,
        body_html,
        script_html,
        core_css: config.inject_core_css.then(|| CORE_CSS.to_string()),
        import_map_html,
    })
}

fn render_metadata_value(
    value: Option<&OwnedFoldedString>,
    structural_url_renderer: &StructuralUrlRenderer<'_>,
) -> Result<Option<String>, CompilerError> {
    value
        .map(|value| structural_url_renderer.render_owned(value))
        .transpose()
}

fn render_resolved_document(document: &ResolvedHtmlDocument) -> String {
    let mut html = String::new();
    html.push_str("<!DOCTYPE html>\n");
    let _ = writeln!(
        html,
        "<html lang=\"{}\">",
        escape_html_attribute(&document.lang)
    );
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
    let _ = writeln!(
        html,
        "    <title>{}</title>",
        escape_html_text(&document.title)
    );

    if let Some(description) = &document.description {
        let _ = writeln!(
            html,
            "    <meta name=\"description\" content=\"{}\">",
            escape_html_attribute(description)
        );
    }

    if let Some(favicon) = &document.favicon {
        let _ = writeln!(
            html,
            "    <link rel=\"icon\" href=\"{}\">",
            escape_html_attribute(favicon)
        );
    }

    if let Some(core_css) = &document.core_css {
        html.push_str("    <style>\n");
        html.push_str(core_css);
        if !core_css.ends_with('\n') {
            html.push('\n');
        }
        html.push_str("    </style>\n");
    }

    if let Some(import_map) = &document.import_map_html {
        append_rendered_html_fragment(&mut html, import_map);
    }

    if !document.head_html.is_empty() {
        append_rendered_html_fragment(&mut html, &document.head_html);
    }

    html.push_str("  </head>\n");
    let _ = writeln!(
        html,
        "  <body style=\"{}\">",
        escape_html_attribute(&document.body_style)
    );
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

/// Derive a human-readable title from the validated route path.
///
/// WHAT: returns the directory name for `index.html` routes or the file stem for flat routes,
/// formatted with spaces and title-cased word boundaries.
/// WHY: `logical_html_path` is built from already-validated route components, so a non-UTF-8
///      segment here breaks the validated-route contract and must surface as an internal error
///      rather than silently downgrading to a project-title fallback. A missing route segment
///      (root page) legitimately returns `None` so the caller can fall through to the project name.
fn route_title_fallback(logical_html_path: &Path) -> Result<Option<String>, CompilerError> {
    let route_segment = match extract_route_segment(logical_html_path)? {
        Some(segment) => segment,
        None => return Ok(None),
    };

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

/// Extract the raw route segment used for the title fallback.
///
/// WHAT: returns the directory name for `index.html` routes or the file stem for flat routes.
/// WHY: a missing segment (root page or path with no parent) legitimately yields `None`, but a
///      present non-UTF-8 segment breaks the validated-route contract and must surface as an
///      internal error rather than silently downgrading to a project-title fallback.
fn extract_route_segment(logical_html_path: &Path) -> Result<Option<String>, CompilerError> {
    let is_index_route =
        logical_html_path.file_name().and_then(|name| name.to_str()) == Some("index.html");

    if is_index_route {
        let Some(parent) = logical_html_path.parent() else {
            return Ok(None);
        };
        let Some(folder_name) = parent.file_name() else {
            return Ok(None);
        };
        let segment = folder_name.to_str().ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "HTML route directory component {parent:?} is not valid UTF-8; route components must be validated before title fallback."
            ))
        })?;
        return Ok(Some(segment.to_string()));
    }

    let Some(stem) = logical_html_path.file_stem() else {
        return Ok(None);
    };
    let segment = stem.to_str().ok_or_else(|| {
        CompilerError::compiler_error(format!(
            "HTML route stem component {logical_html_path:?} is not valid UTF-8; route components must be validated before title fallback."
        ))
    })?;
    Ok(Some(segment.to_string()))
}

fn escape_html_text(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn escape_html_attribute(value: &str) -> String {
    escape_html_text(value).replace('"', "&quot;")
}

#[cfg(test)]
#[path = "tests/document_shell_tests.rs"]
mod tests;
