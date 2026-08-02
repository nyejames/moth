//! Frontend-only `check` command orchestration.
//!
//! WHAT: compiles input through Stage 0 + frontend validation (including borrow checking)
//! without running backend lowering or writing output artifacts.
//! WHY: users and tooling need a fast diagnostic pass that validates source correctness while
//! remaining backend-agnostic.

use crate::build_system::BuildProfile;
use crate::build_system::build::{
    BuildBootstrap, ProjectBuilder, bootstrap_project_build, collect_frontend_warnings,
};
use crate::build_system::create_project_modules::compile_project_frontend;
use crate::build_system::path_validation::check_if_valid_path;
use crate::compiler_frontend::compiler_errors::CompilerMessages;
use crate::compiler_frontend::display_messages::{
    print_compiler_messages, print_terse_compiler_messages,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::projects::command_status::{
    CommandStatus, benchmark_diagnostic_counts, emit_benchmark_status,
};
use crate::projects::html_project::html_project_builder::HtmlProjectBuilder;
use saying::say;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CheckOptions {
    pub terse: bool,
}

struct CheckOutcome {
    messages: CompilerMessages,
    duration: Duration,
}

pub(crate) fn run_check(path: &str, options: CheckOptions) -> CommandStatus {
    crate::timing::start_command_timing();
    let command_start = crate::timing::start_pipeline_timing();
    let outcome = execute_check(path);
    let error_count = outcome.messages.error_count();
    let warning_count = outcome.messages.warning_count();
    let benchmark_counts = benchmark_diagnostic_counts(&outcome.messages);

    let rendering_start = crate::timing::start_pipeline_timing();
    if options.terse {
        print_terse_compiler_messages(&outcome.messages);
        println!(
            "{}",
            format_terse_summary_line(outcome.duration, error_count, warning_count)
        );
    } else if error_count == 0 && warning_count == 0 {
        say!(Dark White "---------------------");
        say!(success_message(outcome.duration));
        say!(Bold Green "No errors or warnings");
    } else {
        print_compiler_messages(outcome.messages);
    }
    log_check_timing("command.check.message_rendering", rendering_start);
    log_check_timing("command.check.total", command_start);

    crate::timing::print_command_timing_summary();
    if let Some((error_count, warning_count)) = benchmark_counts {
        emit_benchmark_status(error_count, warning_count);
    }

    if error_count > 0 {
        CommandStatus::Failure
    } else {
        CommandStatus::Success
    }
}

fn execute_check(path: &str) -> CheckOutcome {
    let start = Instant::now();
    let normalized_path = normalize_entry_path(path);

    let mut path_string_table = StringTable::new();
    let path_validation_start = crate::timing::start_pipeline_timing();
    let valid_path = match check_if_valid_path(normalized_path, &mut path_string_table) {
        Ok(path) => {
            log_check_timing("command.check.path_validation", path_validation_start);
            path
        }
        Err(error) => {
            log_check_timing("command.check.path_validation", path_validation_start);
            return CheckOutcome {
                messages: CompilerMessages::from_error(error, path_string_table),
                duration: start.elapsed(),
            };
        }
    };

    let builder_construction_start = crate::timing::start_pipeline_timing();
    let project_builder = ProjectBuilder::new(Box::new(HtmlProjectBuilder::new()));
    log_check_timing(
        "command.check.builder_construction",
        builder_construction_start,
    );

    let bootstrap_start = crate::timing::start_pipeline_timing();
    let BuildBootstrap {
        mut config,
        style_directives,
        mut string_table,
        mut frontend_surface,
        validated_directory_output_settings,
    } = match bootstrap_project_build(&project_builder, valid_path) {
        Ok(bootstrap) => {
            log_check_timing("command.check.bootstrap", bootstrap_start);
            bootstrap
        }
        Err(messages) => {
            log_check_timing("command.check.bootstrap", bootstrap_start);
            return CheckOutcome {
                messages,
                duration: start.elapsed(),
            };
        }
    };

    let compile_frontend_start = crate::timing::start_pipeline_timing();
    let messages = match compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        validated_directory_output_settings.as_ref(),
        &style_directives,
        &mut frontend_surface,
        &mut string_table,
    ) {
        Ok(modules) => {
            log_check_timing(
                "command.check.compile_project_frontend",
                compile_frontend_start,
            );
            let warnings = collect_frontend_warnings(&modules);
            CompilerMessages::from_diagnostics(warnings, string_table)
        }
        Err(messages) => {
            log_check_timing(
                "command.check.compile_project_frontend",
                compile_frontend_start,
            );
            messages
        }
    };

    CheckOutcome {
        messages,
        duration: start.elapsed(),
    }
}

fn normalize_entry_path(path: &str) -> &str {
    if path.trim().is_empty() { "." } else { path }
}

fn format_terse_summary_line(
    duration: Duration,
    error_count: usize,
    warning_count: usize,
) -> String {
    if error_count == 0 && warning_count == 0 {
        return format!("{}. No errors or warnings.", success_message(duration));
    }

    format!("errors={error_count}, warnings={warning_count}.")
}

fn format_duration(duration: Duration) -> String {
    format!("{duration:?}")
}

fn success_message(duration: Duration) -> String {
    format!("Done in {}", format_duration(duration))
}

/// Record a check-command stage timing through the central `timers` substrate.
///
/// WHAT: delegates to `timing::record_started_pipeline_timing`, which stores the
///      observation in the active collection scope and emits the stable
///      `MOTH_BENCH timing` line when the output mode permits.
/// WHY:  check-command boundaries use dotted `command.check.*` metric names so the
///      concise summary and benchmark attribution share one recording path.
#[cfg(feature = "timers")]
fn log_check_timing(metric: &str, start: crate::timing::PipelineTimingStart) {
    crate::timing::record_started_pipeline_timing(metric, start);
}

/// No-op timing recorder when `timers` is off.
#[cfg(not(feature = "timers"))]
fn log_check_timing(_metric: &str, _start: crate::timing::PipelineTimingStart) {
    let _ = (_metric, _start);
}

#[cfg(test)]
#[path = "tests/check_tests.rs"]
mod tests;
