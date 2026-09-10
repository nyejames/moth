//! Dev-server HTML rendering for `CompilerDiagnostic`.
//!
//! WHAT: converts structured diagnostics into escaped HTML cards for the dev-server error page.
//! WHY: the dev-server needs clickable source links and readable diagnostic output.

use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages};
use crate::compiler_frontend::compiler_messages::render::{
    DiagnosticRenderContext, ResolvedDiagnosticLabel, display_column_number, display_gutter_width,
    display_line_number, expand_tabs_for_display, primary_caret_padding, primary_underline_length,
    relative_display_path_from_root, render_payload, resolve_label_render_facts_from_root,
};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticLabelStyle, DiagnosticSeverity,
};
use crate::compiler_frontend::utilities::basic::{normalize_path, portable_path_text};
#[cfg(test)]
use std::path::Path;

fn severity_display(severity: DiagnosticSeverity) -> (&'static str, &'static str, &'static str) {
    match severity {
        DiagnosticSeverity::Error => (
            "Error",
            "(\u{256F}\u{00B0}\u{25A1}\u{00B0})\u{256F} \u{1F525}",
            "badge",
        ),
        DiagnosticSeverity::Warning => ("Warning", "\u{26A0}\u{FE0F}", "badge warning"),
        DiagnosticSeverity::Note => ("Note", "\u{1F4DD}", "badge info"),
    }
}

