//! Terminal rendering for `CompilerDiagnostic`.
//!
//! WHAT: converts structured diagnostics into coloured terminal output.
//! WHY: this is the primary human-facing render path for compiler errors and warnings.

use crate::compiler_frontend::compiler_messages::render::{
    DiagnosticRenderContext, diagnostic_type_name, display_column_number, display_line_number,
    primary_caret_padding, primary_underline_length, relative_display_path_from_root,
    render_payload,
};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticLabelMessage, DiagnosticLabelStyle, DiagnosticPayload,
    DiagnosticSeverity,
};
use saying::say;

pub(crate) fn print_diagnostic_with_context(
    diagnostic: &CompilerDiagnostic,
    context: DiagnosticRenderContext<'_>,
) {
    let descriptor = diagnostic.kind.descriptor();
    let severity_name = severity_display_name(diagnostic.severity);
    let visual = severity_visual(diagnostic.severity);

    match diagnostic.severity {
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

    let primary_position = context.primary_position(diagnostic);
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

        let line = position.line;
        if !line.is_empty() {
            say!(Blue "    |");
            let line_label = display_line.to_string();
            let line_padding = " ".repeat(3usize.saturating_sub(line_label.len()));
            say!(Blue line_padding, Bold Blue line_label, " | ", Reset line);
            print!("{}", " ".repeat(display_line.to_string().len() + 4));

            let underline_start = primary_caret_padding(position, line);
            print!("{}", " ".repeat(underline_start));
            let underline_length = primary_underline_length(position, line);
            say!(Red "^".repeat(underline_length));
        }
    }

    for label_message in format_label_messages_with_context(diagnostic, context) {
        say!(Bright Blue "  ", label_message);
    }

    for guidance in format_payload_guidance(&diagnostic.payload, context) {
        say!(Bright Blue "  ", guidance);
    }
}

pub(crate) fn format_label_messages_with_context(
    diagnostic: &CompilerDiagnostic,
    context: DiagnosticRenderContext<'_>,
) -> Vec<String> {
    let mut rendered_labels = Vec::new();

    for label in &diagnostic.labels {
        let Some(message) = &label.message else {
            continue;
        };
        let style_name = match label.style {
            DiagnosticLabelStyle::Secondary => "info",
        };
        let Some(position) = context.label_position(label) else {
            if label.span.is_none() {
                let message_text = diagnostic_label_message_text(message, context);
                rendered_labels.push(format!("{style_name}: - {message_text}"));
            }
            continue;
        };
        let label_line =
            display_line_number(i32::try_from(position.start.line).unwrap_or(i32::MAX));
        let label_col =
            display_column_number(i32::try_from(position.start.column).unwrap_or(i32::MAX));
        let message_text = diagnostic_label_message_text(message, context);

        rendered_labels.push(format!(
            "{style_name}: {label_line}:{label_col} - {message_text}"
        ));
    }

    rendered_labels
}

pub(crate) fn format_payload_guidance(
    payload: &DiagnosticPayload,
    context: DiagnosticRenderContext<'_>,
) -> Vec<String> {
    let rendered_payload = render_payload(payload, context);
    let mut lines = Vec::new();

    if !rendered_payload.message.is_empty() {
        lines.push(rendered_payload.message);
    }
    lines.extend(rendered_payload.guidance);

    lines
}

fn diagnostic_label_message_text(
    message: &DiagnosticLabelMessage,
    context: DiagnosticRenderContext<'_>,
) -> String {
    let string_table = context.string_table;

    match message {
        DiagnosticLabelMessage::PreviousDeclaration => "previous declaration here".to_owned(),
        DiagnosticLabelMessage::ConflictingAccess => "earlier conflicting access here".to_owned(),
        DiagnosticLabelMessage::ExpectedTypeDeclaredHere => {
            "expected type declared here".to_owned()
        }
        DiagnosticLabelMessage::ValueMovedHere => "value moved here".to_owned(),
        DiagnosticLabelMessage::RenderedText(text) => string_table.resolve(*text).to_owned(),
        DiagnosticLabelMessage::GenericInstantiationCallSite => {
            "while instantiating this generic call".to_owned()
        }
        DiagnosticLabelMessage::GenericInstantiationBodySite => {
            "generic body operation failed here".to_owned()
        }
        DiagnosticLabelMessage::GenericInstantiationDeclarationSite => {
            "generic function declared here".to_owned()
        }
        DiagnosticLabelMessage::GenericInstantiationSubstitutions { substitutions } => {
            let substitution_text = substitutions
                .iter()
                .map(|substitution| {
                    let parameter_name = string_table.resolve(substitution.parameter_name);
                    let concrete_type =
                        diagnostic_type_name(substitution.concrete_type_id, context);
                    format!("{parameter_name} = {concrete_type}")
                })
                .collect::<Vec<_>>()
                .join(", ");

            format!("generic substitution: {substitution_text}")
        }
        DiagnosticLabelMessage::GenericInferencePreviousEvidence => {
            "previous generic inference evidence here".to_owned()
        }
        DiagnosticLabelMessage::ImmutableBindingDeclaration => {
            "immutable binding declared here".to_owned()
        }
    }
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
