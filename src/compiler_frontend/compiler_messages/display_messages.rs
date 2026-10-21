//! User-facing message display helpers.
//!
//! WHAT: renders frozen boundary failures and legacy vessels into human-readable terminal output.
//! WHY: this is the final boundary between internal structured diagnostics and what the user sees.

use crate::compiler_frontend::compiler_errors::{
    BoundaryFailure, CompilerMessages, InfrastructureErrors,
};
use crate::compiler_frontend::compiler_messages::DiagnosticSeverity;
use crate::compiler_frontend::compiler_messages::diagnostics::legacy_adapter::into_boundary_failure;
use crate::compiler_frontend::compiler_messages::render::DiagnosticRenderContext;
use crate::compiler_frontend::compiler_messages::render::infrastructure::{
    ISSUE_URL, InfrastructureClass, format_error_guidance_lines, infrastructure_class,
    infrastructure_heading,
};
use crate::compiler_frontend::compiler_messages::render::relative_display_path_from_root;
use crate::compiler_frontend::compiler_messages::render::{terminal, terse};
use saying::say;
use std::fmt::Write;

/// Prints every record of a boundary failure. The infrastructure block follows the error records and precedes the
/// warnings and notes.
pub fn print_boundary_messages(failure: &BoundaryFailure) {
    let mut infrastructure = failure.infrastructure_errors();
    for severity in [
        DiagnosticSeverity::Error,
        DiagnosticSeverity::Warning,
        DiagnosticSeverity::Note,
    ] {
        if severity != DiagnosticSeverity::Error
            && let Some(errors) = infrastructure.take()
        {
            say!(Reset format_infrastructure_errors(errors));
        }
        for (report, view) in failure.reports.iter_severity(severity) {
            terminal::print_diagnostic_view(view, DiagnosticRenderContext::for_report(report));
        }
    }
    if let Some(errors) = infrastructure.take() {
        say!(Reset format_infrastructure_errors(errors));
    }
}

/// Prints the terse records of a boundary failure, one line per record.
pub fn print_terse_boundary_messages(failure: &BoundaryFailure) {
    for line in terse::format_terse_boundary_messages(failure) {
        println!("{line}");
    }
}

/// Prints a legacy vessel after converting it once to the boundary form.
pub fn print_compiler_messages(messages: CompilerMessages) {
    print_boundary_messages(&into_boundary_failure(messages));
}

/// Prints the terse records of a legacy vessel after converting it once to the boundary form.
pub fn print_terse_compiler_messages(messages: CompilerMessages) {
    print_terse_boundary_messages(&into_boundary_failure(messages));
}

/// Format the complete outer event without cloning errors or synthesising source frames.
pub(crate) fn format_infrastructure_errors(errors: &InfrastructureErrors) -> String {
    let mut rendered = String::new();
    for (index, error) in errors.iter().enumerate() {
        if index == 0 {
            let _ = writeln!(rendered, "\n{}", infrastructure_heading(&error.error_type));
        } else {
            let _ = writeln!(
                rendered,
                "\nAdditional cause {index}: {}",
                infrastructure_heading(&error.error_type)
            );
        }
        let class = infrastructure_class(&error.error_type);
        let _ = writeln!(rendered, "{}", class.introduction());
        if class == InfrastructureClass::CompilerBug {
            let _ = writeln!(rendered, "{ISSUE_URL}");
        }
        let _ = writeln!(rendered, "\nDetails:\n{}", error.msg);
        if let Some(host_path) = error.host_path.as_deref() {
            let display_path = relative_display_path_from_root(
                host_path,
                &std::env::current_dir().unwrap_or_default(),
            );
            let _ = writeln!(rendered, "  --> {display_path}");
        }
        for guidance in format_error_guidance_lines(error) {
            let _ = writeln!(rendered, "  {guidance}");
        }
    }
    rendered
}

#[cfg(test)]
#[path = "tests/display_messages_tests.rs"]
mod display_messages_tests;
