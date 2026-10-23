//! Dev-server HTML rendering for frozen diagnostic views.
//!
//! WHAT: converts structured diagnostics into escaped HTML cards for the dev-server error page.
//! WHY: the dev-server needs clickable source links and readable diagnostic output.

use crate::compiler_frontend::compiler_errors::{BoundaryFailure, InfrastructureErrors};
#[cfg(test)]
use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
#[cfg(test)]
use crate::compiler_frontend::compiler_messages::diagnostics::legacy_adapter::compact_render_for_test;
use crate::compiler_frontend::compiler_messages::diagnostics::view::DiagnosticView;
use crate::compiler_frontend::compiler_messages::render::infrastructure::{
    INFRASTRUCTURE_FAILURE_CODE, INFRASTRUCTURE_FAILURE_TITLE, ISSUE_URL, InfrastructureClass,
    format_error_guidance_lines, infrastructure_class, infrastructure_heading,
};
use crate::compiler_frontend::compiler_messages::render::subject::DiagnosticSubject;
use crate::compiler_frontend::compiler_messages::render::{
    DiagnosticRenderContext, ResolvedDiagnosticLabel, display_column_number, display_gutter_width,
    display_line_number, expand_tabs_for_display, primary_caret_padding, primary_underline_length,
    relative_display_path_from_root,
};
use crate::compiler_frontend::compiler_messages::{DiagnosticLabelStyle, DiagnosticSeverity};
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
    subject: &DiagnosticSubject<'_>,
    project_root: &std::path::Path,
    context: DiagnosticRenderContext<'_>,
) -> String {
    let Some(primary_position) = subject.primary_position(context) else {
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
        .map(|d| {
            let compact = compact_render_for_test(d);
            render_diagnostic_card(compact.view(), project_root, context)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Renders every record of a boundary failure as an HTML card. The infrastructure card follows the error cards and
/// precedes the warnings and notes.
pub(crate) fn render_boundary_messages_html(
    failure: &BoundaryFailure,
    project_root: &std::path::Path,
) -> String {
    let mut cards = Vec::new();
    let mut infrastructure = failure.infrastructure_errors();
    for severity in [
        DiagnosticSeverity::Error,
        DiagnosticSeverity::Warning,
        DiagnosticSeverity::Note,
    ] {
        if severity != DiagnosticSeverity::Error
            && let Some(errors) = infrastructure.take()
        {
            cards.push(render_infrastructure_card(errors));
        }
        for (report, view) in failure.reports.iter_severity(severity) {
            cards.push(render_diagnostic_card(
                view,
                project_root,
                DiagnosticRenderContext::for_report(report),
            ));
        }
    }
    if let Some(errors) = infrastructure.take() {
        cards.push(render_infrastructure_card(errors));
    }
    if cards.is_empty() {
        return String::from("<p>No compiler diagnostics available.</p>");
    }
    cards.join("\n")
}

/// Keep all causes inside one infrastructure event, without resolving unpaired source IDs.
fn render_infrastructure_card(errors: &InfrastructureErrors) -> String {
    let (severity_label, severity_visual, badge_class) =
        severity_display(DiagnosticSeverity::Error);
    let mut body = String::new();
    for (index, error) in errors.iter().enumerate() {
        body.push_str(r#"<section class="infrastructure-cause">"#);
        let heading = infrastructure_heading(&error.error_type);
        if index == 0 {
            body.push_str(&format!("<h3>{heading}</h3>"));
        } else {
            body.push_str(&format!("<h3>Additional cause {index}: {heading}</h3>"));
        }
        let class = infrastructure_class(&error.error_type);
        body.push_str(&format!(
            r#"<p class="infrastructure-introduction">{}</p>"#,
            escape_html(class.introduction())
        ));
        if class == InfrastructureClass::CompilerBug {
            body.push_str(&format!(r#"<p><a href="{ISSUE_URL}">{ISSUE_URL}</a></p>"#));
        }
        body.push_str(&format!(
            r#"<p>Details:</p><p class="diagnostic-message">{}</p>"#,
            escape_html(&error.msg)
        ));
        if let Some(host_path) = error.host_path.as_deref() {
            body.push_str(&format!(
                r#"<p class="source-path">{}</p>"#,
                escape_html(&portable_path_text(host_path))
            ));
        }
        for guidance in format_error_guidance_lines(error) {
            body.push_str(&format!(
                r#"<p class="guidance">Hint: {}</p>"#,
                escape_html(&guidance)
            ));
        }
        body.push_str("</section>");
    }
    format!(
        r#"<article class="diagnostic" data-diagnostic-code="{INFRASTRUCTURE_FAILURE_CODE}"><div class="diagnostic-head"><span class="{badge_class}">{severity_visual} {severity_label}</span><span class="kind">{INFRASTRUCTURE_FAILURE_TITLE}</span></div>{body}</article>"#
    )
}
fn render_diagnostic_card(
    view: DiagnosticView<'_>,
    project_root: &std::path::Path,
    context: DiagnosticRenderContext<'_>,
) -> String {
    let subject = DiagnosticSubject::new(view);
    let descriptor = subject.descriptor();
    let (severity_label, severity_visual, badge_class) = severity_display(subject.severity());

    // The code is available as a data attribute for debugging but not shown
    // visibly in the browser card. Terse/terminal output still shows codes.
    let data_code = format!(
        r#" data-diagnostic-code="{}""#,
        escape_html(descriptor.code)
    );

    let source_frame = render_source_frame(&subject, project_root, context);

    let rendered_payload = subject.payload(context);

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

    let primary_display_path = subject
        .primary_position(context)
        .map(|position| relative_display_path_from_root(position.path.as_path(), project_root));
    for ResolvedDiagnosticLabel {
        style,
        path,
        line,
        column,
        message,
    } in subject.resolved_labels(context, project_root)
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
                // A location-only label carries an empty message, so the separator is omitted
                // rather than trailing the location.
                if message.is_empty() {
                    format!("{style_name}: {location}")
                } else {
                    format!("{style_name}: {location} - {escaped_message}")
                }
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
