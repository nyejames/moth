//! User-facing message display helpers.
//!
//! WHAT: renders compiled diagnostics and errors into human-readable terminal output.
//! WHY: this is the final boundary between internal structured diagnostics and what the user sees.

use crate::compiler_frontend::compiler_errors::{CompilerMessages, InfrastructureErrors};
use crate::compiler_frontend::compiler_messages::render::infrastructure::{
    ISSUE_URL, InfrastructureClass, format_error_guidance_lines, infrastructure_class,
    infrastructure_heading,
};
use crate::compiler_frontend::compiler_messages::render::relative_display_path_from_root;
use saying::say;
use std::fmt::Write;

pub fn print_compiler_messages(messages: CompilerMessages) {
    // The outer infrastructure failure prints as a standalone error beside the diagnostics,
    // after every error diagnostic and before warnings/notes — mirroring the severity-bucket
    // display order without fabricating a user diagnostic.
    let mut outer_emitted = messages.infrastructure_errors().is_none();
    for diagnostic_index in messages.diagnostic_display_order() {
        let diagnostic = &messages.diagnostic_slice()[diagnostic_index];
        if !outer_emitted
            && diagnostic.severity
                != crate::compiler_frontend::compiler_messages::DiagnosticSeverity::Error
        {
            if let Some(errors) = messages.infrastructure_errors() {
                say!(Reset format_infrastructure_errors(errors));
            }
            outer_emitted = true;
        }
        let render_context = messages.diagnostic_render_context(diagnostic_index);
        crate::compiler_frontend::compiler_messages::render::terminal::print_diagnostic_with_context(
            diagnostic,
            render_context,
        );
    }
    if !outer_emitted && let Some(errors) = messages.infrastructure_errors() {
        say!(Reset format_infrastructure_errors(errors));
    }
}

pub fn print_terse_compiler_messages(messages: &CompilerMessages) {
    for line in format_terse_compiler_messages(messages) {
        println!("{line}");
    }
}

pub fn format_terse_compiler_messages(messages: &CompilerMessages) -> Vec<String> {
    crate::compiler_frontend::compiler_messages::render::terse::format_terse_compiler_messages(
        messages,
    )
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
