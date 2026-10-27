//! Terse rendering for frozen diagnostic views.
//!
//! WHAT: produces single-line machine-friendly diagnostic records.
//! WHY: CI, test runners, and IDEs often prefer compact output without ASCII art or colours.

use crate::compiler_frontend::compiler_errors::{
    CompilationOutcome, CompilationResult, InfrastructureClass, InfrastructureErrors,
};
use crate::compiler_frontend::compiler_messages::DiagnosticSeverity;
use crate::compiler_frontend::compiler_messages::diagnostics::report::DiagnosticReportSet;
use crate::compiler_frontend::compiler_messages::diagnostics::view::DiagnosticView;
use crate::compiler_frontend::compiler_messages::render::infrastructure::{
    INFRASTRUCTURE_FAILURE_CODE, ISSUE_URL, format_error_guidance_lines,
};
use crate::compiler_frontend::compiler_messages::render::subject::DiagnosticSubject;
use crate::compiler_frontend::compiler_messages::render::{
    DiagnosticRenderContext, display_column_number, display_line_number,
    relative_display_path_from_root, terse_diagnostic_message,
};
use crate::compiler_frontend::utilities::basic::portable_path_text;
use std::fmt::Write;

/// Render a compilation result while preserving the separation between diagnostic and infrastructure lanes.
pub(crate) fn format_terse_compilation_result<T>(result: &CompilationResult<T>) -> Vec<String> {
    match result {
        Ok(CompilationOutcome::Success {
            warnings: Some(warnings),
            ..
        }) => format_terse_diagnostic_reports(warnings),
        Ok(CompilationOutcome::Success { warnings: None, .. }) => Vec::new(),
        Ok(CompilationOutcome::Diagnosed(reports)) => format_terse_diagnostic_reports(reports),
        Err(errors) => vec![format_terse_infrastructure_errors(errors)],
    }
}

pub(crate) fn format_terse_diagnostic_reports(reports: &DiagnosticReportSet) -> Vec<String> {
    [
        DiagnosticSeverity::Error,
        DiagnosticSeverity::Warning,
        DiagnosticSeverity::Note,
    ]
    .into_iter()
    .flat_map(|severity| {
        reports.iter_severity(severity).map(|(report, view)| {
            format_terse_diagnostic_view(view, DiagnosticRenderContext::for_report(report))
        })
    })
    .collect()
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
        let class = error.kind.class();
        let _ = write!(message, "{}", class.introduction());
        if class == InfrastructureClass::CompilerBug && !has_compiler_bug {
            let _ = write!(message, " {ISSUE_URL}");
            has_compiler_bug = true;
        }
        let _ = write!(message, " Details: {}", error.message);
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

fn sanitize_terse_field(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace('|', "/")
}
