//! Terminal rendering for frozen diagnostic views.
//!
//! WHAT: converts structured diagnostics into coloured terminal output.
//! WHY: this is the primary human-facing render path for compiler errors and warnings.

#[cfg(test)]
use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
#[cfg(test)]
use crate::compiler_frontend::compiler_messages::diagnostics::legacy_adapter::compact_render_for_test;
use crate::compiler_frontend::compiler_messages::diagnostics::view::DiagnosticView;
use crate::compiler_frontend::compiler_messages::render::subject::DiagnosticSubject;
use crate::compiler_frontend::compiler_messages::render::{
    DiagnosticPrimaryPosition, DiagnosticRenderContext, ResolvedDiagnosticLabel,
    display_column_number, display_gutter_width, display_line_number, expand_tabs_for_display,
    primary_caret_padding, primary_underline_length, relative_display_path_from_root,
};
use crate::compiler_frontend::compiler_messages::{DiagnosticLabelStyle, DiagnosticSeverity};
use saying::say;
use std::path::Path;

pub(crate) fn print_diagnostic_view(
    view: DiagnosticView<'_>,
    context: DiagnosticRenderContext<'_>,
) {
    let subject = DiagnosticSubject::new(view);
    let descriptor = subject.descriptor();
    let severity = subject.severity();
    let severity_name = severity_display_name(severity);
    let visual = severity_visual(severity);

    match severity {
        DiagnosticSeverity::Error => {
            say!("\n", Bright Bold Red severity_name, Reset " ", Reset visual);
        }
        DiagnosticSeverity::Warning => {
            say!("\n", Bright Bold Yellow severity_name, Reset " ", Reset visual);
        }
        DiagnosticSeverity::Note => {
            say!("\n", Bright Bold Blue severity_name, Reset " ", Reset visual);
        }
    }

    say!(Reset descriptor.title);
    say!(Dark "  [", descriptor.code, "]");

    let primary_position = subject.primary_position(context);
    if let Some(position) = primary_position.as_ref() {
        let relative_dir = relative_display_path_from_root(
            position.path.as_path(),
            &std::env::current_dir().unwrap_or_default(),
        );
        let display_line =
            display_line_number(i32::try_from(position.start.line).unwrap_or(i32::MAX));
        let display_column =
            display_column_number(i32::try_from(position.start.column).unwrap_or(i32::MAX));

        if !relative_dir.is_empty() {
            say!(
                Blue "\n  --> ",
                Reset Magenta relative_dir.as_str(),
                Dark Magenta ":",
                Reset Bold Blue display_line,
                Reset Grey ":",
                Reset Magenta display_column
            );
        } else {
            say!(
                Blue "\n   --> ",
                Reset Magenta display_line,
                Dark Magenta ":",
                Reset Magenta display_column
            );
        }

        if let Some(frame) = terminal_source_frame(position) {
            say!(Blue frame.gutter_marker.as_str(), "|");
            say!(
                Blue frame.line_padding.as_str(),
                Bold Blue frame.line_label.as_str(),
                " | ",
                Reset frame.line_text.as_str()
            );
            print!("{}", frame.caret_padding);
            say!(Red frame.carets.as_str());
        }
    }

    let root = std::env::current_dir().unwrap_or_default();
    for label_message in format_label_messages_from_subject(&subject, context, &root) {
        say!(Bright Blue "  ", label_message);
    }

    for guidance in format_guidance_from_subject(&subject, context) {
        say!(Bright Blue "  ", guidance);
    }
}

struct TerminalSourceFrame {
    gutter_marker: String,
    line_padding: String,
    line_label: String,
    line_text: String,
    caret_padding: String,
    carets: String,
}

fn terminal_source_frame(position: &DiagnosticPrimaryPosition<'_>) -> Option<TerminalSourceFrame> {
    let source_line = position.line;
    if source_line.is_empty() {
        return None;
    }

    let display_line = display_line_number(i32::try_from(position.start.line).unwrap_or(i32::MAX));
    let line_label = display_line.to_string();
    let gutter_width = display_gutter_width(display_line);
    let line_padding = " ".repeat(gutter_width.saturating_sub(line_label.len()));
    let caret_padding = " ".repeat(gutter_width + 3);
    let underline_start = primary_caret_padding(position, source_line);
    let underline_length = primary_underline_length(position, source_line);

    Some(TerminalSourceFrame {
        gutter_marker: " ".repeat(gutter_width + 1),
        line_padding,
        line_label,
        line_text: expand_tabs_for_display(source_line),
        caret_padding: format!("{caret_padding}{:width$}", "", width = underline_start),
        carets: "^".repeat(underline_length),
    })
}

