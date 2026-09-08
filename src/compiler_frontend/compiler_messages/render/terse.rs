//! Terse rendering for `CompilerDiagnostic`.
//!
//! WHAT: produces single-line machine-friendly diagnostic records.
//! WHY: CI, test runners, and IDEs often prefer compact output without ASCII art or colours.

use crate::compiler_frontend::compiler_errors::CompilerMessages;
use crate::compiler_frontend::compiler_messages::render::{
    DiagnosticRenderContext, display_column_number, display_line_number,
    relative_display_path_from_root, resolve_source_file_path, terse_payload_message,
};
use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, DiagnosticSeverity};

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
    messages
        .diagnostic_display_order()
        .into_iter()
        .map(|diagnostic_index| {
            format_terse_diagnostic_with_context(
                &messages.diagnostic_slice()[diagnostic_index],
                messages.diagnostic_render_context(diagnostic_index),
            )
        })
        .collect()
}

pub(crate) fn format_terse_diagnostic_with_context(
    diagnostic: &CompilerDiagnostic,
    context: DiagnosticRenderContext<'_>,
) -> String {
    let string_table = context.string_table;
    let descriptor = diagnostic.kind.descriptor();
    let primary_position = context.primary_position(diagnostic);
    let severity_char = match diagnostic.severity {
        DiagnosticSeverity::Error => 'E',
        DiagnosticSeverity::Warning => 'W',
        DiagnosticSeverity::Note => 'N',
    };

    let display_path = relative_display_path_from_root(
        &resolve_source_file_path(&primary_position.scope, string_table),
        &std::env::current_dir().unwrap_or_default(),
    );
    let sanitized_path = sanitize_terse_field(&display_path);
    let line = display_line_number(i32::try_from(primary_position.start.line).unwrap_or(i32::MAX));
    let column =
        display_column_number(i32::try_from(primary_position.start.column).unwrap_or(i32::MAX));

    let message = terse_payload_message(&diagnostic.payload, diagnostic.kind, context);

    format!(
        "{severity_char}|{}|{sanitized_path}|{line}:{column}|{}",
        descriptor.code,
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
