//! User-facing message display helpers.
//!
//! WHAT: renders frozen boundary failures into human-readable terminal output.
//! WHY: this is the final boundary between internal structured diagnostics and what the user sees.

use crate::compiler_frontend::compiler_errors::{
    CompilationOutcome, CompilationResult, InfrastructureClass, InfrastructureErrors,
};
use crate::compiler_frontend::compiler_messages::DiagnosticSeverity;
use crate::compiler_frontend::compiler_messages::diagnostics::report::DiagnosticReportSet;
use crate::compiler_frontend::compiler_messages::render::DiagnosticRenderContext;
use crate::compiler_frontend::compiler_messages::render::infrastructure::{
    ISSUE_URL, format_error_guidance_lines,
};
use crate::compiler_frontend::compiler_messages::render::relative_display_path_from_root;
use crate::compiler_frontend::compiler_messages::render::{terminal, terse};
use saying::say;
use std::fmt::Write;

/// Print one compilation result without combining diagnostics and infrastructure failures.
pub fn print_compilation_result<T>(result: &CompilationResult<T>) {
    match result {
        Ok(CompilationOutcome::Success {
            warnings: Some(warnings),
            ..
        }) => print_diagnostic_reports(warnings),
        Ok(CompilationOutcome::Success { warnings: None, .. }) => {}
        Ok(CompilationOutcome::Diagnosed(reports)) => print_diagnostic_reports(reports),
        Err(errors) => say!(Reset format_infrastructure_errors(errors)),
    }
}

pub fn print_diagnostic_reports(reports: &DiagnosticReportSet) {
    for severity in [
        DiagnosticSeverity::Error,
        DiagnosticSeverity::Warning,
        DiagnosticSeverity::Note,
    ] {
        for (report, view) in reports.iter_severity(severity) {
            terminal::print_diagnostic_view(view, DiagnosticRenderContext::for_report(report));
        }
    }
}

/// Print the terse records of one compilation result.
pub fn print_terse_compilation_result<T>(result: &CompilationResult<T>) {
    for line in terse::format_terse_compilation_result(result) {
        println!("{line}");
    }
}

/// Format the complete outer event without cloning errors or synthesising source frames.
pub(crate) fn format_infrastructure_errors(errors: &InfrastructureErrors) -> String {
    let mut rendered = String::new();
    for (index, error) in errors.iter().enumerate() {
        if index == 0 {
            let _ = writeln!(rendered, "\n{}", error.kind.heading());
        } else {
            let _ = writeln!(
                rendered,
                "\nAdditional cause {index}: {}",
                error.kind.heading()
            );
        }
        let class = error.kind.class();
        let _ = writeln!(rendered, "{}", class.introduction());
        if class == InfrastructureClass::CompilerBug {
            let _ = writeln!(rendered, "{ISSUE_URL}");
        }
        let _ = writeln!(rendered, "\nDetails:\n{}", error.message);
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

/// Print one aggregate infrastructure failure at the command boundary.
pub fn print_infrastructure_errors(errors: &InfrastructureErrors) {
    say!(Reset format_infrastructure_errors(errors));
}

#[cfg(test)]
#[path = "tests/display_messages_tests.rs"]
mod display_messages_tests;