#[cfg(test)]
pub(crate) fn format_terminal_source_frame_for_test(
    diagnostic: &CompilerDiagnostic,
    context: DiagnosticRenderContext<'_>,
) -> Option<String> {
    let compact = compact_render_for_test(diagnostic, context);
    let subject = DiagnosticSubject::new(compact.view());
    let position = subject.primary_position(context)?;
    let frame = terminal_source_frame(&position)?;
    Some(format!(
        "{}|\n{}{} | {}\n{}{}",
        frame.gutter_marker,
        frame.line_padding,
        frame.line_label,
        frame.line_text,
        frame.caret_padding,
        frame.carets,
    ))
}

#[cfg(test)]
pub(crate) fn format_label_messages_with_context(
    diagnostic: &CompilerDiagnostic,
    context: DiagnosticRenderContext<'_>,
) -> Vec<String> {
    let compact = compact_render_for_test(diagnostic, context);
    let subject = DiagnosticSubject::new(compact.view());
    let root = std::env::current_dir().unwrap_or_default();
    format_label_messages_from_subject(&subject, context, &root)
}

#[cfg(test)]
pub(crate) fn format_label_messages_with_context_from_root(
    diagnostic: &CompilerDiagnostic,
    context: DiagnosticRenderContext<'_>,
    root: &Path,
) -> Vec<String> {
    let compact = compact_render_for_test(diagnostic, context);
    let subject = DiagnosticSubject::new(compact.view());
    format_label_messages_from_subject(&subject, context, root)
}

fn format_label_messages_from_subject(
    subject: &DiagnosticSubject<'_>,
    context: DiagnosticRenderContext<'_>,
    root: &Path,
) -> Vec<String> {
    let primary_display_path = subject
        .primary_position(context)
        .map(|position| relative_display_path_from_root(position.path.as_path(), root));
    let labels = subject.resolved_labels(context, root);
    let mut rendered_labels = Vec::with_capacity(labels.len());

    for ResolvedDiagnosticLabel {
        style,
        path,
        line,
        column,
        message,
    } in labels
    {
        let style_name = match style {
            DiagnosticLabelStyle::Secondary => "info",
        };
        let rendered_label = match (path.as_deref(), line, column) {
            (None, None, None) => format!("{style_name}: - {message}"),
            (Some(label_path), Some(label_line), Some(label_col)) => {
                let include_path = primary_display_path
                    .as_deref()
                    .is_none_or(|primary_path| primary_path != label_path);
                let location = if include_path && !label_path.is_empty() {
                    format!("{label_path}:{label_line}:{label_col}")
                } else {
                    format!("{label_line}:{label_col}")
                };
                // A location-only label carries an empty message, so the separator is omitted
                // rather than trailing the location.
                if message.is_empty() {
                    format!("{style_name}: {location}")
                } else {
                    format!("{style_name}: {location} - {message}")
                }
            }
            _ => continue,
        };

        rendered_labels.push(rendered_label);
    }

    rendered_labels
}

pub(crate) fn format_label_messages_view(
    view: DiagnosticView<'_>,
    context: DiagnosticRenderContext<'_>,
) -> Vec<String> {
    let root = std::env::current_dir().unwrap_or_default();
    format_label_messages_from_subject(&DiagnosticSubject::new(view), context, &root)
}

pub(crate) fn format_guidance_view(
    view: DiagnosticView<'_>,
    context: DiagnosticRenderContext<'_>,
) -> Vec<String> {
    format_guidance_from_subject(&DiagnosticSubject::new(view), context)
}

#[cfg(test)]
pub(crate) fn format_diagnostic_guidance(
    diagnostic: &CompilerDiagnostic,
    context: DiagnosticRenderContext<'_>,
) -> Vec<String> {
    let compact = compact_render_for_test(diagnostic, context);
    let subject = DiagnosticSubject::new(compact.view());
    format_guidance_from_subject(&subject, context)
}

fn format_guidance_from_subject(
    subject: &DiagnosticSubject<'_>,
    context: DiagnosticRenderContext<'_>,
) -> Vec<String> {
    let rendered_payload = subject.payload(context);
    let mut lines = Vec::new();

    if !rendered_payload.message.is_empty() {
        lines.push(rendered_payload.message);
    }
    lines.extend(rendered_payload.guidance);

    lines
}

fn severity_display_name(severity: DiagnosticSeverity) -> &'static str {
    match severity {
        DiagnosticSeverity::Error => "Error",
        DiagnosticSeverity::Warning => "Warning",
        DiagnosticSeverity::Note => "Note",
    }
}

fn severity_visual(severity: DiagnosticSeverity) -> &'static str {
    match severity {
        DiagnosticSeverity::Error => "(╯°□°)╯ 🔥",
        DiagnosticSeverity::Warning => "⚠️",
        DiagnosticSeverity::Note => "📝",
    }
}