fn render_source_frame(
    diagnostic: &CompilerDiagnostic,
    project_root: &std::path::Path,
    context: DiagnosticRenderContext<'_>,
) -> String {
    let Some(primary_position) = context.primary_position(diagnostic) else {
        return String::new();
    };

    let display_root = normalize_path(project_root);
    let relative_path =
        relative_display_path_from_root(primary_position.path.as_path(), &display_root);
    let escaped_relative_path = escape_html(&relative_path);
    let line = display_line_number(i32::try_from(primary_position.start.line).unwrap_or(i32::MAX));
    let column =
        display_column_number(i32::try_from(primary_position.start.column).unwrap_or(i32::MAX));
    let source_line = primary_position.line;
    let line_label = line.to_string();
    let gutter_width = display_gutter_width(line);
    let gutter_padding = " ".repeat(gutter_width.saturating_sub(line_label.len()));
    let empty_line_label = " ".repeat(line_label.len());
    let escaped_line = escape_html(&expand_tabs_for_display(source_line));

    let location = match primary_position.host_path {
        Some(host_path) => format!(
            r#"<a class="source-location" href="file://{}">--> {}:{}:{}</a>"#,
            escape_html(&portable_path_text(host_path)),
            escaped_relative_path,
            line,
            column,
        ),
        None => format!(
            r#"<span class="source-location">--> {}:{}:{}</span>"#,
            escaped_relative_path, line, column
        ),
    };

    if source_line.is_empty() {
        return format!(r#"<div class="source-frame">{location}</div>"#);
    }

    let underline_start = primary_caret_padding(&primary_position, source_line);
    let underline_length = primary_underline_length(&primary_position, source_line);
    let padding = " ".repeat(underline_start);
    let underlines = "^".repeat(underline_length);

    format!(
        r#"<div class="source-frame">{location}<br><span class="source-line-number">{gutter_padding}{line_label} | </span><span class="source-line">{escaped_line}</span><br><span class="source-line-number">{gutter_padding}{empty_line_label} | </span><span class="source-caret">{padding}{underlines}</span></div>"#
    )
}

#[cfg(test)]
pub(crate) fn render_diagnostics_html_with_context(
    diagnostics: &[CompilerDiagnostic],
    project_root: &Path,
    context: DiagnosticRenderContext<'_>,
) -> String {
    if diagnostics.is_empty() {
        return String::from("<p>No compiler diagnostics available.</p>");
    }

    diagnostics
        .iter()
        .map(|d| render_diagnostic_card(d, project_root, context))
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn render_compiler_messages_html(
    messages: &CompilerMessages,
    project_root: &std::path::Path,
) -> String {
    // The outer infrastructure failure renders as an error card beside the diagnostics,
    // after every error diagnostic and before warnings/notes — mirroring the severity-bucket
    // display order without fabricating a user diagnostic.
    let mut cards = Vec::new();
    let mut outer_emitted = messages.infrastructure_error().is_none();
    for diagnostic_index in messages.diagnostic_display_order() {
        let diagnostic = &messages.diagnostic_slice()[diagnostic_index];
        if !outer_emitted && diagnostic.severity != DiagnosticSeverity::Error {
            if let Some(error) = messages.infrastructure_error() {
                cards.push(render_compiler_error_card(error));
            }
            outer_emitted = true;
        }
        cards.push(render_diagnostic_card(
            diagnostic,
            project_root,
            messages.diagnostic_render_context(diagnostic_index),
        ));
    }
    if !outer_emitted && let Some(error) = messages.infrastructure_error() {
        cards.push(render_compiler_error_card(error));
    }
    if cards.is_empty() {
        return String::from("<p>No compiler diagnostics available.</p>");
    }
    cards.join("\n")
}

/// Render the outer infrastructure failure as an error card.
///
/// Infrastructure failures carry no retained source snapshot at this boundary. Only their host
/// path, message and structured guidance are rendered; no source frame is synthesized.
fn render_compiler_error_card(error: &CompilerError) -> String {
    let (severity_label, severity_visual, badge_class) =
        severity_display(DiagnosticSeverity::Error);

    let mut body = String::new();
    body.push_str(&format!(
        r#"<p class="diagnostic-message">{}</p>"#,
        escape_html(&error.msg)
    ));
    if let Some(host_path) = error.host_path.as_deref() {
        body.push_str(&format!(
            r#"<p class="source-path">{}</p>"#,
            escape_html(&portable_path_text(host_path))
        ));
    }
    for guidance in
        crate::compiler_frontend::compiler_messages::display_messages::format_error_guidance_lines(
            error,
        )
    {
        body.push_str(&format!(
            r#"<p class="guidance">Hint: {}</p>"#,
            escape_html(&guidance)
        ));
    }

    format!(
        r#"<article class="diagnostic" data-diagnostic-code="MOTH-INFRA-0001"><div class="diagnostic-head"><span class="{badge_class}">{severity_visual} {severity_label}</span><span class="kind">Infrastructure failure</span></div>{body}</article>"#
    )
}
fn render_diagnostic_card(
    diagnostic: &CompilerDiagnostic,
    project_root: &std::path::Path,
    context: DiagnosticRenderContext<'_>,
) -> String {
    let descriptor = diagnostic.kind.descriptor();
    let (severity_label, severity_visual, badge_class) = severity_display(diagnostic.severity);

    // The code is available as a data attribute for debugging but not shown
    // visibly in the browser card. Terse/terminal output still shows codes.
    let data_code = format!(
        r#" data-diagnostic-code="{}""#,
        escape_html(descriptor.code)
    );

    let source_frame = render_source_frame(diagnostic, project_root, context);

    let rendered_payload = render_payload(&diagnostic.payload, context);

    let mut body = String::new();

    if !rendered_payload.message.is_empty() {
        body.push_str(&format!(
            r#"<p class="diagnostic-message">{}</p>"#,
            escape_html(&rendered_payload.message)
        ));
    }

    for guidance in &rendered_payload.guidance {
        body.push_str(&format!(
            r#"<p class="guidance">Hint: {}</p>"#,
            escape_html(guidance)
        ));
    }

    body.push_str(&source_frame);

    let primary_display_path = context
        .primary_position(diagnostic)
        .map(|position| relative_display_path_from_root(position.path.as_path(), project_root));
    for ResolvedDiagnosticLabel {
        style,
        path,
        line,
        column,
        message,
    } in resolve_label_render_facts_from_root(diagnostic, context, project_root)
    {
        let style_name = match style {
            DiagnosticLabelStyle::Secondary => "info",
        };
        let escaped_message = escape_html(&message);
        let rendered_label = match (path.as_deref(), line, column) {
            (None, None, None) => format!("{style_name}: - {escaped_message}"),
            (Some(label_path), Some(label_line), Some(label_column)) => {
                let include_path = primary_display_path
                    .as_deref()
                    .is_none_or(|primary_path| primary_path != label_path);
                let location = if include_path && !label_path.is_empty() {
                    format!("{}:{label_line}:{label_column}", escape_html(label_path))
                } else {
                    format!("{label_line}:{label_column}")
                };
                format!("{style_name}: {location} - {escaped_message}")
            }
            _ => continue,
        };
        body.push_str(&format!(
            r#"<p class="diagnostic-label">{rendered_label}</p>"#
        ));
    }
    format!(
        r#"<article class="diagnostic"{data_code}><div class="diagnostic-head"><span class="{badge_class}">{severity_visual} {severity_label}</span><span class="kind">{title}</span></div>{body}</article>"#,
        title = escape_html(descriptor.title),
    )
}

fn escape_html(input: &str) -> String {
    let mut escaped = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            _ => escaped.push(ch),
        }
    }
    escaped
}
