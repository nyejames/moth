//! Terse rendering for frozen diagnostic views.
//!
//! WHAT: produces single-line machine-friendly diagnostic records.
//! WHY: CI, test runners, and IDEs often prefer compact output without ASCII art or colours.

use crate::compiler_frontend::compiler_errors::{BoundaryFailure, InfrastructureErrors};
#[cfg(test)]
use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
use crate::compiler_frontend::compiler_messages::DiagnosticSeverity;
#[cfg(test)]
use crate::compiler_frontend::compiler_messages::diagnostics::legacy_adapter::compact_render_for_test;
use crate::compiler_frontend::compiler_messages::diagnostics::view::DiagnosticView;
use crate::compiler_frontend::compiler_messages::render::infrastructure::{
    INFRASTRUCTURE_FAILURE_CODE, ISSUE_URL, InfrastructureClass, format_error_guidance_lines,
    infrastructure_class,
};
use crate::compiler_frontend::compiler_messages::render::subject::DiagnosticSubject;
use crate::compiler_frontend::compiler_messages::render::{
    DiagnosticRenderContext, display_column_number, display_line_number,
    relative_display_path_from_root, terse_diagnostic_message,
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

/// Renders every record of a boundary failure. The infrastructure record follows the error records and precedes the
/// warnings and notes, mirroring the severity-bucket order without inventing a user diagnostic.
pub(crate) fn format_terse_boundary_messages(failure: &BoundaryFailure) -> Vec<String> {
    let mut lines = Vec::new();
    let mut infrastructure = failure.infrastructure_errors();
    for severity in [
        DiagnosticSeverity::Error,
        DiagnosticSeverity::Warning,
        DiagnosticSeverity::Note,
    ] {
        if severity != DiagnosticSeverity::Error
            && let Some(errors) = infrastructure.take()
        {
            lines.push(format_terse_infrastructure_errors(errors));
        }
        for (report, view) in failure.reports.iter_severity(severity) {
            lines.push(format_terse_diagnostic_view(
                view,
                DiagnosticRenderContext::for_report(report),
            ));
        }
    }
    if let Some(errors) = infrastructure.take() {
        lines.push(format_terse_infrastructure_errors(errors));
    }
    lines
}

/// Render every cause in one five-field record, with no invented source location.
fn format_terse_infrastructure_errors(errors: &InfrastructureErrors) -> String {
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
        "E|{INFRASTRUCTURE_FAILURE_CODE}|{}||{}",
        sanitize_terse_field(&display_path),
        sanitize_terse_field(&message)
    )
}

/// Renders one diagnostic view as a single terse record.
pub(crate) fn format_terse_diagnostic_view(
    view: DiagnosticView<'_>,
    context: DiagnosticRenderContext<'_>,
) -> String {
    let subject = DiagnosticSubject::new(view);
    let descriptor = subject.descriptor();
    let severity_char = match subject.severity() {
        DiagnosticSeverity::Error => 'E',
        DiagnosticSeverity::Warning => 'W',
        DiagnosticSeverity::Note => 'N',
    };

    let (display_path, location) = match subject.primary_position(context) {
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

    let message = terse_diagnostic_message(&subject, context);

    format!(
        "{severity_char}|{}|{}|{}|{}",
        descriptor.code,
        sanitize_terse_field(&display_path),
        sanitize_terse_field(&location),
        sanitize_terse_field(&message)
    )
}

#[cfg(test)]
pub(crate) fn format_terse_diagnostic_with_context(
    diagnostic: &CompilerDiagnostic,
    context: DiagnosticRenderContext<'_>,
) -> String {
    let compact = compact_render_for_test(diagnostic, context);
    format_terse_diagnostic_view(compact.view(), context)
}

fn sanitize_terse_field(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace('|', "/")
}
