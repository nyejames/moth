//! Dev-server HTML rendering for `CompilerDiagnostic`.
//!
//! WHAT: converts structured diagnostics into escaped HTML cards for the dev-server error page.
//! WHY: the dev-server needs clickable source links and readable diagnostic output.

use crate::compiler_frontend::compiler_errors::CompilerMessages;
use crate::compiler_frontend::compiler_messages::render::{
    DiagnosticRenderContext, display_column_number, display_line_number,
    relative_display_path_from_root, render_payload, resolve_source_file_path,
};
use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, DiagnosticSeverity};
use crate::compiler_frontend::utilities::basic::portable_path_text;
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
    let string_table = context.string_table;
    let resolved_path = resolve_source_file_path(&diagnostic.primary_location.scope, string_table);
    let display_root = match std::fs::canonicalize(project_root) {
        Ok(canonical_root) => canonical_root,
        Err(_) => project_root.to_path_buf(),
    };
    let relative_path = relative_display_path_from_root(&resolved_path, &display_root);
    let line = display_line_number(diagnostic.primary_location.start_pos.line_number);
    let column = display_column_number(diagnostic.primary_location.start_pos.char_column);

    // Use a simple file:// link to the resolved source path. The terminal
    // renderer works fine with this; browser-hosted dev-server links are a
    // follow-up once a cross-environment open strategy is settled.
    let file_href = format!(
        "file://{}",
        escape_html(&portable_path_text(&resolved_path))
    );

    // A missing retained snapshot omits the source excerpt without rereading the filesystem.
    let source_line = context
        .retained_source_line(
            &diagnostic.primary_location.scope,
            diagnostic.primary_location.start_pos.line_number,
        )
        .unwrap_or_default();

    let line_label = line.to_string();
    let gutter_padding = " ".repeat(3usize.saturating_sub(line_label.len()));
    let escaped_line = escape_html(source_line);

    if source_line.is_empty() {
        return format!(
            r#"<div class="source-frame"><a class="source-location" href="{file_href}">--> {relative_path}:{line}:{column}</a></div>"#
        );
    }

    // Underline the primary span with carets.
    let underline_start = diagnostic.primary_location.start_pos.char_column.max(0) as usize;
    let underline_length = (diagnostic.primary_location.end_pos.char_column
        - diagnostic.primary_location.start_pos.char_column
        + 1)
    .max(1) as usize;
    let padding = " ".repeat(underline_start);
    let underlines = "^".repeat(underline_length);

    format!(
        r#"<div class="source-frame"><a class="source-location" href="{file_href}">--> {relative_path}:{line}:{column}</a><br><span class="source-line-number">{gutter_padding}{line_label} | </span><span class="source-line">{escaped_line}</span><br><span class="source-line-number">{gutter_padding}  | </span><span class="source-caret">{padding}{underlines}</span></div>"#
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
    if messages.diagnostic_slice().is_empty() {
        return String::from("<p>No compiler diagnostics available.</p>");
    }

    messages
        .diagnostic_display_order()
        .into_iter()
        .map(|diagnostic_index| {
            render_diagnostic_card(
                &messages.diagnostic_slice()[diagnostic_index],
                project_root,
                messages.diagnostic_render_context(diagnostic_index),
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
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
