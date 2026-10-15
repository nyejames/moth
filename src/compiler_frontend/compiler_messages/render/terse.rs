//! Terse rendering for `CompilerDiagnostic`.
//!
//! WHAT: produces single-line machine-friendly diagnostic records.
//! WHY: CI, test runners, and IDEs often prefer compact output without ASCII art or colours.

use crate::compiler_frontend::compiler_errors::{CompilerMessages, InfrastructureErrors};
use crate::compiler_frontend::compiler_messages::DiagnosticKind;
use crate::compiler_frontend::compiler_messages::render::infrastructure::{
    ISSUE_URL, InfrastructureClass, format_error_guidance_lines, infrastructure_class,
};
use crate::compiler_frontend::compiler_messages::render::{
    DiagnosticRenderContext, display_column_number, display_line_number,
    relative_display_path_from_root, terse_payload_message,
};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticSeverity, InfrastructureDiagnosticKind,
};
use crate::compiler_frontend::utilities::basic::portable_path_text;
use std::fmt::Write;

#[cfg(test)]
pub(crate) fn format_terse_diagnostics_with_context(
    diagnostics: &[CompilerDiagnostic],
    context: DiagnosticRenderContext<'_>,
) -> Vec<String> {
    diagnostics
        .iter()
        .map(|d| format_terse_diagnostic_with_context(d, context))
        .collect()
}

pub(crate) fn format_terse_compiler_messages(messages: &CompilerMessages) -> Vec<String> {
    // The outer infrastructure failure renders as an error record beside the diagnostics,
    // after every error diagnostic and before warnings/notes — mirroring the severity-bucket
    // display order without fabricating a user diagnostic.
    let mut lines = Vec::new();
    let mut outer_emitted = messages.infrastructure_errors().is_none();
    for diagnostic_index in messages.diagnostic_display_order() {
        let diagnostic = &messages.diagnostic_slice()[diagnostic_index];
        if !outer_emitted && diagnostic.severity != DiagnosticSeverity::Error {
            if let Some(errors) = messages.infrastructure_errors() {
                lines.push(format_terse_infrastructure_errors(errors));
            }
            outer_emitted = true;
        }
        lines.push(format_terse_diagnostic_with_context(
            diagnostic,
            messages.diagnostic_render_context(diagnostic_index),
        ));
    }
    if !outer_emitted && let Some(errors) = messages.infrastructure_errors() {
        lines.push(format_terse_infrastructure_errors(errors));
    }
    lines
}

/// Render every cause in one five-field record, with no invented source location.
fn format_terse_infrastructure_errors(errors: &InfrastructureErrors) -> String {
    let code = DiagnosticKind::Infrastructure(InfrastructureDiagnosticKind::InfrastructureFailure)
        .descriptor()
        .code;
    let display_path = errors
        .primary
        .host_path
        .as_deref()
        .map(|path| {
            relative_display_path_from_root(path, &std::env::current_dir().unwrap_or_default())
        })
        .unwrap_or_default();

    let mut message = String::new();
    let mut has_compiler_bug = false;
    for (index, error) in errors.iter().enumerate() {
        if index == 0 {
            message.push_str("Primary cause: ");
        } else {
            let _ = write!(message, " Additional cause {index}: ");
        }
        let class = infrastructure_class(&error.error_type);
        let _ = write!(message, "{}", class.introduction());
        if class == InfrastructureClass::CompilerBug && !has_compiler_bug {
            let _ = write!(message, " {ISSUE_URL}");
            has_compiler_bug = true;
        }
        let _ = write!(message, " Details: {}", error.msg);
        if index != 0
            && let Some(path) = error.host_path.as_deref()
        {
            let _ = write!(message, " Path: {}", portable_path_text(path));
        }
        for guidance in format_error_guidance_lines(error) {
            let _ = write!(message, " {guidance}");
        }
    }
    format!(
        "E|{code}|{}||{}",
        sanitize_terse_field(&display_path),
        sanitize_terse_field(&message)
    )
}

pub(crate) fn format_terse_diagnostic_with_context(
    diagnostic: &CompilerDiagnostic,
    context: DiagnosticRenderContext<'_>,
) -> String {
    let descriptor = diagnostic.kind.descriptor();
    let severity_char = match diagnostic.severity {
        DiagnosticSeverity::Error => 'E',
        DiagnosticSeverity::Warning => 'W',
        DiagnosticSeverity::Note => 'N',
    };

    let (display_path, location) = match context.primary_position(diagnostic) {
        Some(position) => {
            let display_path = relative_display_path_from_root(
                position.path.as_path(),
                &std::env::current_dir().unwrap_or_default(),
            );
            let line = display_line_number(i32::try_from(position.start.line).unwrap_or(i32::MAX));
            let column =
                display_column_number(i32::try_from(position.start.column).unwrap_or(i32::MAX));
            (display_path, format!("{line}:{column}"))
        }
        None => (String::new(), String::new()),
    };

    let message = terse_payload_message(&diagnostic.payload, diagnostic.kind, context);

    format!(
        "{severity_char}|{}|{}|{}|{}",
        descriptor.code,
        sanitize_terse_field(&display_path),
        sanitize_terse_field(&location),
        sanitize_terse_field(&message)
    )
}

fn sanitize_terse_field(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace('|', "/")
}
